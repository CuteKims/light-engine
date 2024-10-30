use std::{fs::File, path::Path, sync::Arc, thread, time::Duration};

use engine::DownloadRequest;
use tokio::runtime;

mod engine;
mod manager;
mod task;

fn main() {
    let file = File::create(Path::new("C:\\Users\\20475\\Documents\\Playground\\file")).unwrap();
    let url = "https://dldir1.qq.com/qqfile/qq/QQNT/Windows/QQ_9.9.16_241023_x64_01.exe".to_string();

    let rt = Arc::new(runtime::Builder::new_current_thread().build().unwrap());

    let engine = engine::Builder::new().runtime(rt).build();
    let _task_id = engine.send_request(
        vec![
            DownloadRequest::new(file, url)
        ]
    );
    while true {
        thread::sleep(Duration::from_secs(1));
        println!("{:#?}", engine.poll_state_all());
    }
}