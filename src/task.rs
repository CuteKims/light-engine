use std::time::Duration;

use reqwest::{header, Client};
use tokio::{fs::File, io::AsyncWriteExt, task, time::{sleep, timeout}};
use crate::{engine::{DownloadRequest, TaskError, TaskStatus, TaskStatusType}, limiter::Limiter, manager::TaskManager};

// 包含了全部下载逻辑的下载任务结构体。拥有执行任务需要的所有上下文。
pub struct DownloadTask {
    pub request: DownloadRequest,
    pub task_manager: TaskManager,
    pub limiter: Limiter,
    pub client: Client,
}

impl DownloadTask {
    pub fn exec(self) -> task::JoinHandle<Result<(), ErrorKind>>{
        tokio::spawn(exec(
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
) -> Result<(), ErrorKind> {
    let task_id = task::id();
    let mut result: Result<(), ErrorKind> = Ok(());
    for retries in 0..usize::from(request.retries) + 1 {
        result = async {
            task_manager.update_status(
                task_id,
                TaskStatus {
                    retries,
                    status_type: TaskStatusType::Queuing
                }
            );
            sleep(Duration::from_secs(2)).await;
            return Err(ErrorKind::Timeout);
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
                .map_err(|err| {ErrorKind::IO(err.kind())})?;

            let resp = timeout(request.timeout, async {
                client
                    .head(request.url.clone())
                    .send()
                    .await
            })
                .await
                .map_err(|_err| {ErrorKind::Timeout})?
                .map_err(|err| {ErrorKind::Network(get_network_error_kind(err))})?;

            if resp.status().is_success() == false {
                match resp.status().is_client_error() {
                    true => return Err(ErrorKind::Network(NetworkErrorKind::ClientStatus)),
                    false => return Err(ErrorKind::Network(NetworkErrorKind::ServerStatus)),
                }
            }

            let content_length = resp
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
            let mut resp = timeout(request.timeout, async {
                client
                    .get(request.url.clone())
                    .send()
                    .await
            })
                .await
                .map_err(|_err| {ErrorKind::Timeout})?
                .map_err(|err| {ErrorKind::Network(get_network_error_kind(err))})?;

            if resp.status().is_success() == false {
                match resp.status().is_client_error() {
                    true => return Err(ErrorKind::Network(NetworkErrorKind::ClientStatus)),
                    false => return Err(ErrorKind::Network(NetworkErrorKind::ServerStatus)),
                }
            }

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
                    resp
                        .chunk()
                        .await
                })
                    .await
                    .map_err(|_err| {ErrorKind::Timeout})?
                    .map_err(|err| {ErrorKind::Network(get_network_error_kind(err))})?
            } {
                // 等待获取下载许可。用于限流。
                limiter.acquire_stream(chunk.len()).await;
                file
                    .write(&chunk)
                    .await
                    .map_err(|err| {ErrorKind::IO(err.kind())})?;
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
                .map_err(|err| {ErrorKind::IO(err.kind())})?;

            task_manager.update_status(
                task_id,
                TaskStatus {
                    retries,
                    status_type: TaskStatusType::Completed
                }
            );
            Ok(())
        }.await;
        match result {
            Ok(_) => break,
            Err(_) => continue,
        }
    }
    if let Err(ref err) = result {
        println!("Error occured");
        task_manager.update_status(
            task_id,
            TaskStatus {
                retries: usize::from(request.retries),
                status_type: TaskStatusType::Failed(TaskError {
                    kind: err.clone(),
                    origional_request: request,
                })
            }
        );
    }
    result
}

fn get_network_error_kind(err: reqwest::Error) -> NetworkErrorKind {
    if err.is_body() {return NetworkErrorKind::Body}
    if err.is_builder() {return NetworkErrorKind::Builder}
    if err.is_connect() {return NetworkErrorKind::Connect}
    if err.is_decode() {return NetworkErrorKind::Decode}
    if err.is_redirect() {return NetworkErrorKind::Redirect}
    if err.is_request() {return NetworkErrorKind::Request}
    return  NetworkErrorKind::Unknown;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskError {
    Aborted,
    IO{
        error_kind: std::io::ErrorKind,
        origional_request: DownloadRequest
    },
    Network(NetworkErrorKind),
    TimedOut,
}
//FIXME: TaskError struct and return origional DownloadRequest when task failed
//TODO: Find a way to return origional DownloadRequest even when task is cancelled

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum NetworkErrorKind {
    Builder,
    Request,
    Body,
    Connect,
    Decode,
    Redirect,
    Upgrade,
    ServerStatus, 
    ClientStatus,
    Timeout,
    Unknown
}



pub type DownloadTaskResult = Result<(), ErrorKind>;