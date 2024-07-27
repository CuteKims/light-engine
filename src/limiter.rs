use tokio::{sync::{Mutex, Semaphore}, time::Instant};
use std::{sync::{atomic, Arc}, time::Duration};

struct RateLimiter {
    rate: atomic::AtomicU64,
    capacity: atomic::AtomicU64,
    tokens: u64,
    last_update: Instant,
}

impl RateLimiter {
    fn new(rate: u64, capacity: u64) -> Self {
        RateLimiter {
            rate: atomic::AtomicU64::new(rate),
            capacity: atomic::AtomicU64::new(capacity),
            tokens: capacity,
            last_update: Instant::now(),
        }
    }

    async fn consume(&mut self, amount: u64) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();

        let rate = self.rate.load(atomic::Ordering::Relaxed);
        let capacity = self.capacity.load(atomic::Ordering::Relaxed);

        self.tokens = ((self.tokens as f64 + elapsed * rate as f64) as u64).min(capacity);
        self.last_update = now;

        if amount > self.tokens {
            let required_time = (amount - self.tokens) as f64 / rate as f64;
            tokio::time::sleep(Duration::from_secs_f64(required_time)).await;
            self.tokens = 0;
        } else {
            self.tokens -= amount;
        }
    }

    fn set_rate(&self, val: u64) {
        self.rate.store(val, atomic::Ordering::Relaxed);
    }

    fn set_capacity(&self, val: u64) {
        self.capacity.store(val, atomic::Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct SharedRateLimiter(Arc<Mutex<RateLimiter>>);

impl SharedRateLimiter {
    fn new(rate: u64, capacity: u64) -> Self {
        SharedRateLimiter(Arc::new(Mutex::new(RateLimiter::new(rate, capacity))))
    }

    async fn consume(&self, amount: u64) {
        let mut limiter = self.0.lock().await;
        limiter.consume(amount).await;
    }

    async fn set_rate(&self, val: u64) {
        let limiter = self.0.lock().await;
        limiter.set_rate(val);
    }

    async fn set_capacity(&self, val: u64) {
        let limiter = self.0.lock().await;
        limiter.set_capacity(val);
    }
}

#[derive(Clone)]
pub struct DownloadLimiter {
    rate_limiter: SharedRateLimiter,
}

pub type DownloadPermit = ();

impl DownloadLimiter {
    pub fn new(max_concurrency: usize, max_speed: usize) -> Self {
        DownloadLimiter {
            rate_limiter: SharedRateLimiter::new(max_speed as u64, max_speed as u64),
        }
    }
    pub async fn consume(&self, package_size: usize) -> DownloadPermit {
        let _ = self.rate_limiter.consume(package_size as u64).await;
        ()
    }
    pub async fn set_max_speed(&self, val: usize) {
        self.rate_limiter.set_capacity(val as u64).await;
        self.rate_limiter.set_rate(val as u64).await;
    }
}