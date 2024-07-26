use bytes::BufMut;
use reqwest::header;
use tokio::sync::mpsc;

use crate::limiter::DownloadLimiter;

struct HttpGetRequest {
    url: String,
    header: header::HeaderMap<header::HeaderValue>,
    bytes: bytes::BytesMut,
}

// type HttpGetResult = Result<(), Box<dyn std::error::Error>>;

pub struct HttpRequester {
    rt: tokio::runtime::Runtime,
    client: reqwest::Client,
    limiter: DownloadLimiter
}

impl HttpRequester {
    pub fn new(worker_threads: usize, limiter: DownloadLimiter) -> Result<HttpRequester, Box<dyn std::error::Error>> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .thread_name("http-requester")
            .build()?;
        let client = reqwest::Client::new();
        let requester = HttpRequester {
            rt,
            client,
            limiter
        };
        Ok(requester)
    }
    pub fn handle(&self) -> HttpRequesterHandle {
        HttpRequesterHandle {
            rt_handle: self.rt.handle().clone(),
            client: self.client.clone(),
            limiter: self.limiter.clone()
        }
    }
}

#[derive(Clone)]
pub struct HttpRequesterHandle {
    rt_handle: tokio::runtime::Handle,
    client: reqwest::Client,
    limiter: DownloadLimiter,
}

impl HttpRequesterHandle {
    async fn dispatch(&self, mut request: HttpGetRequest) -> () {
        let client = self.client.clone();
        let limiter = self.limiter.clone();
        let i: Result<(), tokio::task::JoinError> = self.rt_handle.spawn(async move {
            let _permit = limiter.acquire(request.bytes.capacity()).await;
            let resp = client
                .get(request.url)
                .headers(request.header)
                .send()
                .await.unwrap()
                .bytes()
                .await.unwrap();
            request.bytes.put(resp);
        }).await;
        i.unwrap()
    }
}