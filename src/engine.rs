use std::{
    future::Future,
    num::NonZeroUsize,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll::{self, Ready},
    time::Duration,
};

use futures::FutureExt;
use reqwest::Client;
use tokio::task::{self, Id, JoinHandle};

use crate::{
    limiter::{self, Limiter},
    manager::TaskManager,
    task::{DownloadTask, DownloadTaskResult, TaskError},
};

pub struct Builder {
    client: Option<Client>,
    limiter: Option<Limiter>,
    worker_threads: Option<usize>,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            client: None,
            limiter: None,
            worker_threads: None,
        }
    }

    pub fn client(self, client: Client) -> Self {
        Builder {
            client: Some(client),
            limiter: self.limiter,
            worker_threads: self.worker_threads,
        }
    }

    pub fn limiter(self, limiter: Limiter) -> Self {
        Builder {
            client: self.client,
            limiter: Some(limiter),
            worker_threads: self.worker_threads,
        }
    }

    pub fn worker_threads(self, val: NonZeroUsize) -> Self {
        Builder {
            client: self.client,
            limiter: self.limiter,
            worker_threads: Some(usize::from(val)),
        }
    }

    pub fn build(self) -> DownloadEngine {
        let client = self
            .client
            .unwrap_or_else(|| Client::builder().build().unwrap());
        let limiter = self
            .limiter
            .unwrap_or_else(|| limiter::Builder::new().build());
        let task_manager = TaskManager::new();

        DownloadEngine {
            client,
            task_manager,
            limiter,
        }
    }
}

pub struct DownloadEngine {
    client: Client,
    task_manager: TaskManager,
    limiter: Limiter,
}

impl DownloadEngine {
    // DownloadRequest在这里获取上下文并封装成为DownloadTask。
    pub fn send_request(&self, request: DownloadRequest) -> TaskHandle {
        let jh: JoinHandle<Result<(), ErrorKind>> = request
            .clone()
            .into_task(
                self.task_manager.clone(),
                self.limiter.clone(),
                self.client.clone(),
            )
            .exec();
        self.task_manager.new_task(jh.id());
        TaskHandle::new(self.task_manager.clone(), jh)
    }
    //FIXME: Fix this impl for task return struct changes

    pub fn send_batched_requests<I>(&self, requests: I) -> BatchedTaskHandle
    where
        I: IntoIterator<Item = DownloadRequest>,
    {
        let mut tasks: Vec<Id> = Vec::new();
        let mut jhs: Vec<JoinHandle<DownloadTaskResult>> = Vec::new();
        requests.into_iter().for_each(|request| {
            let jh = request
                .clone()
                .into_task(
                    self.task_manager.clone(),
                    self.limiter.clone(),
                    self.client.clone(),
                )
                .exec();
            tasks.push(jh.id());
            jhs.push(jh);
        });
        self.task_manager.new_tasks(tasks.clone());
        BatchedTaskHandle::new(self.task_manager.clone(), jhs)
    }
    pub fn pull_status(&self, task_id: Id) -> Option<TaskStatus> {
        self.task_manager.get_status(task_id)
    }
    pub fn pull_statuses(&self, task_ids: Vec<Id>) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(task_ids)
    }
}

pub struct BatchedTaskHandle {
    task_manager: TaskManager,
    jhs: Vec<task::JoinHandle<DownloadTaskResult>>,
    aborted: Arc<AtomicBool>,
}

impl BatchedTaskHandle {
    pub fn new(task_manager: TaskManager, jhs: Vec<task::JoinHandle<DownloadTaskResult>>) -> Self {
        BatchedTaskHandle {
            task_manager,
            jhs,
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn debatch_into(self) -> Vec<TaskHandle> {
        self.jhs
            .into_iter()
            .map(|jh| TaskHandle {
                task_manager: self.task_manager.clone(),
                jh,
                aborted: self.aborted.clone(),
            })
            .collect()
    }
    pub fn abort_handle(&self) -> BatchedAbortHandle {
        BatchedAbortHandle {
            task_manager: self.task_manager.clone(),
            aborted: self.aborted.clone(),
            handles: self.jhs.iter().map(|jh| jh.abort_handle()).collect(),
        }
    }
    pub fn status_handle(&self) -> BatchedStatusHandle {
        BatchedStatusHandle {
            task_manager: self.task_manager.clone(),
            task_ids: self.jhs.iter().map(|jh| jh.id()).collect(),
        }
    }
    pub fn ids(&self) -> Vec<Id> {
        self.jhs.iter().map(|jh| jh.id()).collect()
    }
    pub fn poll_statuses(&self) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(self.ids())
    }
    pub fn abort(self) {
        let mut task_ids: Vec<Id> = Vec::new();
        self.aborted.store(true, Ordering::Relaxed);
        self.jhs.into_iter().for_each(|handle| {
            task_ids.push(handle.id());
            handle.abort();
        });
        self.task_manager.delete_tasks(task_ids);
    }
}

impl Future for BatchedTaskHandle {
    type Output = Result<(), TaskError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.aborted.load(Ordering::Relaxed) {
            return Poll::Ready(Err(TaskError {}));
        }

        let mut polls: Vec<Poll<Result<DownloadTaskResult, task::JoinError>>> = Vec::new();

        for jh in self.jhs.iter_mut() {
            if !jh.is_finished() {
                let poll: Poll<Result<Result<(), ErrorKind>, task::JoinError>> = jh.poll_unpin(cx);
                if let Ready(Err(ref join_error)) = poll {
                    if join_error.is_cancelled() {
                        return Poll::Ready(Err(TaskError::Aborted));
                    } else if join_error.is_panic() {
                        panic!(
                            "light-engine worker thread panicked when executing download task id {}",
                            join_error.id()
                        )
                    }
                }
                if let Ready(Ok(Err(error))) = poll {
                    
                    println!("Failed");
                    return Poll::Ready(Err(TaskError::Failed(error)));
                }
                polls.push(poll);
            }
        }
        for poll in polls {
            match poll {
                Poll::Pending => return Poll::Pending,
                _ => continue,
            }
        }
        Poll::Ready(Ok(()))
    }
}
//FIXME: Fix this impl for task return struct changes

pub struct TaskHandle {
    task_manager: TaskManager,
    jh: task::JoinHandle<DownloadTaskResult>,
    aborted: Arc<AtomicBool>,
}

impl TaskHandle {
    pub fn new(task_manager: TaskManager, jh: task::JoinHandle<DownloadTaskResult>) -> Self {
        TaskHandle {
            task_manager,
            jh,
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn abort_handle(&self) -> AbortHandle {
        AbortHandle {
            task_manager: self.task_manager.clone(),
            aborted: self.aborted.clone(),
            handle: self.jh.abort_handle(),
        }
    }
    pub fn status_handle(&self) -> StatusHandle {
        StatusHandle {
            task_manager: self.task_manager.clone(),
            task_id: self.jh.id(),
        }
    }
    pub fn id(&self) -> Id {
        self.jh.id()
    }
    pub fn poll_status(&self) -> Option<TaskStatus> {
        self.task_manager.get_status(self.id())
    }
    pub fn abort(self) {
        self.aborted.store(true, Ordering::Relaxed);
        self.jh.abort();
        self.task_manager.delete_task(self.jh.id());
    }
}

impl Future for TaskHandle {
    type Output = Result<(), TaskError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.aborted.load(Ordering::Relaxed) {
            return Poll::Ready(Err(TaskError::Aborted));
        }
        self.jh.poll_unpin(cx).map(|output| {
            match output {
                Ok(task_result) => {
                    match task_result {
                        Ok(_) => { return Ok(()) }
                        Err(err) => { return Err(TaskError::Failed(err)) }
                    }
                },
                Err(join_error) => {
                    if join_error.is_cancelled() { return Err(TaskError::Aborted) }
                    else if join_error.is_panic() { panic!("light-engine worker thread panicked when executing download task id {}", join_error.id()) }
                },
            }
            Ok(())
        })
    }
}
//FIXME: Fix this impl for task return struct changes

pub struct AbortHandle {
    task_manager: TaskManager,
    aborted: Arc<AtomicBool>,
    handle: task::AbortHandle,
}

impl AbortHandle {
    pub fn abort(self) {
        self.aborted.store(true, Ordering::Relaxed);
        self.handle.abort();
        self.task_manager.delete_task(self.handle.id());
    }
}

pub struct BatchedAbortHandle {
    task_manager: TaskManager,
    aborted: Arc<AtomicBool>,
    handles: Vec<task::AbortHandle>,
}

impl BatchedAbortHandle {
    pub fn abort(self) {
        let mut task_ids: Vec<Id> = Vec::new();
        self.aborted.store(true, Ordering::Relaxed);
        self.handles.into_iter().for_each(|handle| {
            task_ids.push(handle.id());
            handle.abort();
        });
        self.task_manager.delete_tasks(task_ids);
    }
}

pub struct StatusHandle {
    task_manager: TaskManager,
    task_id: Id,
}

impl StatusHandle {
    pub fn poll(&self) -> Option<TaskStatus> {
        self.task_manager.get_status(self.task_id)
    }
}

pub struct BatchedStatusHandle {
    task_manager: TaskManager,
    task_ids: Vec<Id>,
}

impl BatchedStatusHandle {
    pub fn poll(&self) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(self.task_ids.clone())
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub path: PathBuf,
    pub url: String,
    pub retries: NonZeroUsize,
    pub timeout: Duration,
}

impl DownloadRequest {
    pub fn new(
        path: PathBuf,
        url: String,
        retries: NonZeroUsize,
        timeout: Duration
    ) -> Self {
        return DownloadRequest {
            path,
            url,
            retries,
            timeout,
        };
    }

    pub fn into_task(
        self,
        task_manager: TaskManager,
        limiter: Limiter,
        client: Client,
    ) -> DownloadTask {
        return DownloadTask {
            request: self,
            task_manager,
            limiter,
            client,
        };
    }
}

#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub status_type: TaskStatusType,
    pub retries: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatusType {
    Queuing,
    Pending,
    Downloading {
        total: Option<usize>,
        streamed: usize,
        rate: usize,
    },
    Finishing,
    Completed,
    Failed(TaskError)
}

impl TaskStatusType {
    pub fn is_finished(&self) -> bool {
        if let Self::Failed(_) = self {
            true
        }
        else if *self == Self::Completed {
            true
        }
        else {return false}
    }
}