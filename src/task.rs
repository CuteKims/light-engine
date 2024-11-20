use std::sync::Arc;
use reqwest::{header, Client};
use thiserror::Error;
use tokio::{fs::File, io::AsyncWriteExt, runtime::Runtime, task, time::{error::Elapsed, timeout}};
use crate::{engine::{DownloadRequest, TaskStatus, TaskStatusType}, manager::TaskManager, limiter::Limiter};

// 包含了全部下载逻辑的下载任务结构体。拥有执行任务需要的所有上下文。
pub struct DownloadTask {
    pub request: DownloadRequest,
    pub task_manager: TaskManager,
    pub limiter: Limiter,
    pub rt: Arc<Runtime>,
    pub client: Client,
}

impl DownloadTask {
    pub fn exec(self) -> task::JoinHandle<Result<(), DownloadError>>{
        self.rt.spawn(exec(
            self.request,
            self.client,
            self.limiter,
            self.task_manager
        ))
    }
}

async fn exec(
    request: DownloadRequest,
    client: Client,
    limiter: Limiter,
    task_manager: TaskManager
) -> Result<(), DownloadError> {
    let task_id = task::id();
    let mut result: Result<(), DownloadError> = Ok(());
    for retries in 0..usize::from(request.retries) + 1 {
        result = async {
            task_manager.update_status(
                task_id,
                TaskStatus {
                    retries,
                    status_type: TaskStatusType::Queuing
                }
            );
            let _permit = limiter.acquire_execute().await;
            task_manager.update_status(
                task_id,
                TaskStatus {
                    retries,
                    status_type: TaskStatusType::Pending
                }
            );
            let mut file = File::create(request.path.clone())
                .await
                .map_err(|err| {DownloadError::IO(err)})?;

            let head_resp = timeout(request.timeout, async {
                client
                    .head(request.url.clone())
                    .send()
                    .await
            })
                .await
                .map_err(|err| {DownloadError::Timeout(err)})?
                .map_err(|err| {DownloadError::Network(err)})?;

            if head_resp.status().is_success() == false {
                return Err(DownloadError::Server)
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

            // 这里为什么要多请求一次来获取content length？因为这个是用来以后实现分段异步下载用的。我还没开始写而已。
            // 你觉得这样已经够快了，没必要分段？我不要你觉得，我要我觉得。
            let mut full_resp = timeout(request.timeout, async {
                client
                    .get(request.url.clone())
                    .send()
                    .await
            })
                .await
                .map_err(|err| {DownloadError::Timeout(err)})?
                .map_err(|err| {DownloadError::Network(err)})?;

            task_manager.update_status(
                task_id,
                TaskStatus {
                    retries,
                    status_type: TaskStatusType::Downloading {
                        total: content_length,
                        streamed: 0,
                        rate: 0
                    }
                }
            );
            
            let mut streamed: usize = 0;
            while let Some(chunk) = {
                timeout(request.timeout, async {
                    full_resp
                        .chunk()
                        .await
                })
                    .await
                    .map_err(|err| {DownloadError::Timeout(err)})?
                    .map_err(|err| {DownloadError::Network(err)})?
            } {
                // 等待获取下载许可。用于限流。
                limiter.acquire_stream(chunk.len()).await;
                file
                    .write(&chunk)
                    .await
                    .map_err(|err| {DownloadError::IO(err)})?;
                streamed += chunk.len();
                task_manager.update_status(
                    task_id,
                    TaskStatus {
                        retries,
                        status_type: TaskStatusType::Downloading {
                            total: content_length,
                            streamed,
                            rate: 0
                        }
                    }
                );
            }
            task_manager.update_status(
                task_id, TaskStatus {
                    retries,
                    status_type: TaskStatusType::Finishing
                }
            );
            // “确保数据已经到达文件系统”说是。
            // 其实我也不完全确定这个方法到底干了啥。
            file
                .sync_all()
                .await
                .map_err(|err| {DownloadError::IO(err)})?;

            task_manager.update_status(
                task_id,
                TaskStatus {
                    retries,
                    status_type: TaskStatusType::Finished
                }
            );
            Ok(())
        }.await;
        match result {
            Ok(_) => break,
            Err(_) => continue,
        }
    }
    if let Err(ref _err) = result {
        task_manager.update_status(
            task_id,
            TaskStatus {
                retries: usize::from(request.retries),
                status_type: TaskStatusType::Failed
            }
        );
    }
    result
}

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("failed to create or access file")]
    IO(std::io::Error),
    #[error("network error")]
    Network(reqwest::Error),
    #[error("request timed out")]
    Timeout(Elapsed),
    #[error("server error")]
    Server,
    #[error("parsing error")]
    InvalidContent
}

pub type DownloadTaskResult = Result<(), DownloadError>;