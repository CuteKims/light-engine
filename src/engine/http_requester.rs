use bytes::{BufMut, Bytes, BytesMut};
use futures::FutureExt;
use reqwest::header;
use tokio::{sync::mpsc, time::Instant};

use crate::limiter::DownloadLimiter;

pub struct HttpGetRequest {
    pub url: String,
    pub headers: header::HeaderMap<header::HeaderValue>,
    pub size: usize,
}

// type HttpGetResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone)]
pub struct HttpRequester {
    rt_handle: tokio::runtime::Handle,
    client: reqwest::Client,
    limiter: DownloadLimiter
}

impl HttpRequester {
    pub fn new(rt_handle: tokio::runtime::Handle, limiter: DownloadLimiter, client: reqwest::Client) -> Result<HttpRequester, Box<dyn std::error::Error>> {
        let requester = HttpRequester {
            rt_handle,
            client,
            limiter
        };
        Ok(requester)
    }
    pub async fn dispatch(&self, mut request: HttpGetRequest) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
        println!("Receieved HttpGetRequest");
        let client = self.client.clone();
        let limiter = self.limiter.clone();
        let i: Result<Result<Bytes, Box<dyn std::error::Error + Send + Sync>>, tokio::task::JoinError> = self.rt_handle.spawn(async move {
            println!("Fetching block, range: {}", request.headers.clone().get("Range").unwrap().to_str().unwrap());
            let mut resp = client
                .get(request.url)
                .headers(request.headers)
                .send()
                .await?;
            let mut bytes = BytesMut::new();
            while let Some(chunk) = resp.chunk().await? {
                let _permit = limiter.consume(chunk.len()).then(|()| async {
                    bytes.put(chunk);
                }).await;
            }
            Ok(bytes.freeze())
        }).await;
        i.unwrap()
    }
}