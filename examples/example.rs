use std::{num::NonZeroUsize, path::Path, time::Duration};
use light_engine::{engine::{self, DownloadRequest}, limiter};
use tokio::{runtime::Runtime, time::sleep};

fn main() {
    let path1 = Path::new("C:\\__Playground_created_by_CuteKims_for_testing\\file1.exe").to_path_buf();
    let url = "https://dldir1.qq.com/qqfile/qq/QQNT/Windows/QQ_9.9.16_241023_x64_01.exe".to_string();

    let limiter = limiter::Builder::new()
        .max_concurrency(NonZeroUsize::new(128).unwrap())
        .max_rate(NonZeroUsize::new(4 * 1024 * 1024).unwrap())
        .build();

    let engine = engine::Builder::new()
        .limiter(limiter)
        .build();

    let request = DownloadRequest::new(
        path1,
        url.clone(),
        NonZeroUsize::new(3).unwrap(),
        Duration::from_secs(10)
    );

    let handle = engine.send_request(request);

    let rt = Runtime::new().unwrap();

    rt.block_on(async move {
        loop {
            if let Some(task_status) = handle.poll_status() {
                sleep(Duration::from_secs(1)).await;
                println!("{:#?}", task_status);
            }
            else {
                break;
            }
        }
    })
}