use std::{collections::HashMap, fs::File, num::NonZeroUsize, sync::Arc};

use reqwest::Client;
use tokio::runtime::{self, Runtime};
use uuid::Uuid;

use crate::{manager::TaskManager, task::DownloadTask, watcher::{Quota, Watcher}};

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
    pub fn send_request(&self, requests: Vec<DownloadRequest>) -> Vec<Uuid> {
        self.task_manager.dispatch(requests.into_iter().map(|request| {
            request.into_task(self.task_manager.clone(), self.watcher.clone(), self.rt.clone(), self.client.clone())
        }).collect())
    }
    pub fn poll_status(&self, task_id: Vec<Uuid>) -> Vec<TaskStatus> {
        self.task_manager.poll_status(task_id)
    }
    pub fn poll_status_all(&self) -> HashMap<Uuid, TaskStatus> {
        self.task_manager.poll_status_all()
    }
}

pub struct DownloadRequest {
    pub file: File,
    pub url: String,
}

impl DownloadRequest {
    pub fn new(file: File, url: String) -> Self {
        return DownloadRequest { file, url }
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
pub enum TaskStatus {
    Pending,
    Downloading {
        total: Option<usize>,
        streamed: usize,
    },
    Finishing,
    Finished,
    Failed
}