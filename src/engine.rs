use std::{future::Future, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc}, task::Poll::{self, Ready}};

use futures::FutureExt;
use reqwest::Client;
use tokio::{runtime::{self, Runtime}, task::{self, Id, JoinHandle}};
use thiserror::Error;

use crate::{manager::TaskManager, task::{DownloadError, DownloadTask, DownloadTaskResult}, limiter::{self, Limiter}};

pub struct Builder {
    client: Option<Client>,
    worker_threads: Option<usize>
}

impl Builder {
    pub fn new() -> Self {
        return Builder {
            client: None,
            worker_threads: None,
        }
    }

    pub fn client(self, client: Client) -> Self {
        Builder {
            client: Some(client),
            worker_threads: self.worker_threads,
        }
    }

    pub fn worker_threads(self, val: usize) -> Self {
        Builder {
            client: self.client,
            worker_threads: Some(val),
        }
    }

    pub fn build(self) -> DownloadEngine {
        let client = self.client.unwrap_or_else(|| {
            Client::builder().build().unwrap()
        });
        let rt = Arc::new(runtime::Builder::new_multi_thread()
            .worker_threads(self.worker_threads.unwrap_or(1))
            .thread_name("light-engine-worker-thread")
            .enable_all()
            .build()
            .unwrap()
        );
        let task_manager = TaskManager::new();
        let limiter = limiter::Builder::new().build();
        DownloadEngine {
            client,
            rt,
            task_manager,
            limiter
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
    pub fn send_request(&self, request: DownloadRequest) -> (Id, TaskHandle) {
        let jh: JoinHandle<Result<(), DownloadError>> = request
            .clone()
            .into_task(self.task_manager.clone(), self.limiter.clone(), self.rt.clone(), self.client.clone())
            .exec();
        self.task_manager.new_task(jh.id());
        (jh.id(), TaskHandle::new(self.limiter.clone(), self.task_manager.clone(), jh))
    }

    pub fn send_batched_requests<I>(&self, requests: I) -> (Vec<Id>, BatchedTaskHandle)
    where
        I: IntoIterator<Item = DownloadRequest>
    {
        let mut tasks: Vec<Id> = Vec::new();
        let mut jhs: Vec<JoinHandle<DownloadTaskResult>> = Vec::new();
        requests.into_iter().for_each(|request| {
            let jh = request
                .clone()
                .into_task(self.task_manager.clone(), self.limiter.clone(), self.rt.clone(), self.client.clone())
                .exec();
            tasks.push(jh.id());
            jhs.push(jh);
        });
        self.task_manager.new_tasks(tasks.clone());
        (tasks, BatchedTaskHandle::new(self.limiter.clone(), self.task_manager.clone(), jhs))
    }
}

pub struct BatchedTaskHandle {
    limiter: Limiter,
    task_manager: TaskManager,
    jhs: Vec<task::JoinHandle<DownloadTaskResult>>,
    aborted: Arc<AtomicBool>
}

impl BatchedTaskHandle {
    pub fn new(limiter: Limiter, task_manager: TaskManager, jhs: Vec<task::JoinHandle<DownloadTaskResult>>) -> Self {
        BatchedTaskHandle { limiter, task_manager, jhs, aborted: Arc::new(AtomicBool::new(false)) }
    }
    pub fn debatch_into(self) -> Vec<TaskHandle> {
        self.jhs
            .into_iter()
            .map(|jh| {
                return TaskHandle {
                    limiter: self.limiter.clone(),
                    task_manager: self.task_manager.clone(),
                    jh,
                    aborted: self.aborted.clone()
                }
            })
            .collect()
    }
    pub fn abort_handle(&self) -> BatchedAbortHandle {
        return BatchedAbortHandle {
            task_manager: self.task_manager.clone(),
            aborted: self.aborted.clone(),
            handles: self.jhs.iter().map(|jh| {
                jh.abort_handle()
            }).collect()
        }
    }
    pub fn status_handle(&self) -> BatchedTaskStatusHandle {
        return BatchedTaskStatusHandle {
            limiter: self.limiter.clone(),
            task_manager: self.task_manager.clone(),
            task_ids: self.jhs.iter().map(|jh| {
                jh.id()
            }).collect(),
        }
    }
}

impl Future for BatchedTaskHandle {
    type Output = Result<(), TaskError>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        
        if self.aborted.load(Ordering::Relaxed) {
            
            return Poll::Ready(Err(TaskError::Aborted))
        }

        let mut polls: Vec<Poll<Result<DownloadTaskResult, task::JoinError>>> = Vec::new();

        for jh in self.jhs.iter_mut() {
            let poll = jh.poll_unpin(cx);
            if let Ready(Err(ref join_error)) = poll {
                if join_error.is_cancelled() { return Poll::Ready(Err(TaskError::Aborted)) }
                else if join_error.is_panic() { panic!("light-engine worker thread panicked when executing download task id {}", join_error.id()) }
            }
            else if let Ready(Ok(Err(error))) = poll {
                return Poll::Ready(Err(TaskError::Failed { error }))
            };
            polls.push(poll);
        }
        
        for poll in polls {
            match poll {
                Poll::Pending => return Poll::Pending,
                _ => continue
            }
        };
        Poll::Ready(Ok(()))
    }
}

pub struct TaskHandle {
    limiter: Limiter,
    task_manager: TaskManager,
    jh: task::JoinHandle<DownloadTaskResult>,
    aborted: Arc<AtomicBool>
}

impl TaskHandle {
    pub fn new(limiter: Limiter, task_manager: TaskManager, jh: task::JoinHandle<DownloadTaskResult>) -> Self {
        TaskHandle { limiter, task_manager, jh, aborted: Arc::new(AtomicBool::new(false)) }
    }
    
    pub fn abort_handle(&self) -> AbortHandle {
        AbortHandle {
            task_manager: self.task_manager.clone(),
            aborted: self.aborted.clone(),
            handle: self.jh.abort_handle()
        }
    }
    pub fn status_handle(&self) -> TaskStatusHandle {
        TaskStatusHandle {
            limiter: self.limiter.clone(),
            task_manager: self.task_manager.clone(),
            task_id: self.jh.id()
        }
    }
}

impl Future for TaskHandle {
    type Output = Result<(), TaskError>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        if self.aborted.load(Ordering::Relaxed) { return Poll::Ready(Err(TaskError::Aborted)) }
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
    handle: task::AbortHandle
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
    handles: Vec<task::AbortHandle>
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
    limiter: Limiter,
    task_manager: TaskManager,
    task_id: Id
}

impl TaskStatusHandle {
    pub fn poll(&self) -> Option<TaskStatus> {
        self.task_manager.get_status(self.task_id)
    }
}

pub struct BatchedTaskStatusHandle {
    limiter: Limiter,
    task_manager: TaskManager,
    task_ids: Vec<Id>
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
    Failed {
        error: DownloadError,
    },
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub path: PathBuf,
    pub url: String,
    pub retries: usize,
}

impl DownloadRequest {
    pub fn new(path: PathBuf, url: String, retries: usize) -> Self {
        return DownloadRequest { path, url, retries }
    }

    pub fn into_task(self, task_manager: TaskManager, limiter: Limiter, rt: Arc<Runtime>, client: Client) -> DownloadTask {
        return DownloadTask {
            request: self,
            task_manager,
            limiter,
            rt,
            client,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub status_type: TaskStatusType,
    pub retries: usize
}

#[derive(Debug, Clone)]
pub enum TaskStatusType {
    Pending,
    Downloading {
        total: Option<usize>,
        streamed: usize,
    },
    Finishing,
    Finished,
    Failed
}