use std::{fs::File, io::Write, path::PathBuf};

use bytes::Bytes;
use futures::stream::{self, FuturesUnordered, TryStreamExt};
use futures::StreamExt;
use tokio::{select, sync::mpsc};


use super::{http_requester::HttpRequester, task_manager::Priority};
use super::http_requester::HttpGetRequest;

pub struct DownloadRequest {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub priority: Priority
}

#[derive(Clone)]
pub struct DownloadManager {
    tx_high: mpsc::Sender<DownloadRequest>,
    tx_medium: mpsc::Sender<DownloadRequest>,
    tx_low: mpsc::Sender<DownloadRequest>,
}

async fn download(req: DownloadRequest, client: reqwest::Client, http_requester: HttpRequester) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Download task started");
    let resp = client.head(&req.url).send().await?;
    if !resp.status().is_success() {
        println!("Failed to get file size");
        return Ok(())
    }
    let content_length = resp.headers().get(reqwest::header::CONTENT_LENGTH).unwrap();
    let size = content_length.to_str()?.parse::<usize>()?;
    println!("File size: {}", size);
    let block_size = 8 * 1024 * 1024; // 8MiB

    if size <= block_size {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Range", format!("bytes={}-{}", "0", size).parse()?);
        let bytes = http_requester.dispatch(HttpGetRequest {
            url: req.url.clone(),
            headers,
            size
        }).await?;
        println!("File download complete");
        File::create(req.path.clone())?
            .write_all(&bytes)?;
        return Ok(())
    }
    
    let mut request_vec: Vec<HttpGetRequest> = vec![];
    for i in (0..size).step_by(block_size) {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Range", format!("bytes={}-{}", i, i + (size - i).min(block_size)).parse()?);
        request_vec.push(HttpGetRequest {
            url: req.url.clone(),
            headers,
            size: (size - i).min(block_size)
        });
    }
    let bytes_vec = request_vec.into_iter()
        .map(|req| {
            let http_requester = http_requester.clone();
            async move {
                http_requester.dispatch(req).await.unwrap()
            }
        })
        .collect::<FuturesUnordered<_>>()
        .collect::<Vec<_>>()
        .await;
    println!("Download complete");
    let mut file = File::create(req.path.clone())?;
    let mut iter = bytes_vec.iter();
    while let Some(bytes) = iter.next() {
        file.write_all(&bytes).unwrap();
    }
    Ok(())
}

impl DownloadManager {
    pub fn new(rt_handle: tokio::runtime::Handle, http_requester: HttpRequester, client: reqwest::Client) -> DownloadManager {
        let (tx_high, mut rx_high) = mpsc::channel::<DownloadRequest>(1);
        let (tx_medium, mut rx_medium) = mpsc::channel::<DownloadRequest>(1);
        let (tx_low, mut rx_low) = mpsc::channel::<DownloadRequest>(1);

        let rt_handle_ = rt_handle.clone();
        rt_handle.spawn(async move {
            loop {
                select! {
                    Some(req) = rx_high.recv() => {
                        println!("Receieved DownloadRequest Priority High");
                        let client = client.clone();
                        let http_requester = http_requester.clone();
                        let i: tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> = rt_handle_.spawn(async move {
                            download(req, client, http_requester).await
                        });
                    },
                    Some(req) = rx_medium.recv() => {
                        println!("Receieved DownloadRequest Priority Medium");
                        let client = client.clone();
                        let http_requester = http_requester.clone();
                        let i: tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> = rt_handle_.spawn(async move {
                            download(req, client, http_requester).await
                        });
                    },
                    Some(req) = rx_low.recv() => {
                        println!("Receieved DownloadRequest Priority Low");
                        let client = client.clone();
                        let http_requester = http_requester.clone();
                        let i: tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> = rt_handle_.spawn(async move {
                            download(req, client, http_requester).await
                        });
                    },
                    else => break,
                }
            }
        });
        DownloadManager {
            tx_high,
            tx_medium,
            tx_low,
        }
    }
    pub async fn dispatch(&self, request: DownloadRequest) {
        match request.priority {
            Priority::High => self.tx_high.send(request).await.unwrap(),
            Priority::Normal => self.tx_medium.send(request).await.unwrap(),
            Priority::Low => self.tx_low.send(request).await.unwrap(),
        }
    }
}