use std::{future::Future, num::NonZeroUsize, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc}, task::Poll::{self, Ready}};

use futures::FutureExt;
use reqwest::Client;
use tokio::{runtime::{self, Runtime}, task::{self, AbortHandle, Id, JoinHandle}};
use thiserror::Error;

use crate::{manager::TaskManager, task::{DownloadError, DownloadTask, DownloadTaskResult}, watcher::{Quota, Watcher}};

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
        let watcher = Watcher::new(Quota::bytes_per_second(NonZeroUsize::new(2048 * 1024).unwrap()));
        DownloadEngine {
            client,
            rt,
            task_manager,
            watcher
        }
    }
}

#[derive(Clone)]
pub struct DownloadEngine {
    client: Client,
    rt: Arc<Runtime>,
    task_manager: TaskManager,
    watcher: Watcher,
}

impl DownloadEngine {
    // DownloadRequest在这里获取上下文并封装成为DownloadTask。
    pub fn send_request(&self, request: DownloadRequest) -> JobHandle {
        let jh: JoinHandle<Result<(), DownloadError>> = request
            .into_task(self.task_manager.clone(), self.watcher.clone(), self.rt.clone(), self.client.clone())
            .exec();
        self.task_manager.new_task(jh.id());
        JobHandle::new(jh)
    }

    pub fn send_batched_requests<I>(&self, requests: I) -> JobHandle
    where
        I: IntoIterator<Item = DownloadRequest>
    {
        let mut task_ids: Vec<Id> = Vec::new();
        let mut jhs: Vec<JoinHandle<DownloadTaskResult>> = Vec::new();
        requests.into_iter().for_each(|request| {
            let jh = request
                .into_task(self.task_manager.clone(), self.watcher.clone(), self.rt.clone(), self.client.clone())
                .exec();
            task_ids.push(jh.id());
            jhs.push(jh);
        });
        JobHandle::new_batched(jhs)
    }
}

pub struct JobHandle {
    jh: DownloadTaskJoinHandle,
    aborted: Arc<AtomicBool>
}

enum DownloadTaskJoinHandle {
    Signle(task::JoinHandle<DownloadTaskResult>),
    Batched(Vec<task::JoinHandle<DownloadTaskResult>>)
}

impl JobHandle {
    pub fn new(jh: task::JoinHandle<DownloadTaskResult>) -> Self {
        JobHandle { jh: DownloadTaskJoinHandle::Signle(jh), aborted: Arc::new(AtomicBool::new(false)) }
    }
    pub fn new_batched(jh: Vec<task::JoinHandle<DownloadTaskResult>>) -> Self {
        JobHandle { jh: DownloadTaskJoinHandle::Batched(jh), aborted: Arc::new(AtomicBool::new(false)) }
    }
    pub fn abort_handle(&self) -> JobAbortHandle {
        match &self.jh {
            DownloadTaskJoinHandle::Signle(jh) => {
                JobAbortHandle { aborted: self.aborted.clone(), handle: DownloadTaskAbortHandle::Signle(jh.abort_handle()) }
            },
            DownloadTaskJoinHandle::Batched(jhs) => {
                JobAbortHandle {
                    aborted: self.aborted.clone(),
                    handle: DownloadTaskAbortHandle::Batched(jhs.into_iter().map(|jh| {jh.abort_handle()}).collect::<Vec<AbortHandle>>())
                }
            },
        }
    }
    pub fn status_handle(&self) -> JobStatusHandle {
        JobStatusHandle {  }
    }
}

pub struct JobAbortHandle {
    aborted: Arc<AtomicBool>,
    handle: DownloadTaskAbortHandle
}

enum DownloadTaskAbortHandle {
    Signle(AbortHandle),
    Batched(Vec<AbortHandle>)
}

impl JobAbortHandle {
    pub fn abort(self) {
        self.aborted.store(true, Ordering::Relaxed);
        match self.handle {
            DownloadTaskAbortHandle::Signle(abort_handle) => { abort_handle.abort(); },
            DownloadTaskAbortHandle::Batched(abort_handles) => {
                abort_handles.into_iter().for_each(|abort_handle| {
                    abort_handle.abort();
                });
            },
        }
    }
}

pub struct JobStatusHandle {

}

impl JobStatusHandle {
    pub fn poll_status() -> JobStatus {
        JobStatus { task_statuses: todo!() }
    }
}

pub struct JobStatus {
    task_statuses: Vec<TaskStatus>
}

impl JobStatus {

}

#[derive(Error, Debug)]
pub enum JobError {
    #[error("aborted")]
    Aborted,
    #[error("")]
    Failed {
        error: DownloadError,
    },
}

impl Future for JobHandle {
    type Output = Result<(), JobError>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        
        if self.aborted.load(Ordering::Relaxed) { return Poll::Ready(Err(JobError::Aborted)) }

        match self.jh {
            DownloadTaskJoinHandle::Signle(ref mut jh) => {
                jh.poll_unpin(cx).map(|output| {
                    match output {
                        Ok(task_result) => {
                            match task_result {
                                Ok(_) => { return Ok(()) }
                                Err(download_error) => { return Err(JobError::Failed { error: download_error }) }
                            }
                        },
                        Err(join_error) => {
                            if join_error.is_cancelled() { return Err(JobError::Aborted) }
                            else if join_error.is_panic() { panic!("light-engine worker thread panicked when executing download task id {}", join_error.id()) }
                        },
                    }
                    Ok(())
                })
            },
            DownloadTaskJoinHandle::Batched(ref mut jhs) => {
                let mut polls: Vec<Poll<Result<DownloadTaskResult, task::JoinError>>> = Vec::new();

                for jh in jhs.iter_mut() {
                    let poll = jh.poll_unpin(cx);
                    if let Ready(Err(ref error)) = poll {
                        if error.is_cancelled() { return Poll::Ready(Err(JobError::Aborted)) }
                        else if error.is_panic() { panic!("light-engine worker thread panicked when executing download task id {}", error.id()) }
                    }
                    else if let Ready(Ok(Err(error))) = poll {
                        return Poll::Ready(Err(JobError::Failed { error }))
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
            },
        }
    }
}

#[derive(Debug)]
pub struct DownloadRequest {
    pub path: PathBuf,
    pub url: String,
    pub retries: usize,
}

impl DownloadRequest {
    pub fn new(path: PathBuf, url: String, retries: usize) -> Self {
        return DownloadRequest { path, url, retries }
    }

    pub fn into_task(self, task_manager: TaskManager, watcher: Watcher, rt: Arc<Runtime>, client: Client) -> DownloadTask {
        return DownloadTask {
            request: self,
            task_manager,
            watcher,
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