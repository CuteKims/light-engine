use std::{cmp::min, num::NonZeroUsize, sync::{atomic::{AtomicUsize, Ordering}, Arc}, thread, time::Duration};

use tokio::sync::{Semaphore, SemaphorePermit};

pub struct Builder {
    max_concurrency: Option<usize>,
    max_rate: Option<usize>,
}

impl Builder {
    pub fn new() -> Self {
        Self { max_concurrency: None, max_rate: None }
    }
    pub fn max_concurrency(self, val: NonZeroUsize) -> Self {
        Self { max_concurrency: Some(usize::from(val)), max_rate: self.max_rate }
    }
    pub fn max_rate(self, val: NonZeroUsize) -> Self {
        Self { max_concurrency: self.max_concurrency, max_rate: Some(usize::from(val)) }
    }
    pub fn build(self) -> Limiter {
        Limiter {
            concurrency_limiting_sem: Arc::new(Semaphore::new(self.max_concurrency.unwrap_or(8))),
            rate_limiting_sem: self.max_rate.map(|val| {
                let sem = Arc::new(Semaphore::new(0));
                let _sem = sem.clone();
                thread::Builder::new()
                    .name("light-engine-Limiter-thread".to_string())
                    .spawn(move || {
                        loop {
                            // refills the tokens at the end of each interval
                            thread::sleep(Duration::from_millis(1000 / 20));
                            if _sem.available_permits() < min(val, 32768) {
                                _sem.add_permits(val / 20);
                            }
                        }
                    })
                    .unwrap();
                sem
            })
        }
    }
}

// 用于限流。
#[derive(Clone)]
pub struct Limiter {
    concurrency_limiting_sem: Arc<Semaphore>,
    rate_limiting_sem: Option<Arc<Semaphore>>,
}

impl Limiter {
    // 获取下载许可。这个方法被用于限流。
    pub async fn acquire_stream(&self, n: usize) {
        if let Some(sem) = &self.rate_limiting_sem {
            let permit = sem.acquire_many(n as u32).await.unwrap();
            // This can return an error if the semaphore is closed, but we
            // never close it, so this error can never happen.
            permit.forget();
            // To avoid releasing the permit back to the semaphore, we use
            // the `SemaphorePermit::forget` method.
        }
    }

    pub async fn acquire_execute(&self) -> SemaphorePermit {
        self.concurrency_limiting_sem.acquire().await.unwrap()
    }
}