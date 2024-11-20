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
use thiserror::Error;
use tokio::{
    runtime::{self, Runtime},
    task::{self, Id, JoinHandle},
};

use crate::{
    limiter::{self, Limiter},
    manager::TaskManager,
    task::{DownloadError, DownloadTask, DownloadTaskResult},
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
        let rt = Arc::new(
            runtime::Builder::new_multi_thread()
                .worker_threads(self.worker_threads.unwrap_or(1))
                .thread_name("light-engine-worker-thread")
                .enable_all()
                .build()
                .unwrap(),
        );
        let task_manager = TaskManager::new();

        DownloadEngine {
            client,
            rt,
            task_manager,
            limiter,
        }
    }
}

#[derive(Clone)]
pub struct DownloadEngine {
    client: Client,
    rt: Arc<Runtime>,
    task_manager: TaskManager,
    limiter: Limiter,
}

impl DownloadEngine {
    // DownloadRequest在这里获取上下文并封装成为DownloadTask。
    pub fn send_request(&self, request: DownloadRequest) -> TaskHandle {
        let jh: JoinHandle<Result<(), DownloadError>> = request
            .clone()
            .into_task(
                self.task_manager.clone(),
                self.limiter.clone(),
                self.rt.clone(),
                self.client.clone(),
            )
            .exec();
        self.task_manager.new_task(jh.id());
        TaskHandle::new(self.task_manager.clone(), jh)
    }

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
                    self.rt.clone(),
                    self.client.clone(),
                )
                .exec();
            tasks.push(jh.id());
            jhs.push(jh);
        });
        self.task_manager.new_tasks(tasks.clone());
        BatchedTaskHandle::new(self.task_manager.clone(), jhs)
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
    pub fn status_handle(&self) -> BatchedTaskStatusHandle {
        BatchedTaskStatusHandle {
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
            return Poll::Ready(Err(TaskError::Aborted));
        }

        let mut polls: Vec<Poll<Result<DownloadTaskResult, task::JoinError>>> = Vec::new();

        for jh in self.jhs.iter_mut() {
            let poll = jh.poll_unpin(cx);
            if let Ready(Err(ref join_error)) = poll {
                if join_error.is_cancelled() {
                    return Poll::Ready(Err(TaskError::Aborted));
                } else if join_error.is_panic() {
                    panic!(
                        "light-engine worker thread panicked when executing download task id {}",
                        join_error.id()
                    )
                }
            } else if let Ready(Ok(Err(error))) = poll {
                return Poll::Ready(Err(TaskError::Failed { error }));
            };
            polls.push(poll);
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
    pub fn task_status_handle(&self) -> TaskStatusHandle {
        TaskStatusHandle {
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
                        Err(download_error) => { return Err(TaskError::Failed { error: download_error }) }
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

pub struct TaskStatusHandle {
    task_manager: TaskManager,
    task_id: Id,
}

impl TaskStatusHandle {
    pub fn poll(&self) -> Option<TaskStatus> {
        self.task_manager.get_status(self.task_id)
    }
}

pub struct BatchedTaskStatusHandle {
    task_manager: TaskManager,
    task_ids: Vec<Id>,
}

impl BatchedTaskStatusHandle {
    pub fn poll(&self) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(self.task_ids.clone())
    }
}

#[derive(Error, Debug)]
pub enum TaskError {
    #[error("aborted")]
    Aborted,
    #[error("")]
    Failed { error: DownloadError },
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub path: PathBuf,
    pub url: String,
    pub retries: NonZeroUsize,
    pub timeout: Duration,
}

impl DownloadRequest {
    pub fn new(path: PathBuf, url: String, retries: NonZeroUsize, timeout: Duration) -> Self {
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
        rt: Arc<Runtime>,
        client: Client,
    ) -> DownloadTask {
        return DownloadTask {
            request: self,
            task_manager,
            limiter,
            rt,
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
    Finished,
    Failed,
}
