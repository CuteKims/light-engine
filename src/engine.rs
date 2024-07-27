pub mod task_manager;
pub mod download_manager;
pub mod http_requester;

use tokio::{runtime, sync::mpsc};

use crate::limiter::DownloadLimiter;
use reqwest::header::{self, HeaderValue};

pub struct Builder {
    worker_threads: Option<usize>,
    max_speed: Option<usize>,
    max_concurrency: Option<usize>
}

impl Builder {
    pub fn new() -> Builder {
        Builder { worker_threads: None, max_speed: None, max_concurrency: None }
    }
    pub fn worker_threads(&self, val: usize) -> Builder {
        Builder { worker_threads: Some(val), max_speed: self.max_speed, max_concurrency: self.max_concurrency }
    }
    pub fn max_speed(&self, val: usize) -> Builder {
        Builder { worker_threads: self.worker_threads, max_speed: Some(val), max_concurrency: self.max_concurrency }
    }
    pub fn max_concurrency(&self, val: usize) -> Builder {
        Builder { worker_threads: self.worker_threads, max_speed: self.max_speed, max_concurrency: Some(val) }
    }
    pub fn build() -> Result<DownloadEngine, Box<dyn std::error::Error>> {
        Ok(DownloadEngine::new(DownloadLimiter::new(10, 1024)))
    }
}

pub struct DownloadEngine {
    limiter: DownloadLimiter
}

impl DownloadEngine {
    fn new(limiter: DownloadLimiter) -> DownloadEngine {
        return DownloadEngine {
            limiter
        }
    }
    pub async fn set_max_speed(&self, val: usize) {
        self.limiter.set_max_speed(val).await;
    }
    pub fn run() {

        
    }
}