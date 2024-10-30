use std::{io::Write, sync::Arc};

use governor::DefaultDirectRateLimiter;
use reqwest::{header, Client};
use tokio::runtime::{self, Runtime};
use uuid::Uuid;

use crate::{engine::DownloadRequest, manager::TaskManager};

pub struct DownloadTask {
    pub request: DownloadRequest,
    pub task_manager: TaskManager,
    pub rt: Arc<Runtime>,
    pub client: Client,
    pub rate_limiter: Option<Arc<DefaultDirectRateLimiter>>,
}

impl DownloadTask {
    pub fn new(request: DownloadRequest, task_manager: TaskManager, rt: Arc<Runtime>, client: Client, rate_limiter: Option<Arc<DefaultDirectRateLimiter>>) -> Self {
        return DownloadTask {
            request,
            task_manager,
            rt,
            client,
            rate_limiter
        }
    }

    pub fn exec(mut self, task_id: Uuid) {
        self.rt.spawn(async move {
            println!("Task spawned");
            let head_resp = self.client
                .head(self.request.url.clone())
                .send()
                .await
                .unwrap();
            if head_resp.status().is_success() == false {
                self.task_manager.report_state(task_id, TaskState::Failed);
                return ()
            }
            let content_length = head_resp
                .headers()
                .get(header::CONTENT_LENGTH)
                .map(|header_value| {
                    header_value
                        .to_str()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap()
                });
            let mut full_resp = self.client.get(self.request.url).send().await.unwrap();
            self.task_manager.report_state(task_id, TaskState::Downloading { total: content_length, streamed: 0 });
            let mut streamed: usize = 0;
            while let Some(chunk) = full_resp.chunk().await.unwrap() {
                self.request.file.write(&chunk).unwrap();
                streamed += chunk.len();
                self.task_manager.report_state(task_id, TaskState::Downloading { total: content_length, streamed });
            }
        });
    }
}

#[derive(Debug, Clone)]
pub enum TaskState {
    Pending,
    Downloading {
        total: Option<usize>,
        streamed: usize,
    },
    Finished,
    Failed
}