use std::{num::NonZeroUsize, sync::{atomic::{AtomicUsize, Ordering}, Arc}, thread, time::Duration};

use tokio::sync::Semaphore;

pub struct Quota {
    replenish_per_millisecond: usize,
    max_burst: usize
}

impl Quota {
    pub fn bytes_per_second(max_burst: NonZeroUsize) -> Self {
        return Quota {
            replenish_per_millisecond: usize::from(max_burst) / 1000,
            max_burst: usize::from(max_burst)
        }
    }
}


// 用于限流和流量统计。
#[derive(Clone)]
pub struct Watcher {
    sem: Arc<Semaphore>,
    counter: Arc<AtomicUsize>,
}

impl Watcher {
    // 一个简单的令牌桶。
    pub fn new(quota: Quota) -> Self {
        let sem = Arc::new(Semaphore::new(0));
        let counter = Arc::new(AtomicUsize::new(0));
        // refills the tokens at the end of each interval
        let _sem = sem.clone();
        thread::Builder::new()
            .name("light-engine-watcher-thread".to_string())
            .spawn(move || {
                while true {
                    thread::sleep(Duration::from_millis(50));
                    if _sem.available_permits() < quota.max_burst {
                        _sem.add_permits(quota.replenish_per_millisecond * 50);
                    }
                }
            })
            .unwrap();
        Self { counter, sem }
    }

    // 获取下载许可。这个方法被用于限流。
    pub async fn acquire_permission(&self, n: usize) {
        // This can return an error if the semaphore is closed, but we
        // never close it, so this error can never happen.
        let permit = self.sem.acquire_many(n as u32).await.unwrap();
        // To avoid releasing the permit back to the semaphore, we use
        // the `SemaphorePermit::forget` method.
        permit.forget();
        // 流量在任务获取到下载许可后的同时被统计。
        // 这是暂时的。
        self.counter.fetch_add(n, Ordering::Relaxed);
    }

    pub fn check_semaphore(&self) -> usize {
        return self.sem.available_permits()
    }
    
    // 这个函数应该有更好的名字，但我还没想出来。
    pub fn poll_counter(&self) -> usize {
        let value = self.counter.load(Ordering::Relaxed);
        self.counter.store(0, Ordering::Relaxed);
        value
    }
}