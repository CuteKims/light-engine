use std::sync::Arc;
use reqwest::{header, Client};
use thiserror::Error;
use tokio::{fs::File, io::AsyncWriteExt, runtime::Runtime, task};
use crate::{engine::{DownloadRequest, TaskStatus, TaskStatusType}, manager::TaskManager, watcher::Watcher};

// 包含了全部下载逻辑的下载任务结构体。拥有执行任务需要的所有上下文。
pub struct DownloadTask {
    pub request: DownloadRequest,
    pub task_manager: TaskManager,
    pub watcher: Watcher,
    pub rt: Arc<Runtime>,
    pub client: Client,
}

impl DownloadTask {
    pub fn exec(self) -> task::JoinHandle<Result<(), DownloadError>>{
        self.rt.spawn(exec(self.request, self.client, self.watcher, self.task_manager))
    }
}

async fn exec(request: DownloadRequest, client: Client, watcher: Watcher, task_manager: TaskManager) -> Result<(), DownloadError> {
    let task_id = task::id();
    let mut result: Result<(), DownloadError> = Ok(());
    for retries in 0..request.retries + 1 {
        result = {
            let mut file = File::create(request.path.clone())
                .await
                .map_err(|error| {
                    DownloadError::IO(error)
                })?;
            let head_resp = client
                .head(request.url.clone())
                .send()
                .await
                .map_err(|error| {
                    DownloadError::Network(error)
                })?;
            if head_resp.status().is_success() == false {
                return Err(DownloadError::Service)
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
            let mut full_resp = client.get(request.url.clone()).send().await.map_err(|error| {
                DownloadError::Network(error)
            })?;
            task_manager.report_status(task_id, TaskStatus { retries, status_type: TaskStatusType::Downloading { total: content_length, streamed: 0 }});
            let mut streamed: usize = 0;
            while let Ok(Some(chunk)) = full_resp.chunk().await.map_err(|error| { DownloadError::Network(error) }) {
                // 等待获取下载许可。用于限流。
                // 传入的chunk length会被同时用于流量统计。这是暂时的。
                watcher.acquire_permission(chunk.len()).await;
                file.write(&chunk).await.map_err(|error| {
                    DownloadError::IO(error)
                })?;
                streamed += chunk.len();
                task_manager.report_status(task_id, TaskStatus { retries, status_type: TaskStatusType::Downloading { total: content_length, streamed }});
            }
            task_manager.report_status(task_id, TaskStatus { retries, status_type: TaskStatusType::Finishing });
            // “确保数据已经到达文件系统”说是。
            // 其实我也不完全确定这个方法到底干了啥。
            file.sync_all().await.map_err(|error| {
                DownloadError::IO(error)
            })?;
            task_manager.report_status(task_id, TaskStatus { retries, status_type: TaskStatusType::Finished });
            Ok(())
        };
        match result {
            Ok(_) => break,
            Err(_) => continue,
        }
    }
    result
}

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("failed to create or access file")]
    IO(std::io::Error),
    #[error("network error")]
    Network(reqwest::Error),
    #[error("service error")]
    Service,
    #[error("parsing error")]
    InvalidContent
}

pub type DownloadTaskResult = Result<(), DownloadError>;