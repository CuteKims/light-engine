use std::{
    future::Future,
    num::NonZeroUsize,
    path::PathBuf,
    sync::atomic::Ordering,
    task::Poll::{self, Ready},
    time::Duration,
};

use reqwest::Client;
use tokio::task::{self, Id, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    limiter::{self, Limiter},
    manager::TaskManager,
    task::{DownloadTask, DownloadTaskResult, TaskError, TaskErrorKind}, task_handle::{BatchedTaskHandle, TaskHandle},
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
        let cancellation_token = CancellationToken::new();
        let jh: JoinHandle<DownloadTaskResult> = request
            .clone()
            .into_task(
                self.task_manager.clone(),
                self.limiter.clone(),
                self.client.clone(),
                cancellation_token.clone()
            )
            .exec();
        self.task_manager.new_task(jh.id());
        TaskHandle::new(self.task_manager.clone(), jh, cancellation_token)
    }
    //FIXME: Fix this impl for task return struct changes

    pub fn send_batched_requests<I>(&self, requests: I) -> BatchedTaskHandle
    where
        I: IntoIterator<Item = DownloadRequest>,
    {
        let cancellation_token = CancellationToken::new();
        let mut tasks: Vec<Id> = Vec::new();
        let mut jhs: Vec<JoinHandle<DownloadTaskResult>> = Vec::new();
        requests.into_iter().for_each(|request| {
            let jh = request
                .clone()
                .into_task(
                    self.task_manager.clone(),
                    self.limiter.clone(),
                    self.client.clone(),
                    cancellation_token.child_token()
                )
                .exec();
            tasks.push(jh.id());
            jhs.push(jh);
        });
        self.task_manager.new_tasks(tasks.clone());
        BatchedTaskHandle::new(self.task_manager.clone(), jhs, cancellation_token)
    }
    pub fn pull_status(&self, task_id: Id) -> Option<TaskStatus> {
        self.task_manager.get_status(task_id)
    }
    pub fn pull_statuses(&self, task_ids: Vec<Id>) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(task_ids)
    }
}




//FIXME: Fix this impl for task return struct changes


//FIXME: Fix this impl for task return struct changes




#[derive(Debug, Clone, PartialEq)]
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
        DownloadRequest {
            path,
            url,
            retries,
            timeout,
        }
    }

    pub fn into_task(
        self,
        task_manager: TaskManager,
        limiter: Limiter,
        client: Client,
        cancellation_token: CancellationToken
    ) -> DownloadTask {
        DownloadTask {
            request: self,
            task_manager,
            limiter,
            client,
            cancellation_token
        }
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