use std::{collections::HashMap, fs::File, sync::Arc, thread::{self, JoinHandle}};

use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use reqwest::Client;
use tokio::runtime::{self, Runtime};
use uuid::Uuid;

use crate::{manager::TaskManager, task::{DownloadTask, TaskState}};

pub struct Builder {
    client: Option<Client>,
    worker_threads: Option<usize>,
    quota: Option<Quota>
}

impl Builder {
    pub fn new() -> Self {
        return Builder {
            client: None,
            worker_threads: None,
            quota: None,
        }
    }

    pub fn client(self, client: Client) -> Self {
        Builder {
            client: Some(client),
            worker_threads: self.worker_threads,
            quota: self.quota
        }
    }

    pub fn worker_threads(self, val: usize) -> Self {
        Builder {
            client: self.client,
            worker_threads: Some(val),
            quota: self.quota
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
        DownloadEngine {
            client,
            rt,
            task_manager,
        }
    }
}

#[derive(Clone)]
pub struct DownloadEngine {
    client: Client,
    rt: Arc<Runtime>,
    task_manager: TaskManager,
}

impl DownloadEngine {
    pub fn send_request(&self, request: Vec<DownloadRequest>) -> Vec<Uuid> {
        self.task_manager.dispatch(request.into_iter().map(|request| {
            request.into_task(self.task_manager.clone(), self.rt.clone(), self.client.clone())
        }).collect())
    }
    pub fn poll_state(&self, task_id: Vec<Uuid>) -> Vec<TaskState> {
        self.task_manager.poll_state(task_id)
    }
    pub fn poll_state_all(&self) -> HashMap<Uuid, TaskState> {
        self.task_manager.poll_state_all()
    }
    // pub fn send_request_async(&self, request: Vec<DownloadRequest>) -> JoinHandle<()>{
    //     let task_id = self.task_manager.dispatch(request.into_iter().map(|request| {
    //         request.into_task(self.task_manager.clone(), self.rt.clone(), self.client.clone(), self.rate_limiter.clone())
    //     }).collect());
    // }
}

pub struct DownloadRequest {
    pub file: File,
    pub url: String,
}

impl DownloadRequest {
    pub fn new(file: File, url: String) -> Self {
        return DownloadRequest { file, url }
    }

    pub fn into_task(self, task_manager: TaskManager, rt: Arc<Runtime>, client: Client) -> DownloadTask {
        return DownloadTask {
            request: self,
            task_manager,
            rt,
            client,
        }
    }
}