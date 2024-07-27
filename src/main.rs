use std::{path::PathBuf, str::FromStr, time::Duration};

use engine::{download_manager::{self, DownloadRequest}, http_requester, task_manager::Priority};
use tokio::{runtime, time::sleep};

mod limiter;
mod engine;

fn main() {
    let rt = runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .worker_threads(1)
        .build()
        .unwrap();
    let client = reqwest::Client::new();
    let limiter = limiter::DownloadLimiter::new(2, 1 * 1024 * 1024);
    let http_requester = http_requester::HttpRequester::new(rt.handle().clone(), limiter, client.clone()).unwrap();
    let download_manager = download_manager::DownloadManager::new(rt.handle().clone(), http_requester, client.clone());
    let request = DownloadRequest {
        url: "https://dldir1.qq.com/qqfile/qq/QQNT/Windows/QQ_9.9.12_240708_x64_01.exe".to_string(),
        path: PathBuf::from_str(r"E:\QQ_9.9.12_240708_x64_01.exe").unwrap(),
        sha1: None,
        priority: Priority::High
    };
    rt.block_on(async move {
        download_manager.dispatch(request).await;
        sleep(Duration::from_secs(120)).await;
    });
}