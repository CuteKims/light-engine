use std::{num::NonZeroUsize, path::Path, time::Duration};
use light_engine::{engine::{self, DownloadRequest}, limiter};
use tokio::{runtime::Runtime, time::sleep};

fn main() {
    let path1 = Path::new("examples\\file.exe").to_path_buf();
    let url = "https://dldir1.qq.com/qqfile/qq/QQNT/Windows/QQ_9.9.16_241023_x64_01.exe".to_string();

    let limiter = limiter::Builder::new()
        .max_concurrency(NonZeroUsize::new(128).unwrap())
        .max_rate(NonZeroUsize::new(16 * 1024 * 1024).unwrap())
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

    let rt = Runtime::new().unwrap();

    rt.block_on(async move {
        let handle = engine.send_batched_requests(vec![request.clone(), request]);
        let status_handle = handle.status_handle();
        // tokio::pin!(handle);
        // loop {
        //     tokio::select! {
        //         _ = sleep(Duration::from_millis(1000)) => {
        //             println!("{:#?}", status_handle.poll());
        //         }
        //         result = &mut handle => {
        //             println!("{:#?}", result);
        //             break;
        //         }
        //     }
        // }
        let result = handle.await;
        println!("{:#?}", result);
    })
}