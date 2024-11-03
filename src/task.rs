use std::{io::Write, sync::Arc};

use reqwest::{header, Client};
use tokio::{runtime::Runtime, task};
use uuid::Uuid;

use crate::{engine::DownloadRequest, manager::TaskManager, watcher::Watcher};

// 包含了全部下载逻辑的下载任务结构体。拥有执行任务需要的所有上下文。
pub struct DownloadTask {
    pub request: DownloadRequest,
    pub task_manager: TaskManager,
    pub watcher: Watcher,
    pub rt: Arc<Runtime>,
    pub client: Client,
}

impl DownloadTask {
    // 目前这个方法返回一个JoinHandle，且目前这个返回值还没有利用上。
    pub fn exec(mut self, task_id: Uuid) -> task::JoinHandle<()>{
        let jh = self.rt.spawn(async move {
            let head_resp = self.client
                .head(self.request.url.clone())
                .send()
                .await
                .unwrap();
            if head_resp.status().is_success() == false {
                self.task_manager.report_status(task_id, TaskStatus::Failed);
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
            // 这里为什么要请求两次来获取content length？因为这个是用来以后实现分段异步下载用的。我还没开始写而已。
            // 你觉得这样已经够快了，没必要分段？我不要你觉得，我要我觉得。
            let mut full_resp = self.client.get(self.request.url).send().await.unwrap();
            self.task_manager.report_status(task_id, TaskStatus::Downloading { total: content_length, streamed: 0 });
            let mut streamed: usize = 0;
            while let Some(chunk) = full_resp.chunk().await.unwrap() {
                // 等待获取下载许可。用于限流。
                // 传入的chunk length会被同时用于流量统计。这是暂时的。
                self.watcher.acquire_permission(chunk.len()).await;
                self.request.file.write(&chunk).unwrap();
                streamed += chunk.len();
                self.task_manager.report_status(task_id, TaskStatus::Downloading { total: content_length, streamed });
            }
            self.task_manager.report_status(task_id, TaskStatus::Finishing);
            // 确保数据已经到达文件系统。
            // 其实我也不完全确定这个方法到底干了啥。
            self.request.file.sync_all().unwrap();
            self.task_manager.report_status(task_id, TaskStatus::Finished);
        });
        jh
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