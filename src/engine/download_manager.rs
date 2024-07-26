use std::path::PathBuf;

use tokio::{select, sync::mpsc};

use super::{http_requester::{self, HttpRequester, HttpRequesterHandle}, task_manager::Priority};

pub struct DownloadRequest {
    url: String,
    path: PathBuf,
    sha1: Option<String>,
    priority: Priority
}

struct DownloadManager {
    rt: tokio::runtime::Runtime,

    tx_high: mpsc::Sender<DownloadRequest>,
    tx_medium: mpsc::Sender<DownloadRequest>,
    tx_low: mpsc::Sender<DownloadRequest>,

    http_requester_handle: HttpRequesterHandle
}

impl DownloadManager {
    fn new(http_requester_handle: HttpRequesterHandle) -> DownloadManager {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let (tx_high, mut rx_high) = mpsc::channel(1);
        let (tx_medium, mut rx_medium) = mpsc::channel(1);
        let (tx_low, mut rx_low) = mpsc::channel(1);
        rt.spawn(async move {
            loop {
                select! {
                    Some(req) = rx_high.recv() => {

                    },
                    Some(req) = rx_medium.recv() => {

                    },
                    Some(req) = rx_low.recv() => {

                    },
                    else => break,
                }
            }
        });
        DownloadManager {
            rt,
            tx_high,
            tx_medium,
            tx_low,
            http_requester_handle
        }
    }
    fn handle(&self) -> DownloadManagerHandle {
        DownloadManagerHandle {
            rt_handle: self.rt.handle().clone(),
            tx_high: self.tx_high.clone(),
            tx_medium: self.tx_medium.clone(),
            tx_low: self.tx_low.clone()
        }
    }
}

struct DownloadManagerHandle {
    rt_handle: tokio::runtime::Handle,
    tx_high: mpsc::Sender<DownloadRequest>,
    tx_medium: mpsc::Sender<DownloadRequest>,
    tx_low: mpsc::Sender<DownloadRequest>,
}

impl DownloadManagerHandle {
    async fn dispatch(&self, request: DownloadRequest) {
        match request.priority {
            Priority::High => self.tx_high.send(request).await.unwrap(),
            Priority::Normal => self.tx_medium.send(request).await.unwrap(),
            Priority::Low => self.tx_low.send(request).await.unwrap(),
        }
    }
}