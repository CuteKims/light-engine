use std::{collections::HashMap, fs::File, sync::Arc, thread};

use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use reqwest::Client;
use tokio::runtime::{self, Runtime};
use uuid::Uuid;

use crate::{manager::TaskManager, task::{DownloadTask, TaskState}};

pub struct Builder {
    client: Option<Client>,
    rt: Option<Arc<Runtime>>,
    quota: Option<Quota>
}

impl Builder {
    pub fn new() -> Self {
        return Builder {
            client: None,
            rt: None,
            quota: None,
        }
    }

    pub fn client(self, client: Client) -> Self {
        Builder {
            client: Some(client),
            rt: self.rt,
            quota: self.quota
        }
    }

    pub fn runtime(self, rt: Arc<Runtime>) -> Self {
        Builder {
            client: self.client,
            rt: Some(rt),
            quota: self.quota
        }
    }

    pub fn quota(self, quota: Quota) -> Self {
        Builder {
            client: self.client,
            rt: self.rt,
            quota: Some(quota)
        }
    }

    pub fn build(self) -> DownloadEngine {
        let client = self.client.unwrap_or_else(|| {
            Client::builder().build().unwrap()
        });
        let rt = self.rt.unwrap_or_else(|| {
            let rt = runtime::Builder::new_current_thread().thread_name("light-engine-worker-thread").enable_all().build().unwrap();
            Arc::new(rt)
        });
        let rate_limiter = match self.quota {
            Some(quota) => Some(Arc::new(RateLimiter::direct(quota))),
            None => None,
        };
        let task_manager = TaskManager::new();
        DownloadEngine {
            client,
            rt,
            rate_limiter,
            task_manager,
        }
    }
}

#[derive(Clone)]
pub struct DownloadEngine {
    client: Client,
    rt: Arc<Runtime>,
    rate_limiter: Option<Arc<DefaultDirectRateLimiter>>,
    task_manager: TaskManager,
}

impl DownloadEngine {
    pub fn send_request(&self, request: Vec<DownloadRequest>) -> Vec<Uuid> {
        self.task_manager.dispatch(request.into_iter().map(|request| {
            request.into_task(self.task_manager.clone(), self.rt.clone(), self.client.clone(), self.rate_limiter.clone())
        }).collect())
    }
    pub fn poll_state(&self, task_id: Vec<Uuid>) -> HashMap<Uuid, TaskState> {
        self.task_manager.poll_state(task_id)
    }
    pub fn poll_state_all(&self) -> HashMap<Uuid, TaskState> {
        self.task_manager.poll_state_all()
    }
    pub async fn send_request_async(&self, request: Vec<DownloadRequest>) {
        let task_id = self.task_manager.dispatch(request.into_iter().map(|request| {
            request.into_task(self.task_manager.clone(), self.rt.clone(), self.client.clone(), self.rate_limiter.clone())
        }).collect());

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

    pub fn into_task(self, task_manager: TaskManager, rt: Arc<Runtime>, client: Client, rate_limiter: Option<Arc<DefaultDirectRateLimiter>>) -> DownloadTask {
        return DownloadTask {
            request: self,
            task_manager,
            rt,
            client,
            rate_limiter,
        }
    }
}