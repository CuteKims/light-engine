use std::{fs::File, path::Path, sync::Arc, thread, time::Duration};

use engine::DownloadRequest;
use tokio::runtime;

mod engine;
mod manager;
mod task;

fn main() {
    let file1 = File::create(Path::new("C:\\Users\\20475\\Documents\\Playground\\file1.exe")).unwrap();
    let file2 = File::create(Path::new("C:\\Users\\20475\\Documents\\Playground\\file2.exe")).unwrap();
    let file3 = File::create(Path::new("C:\\Users\\20475\\Documents\\Playground\\file3.exe")).unwrap();
    let url = "https://dldir1.qq.com/qqfile/qq/QQNT/Windows/QQ_9.9.16_241023_x64_01.exe".to_string();

    let rt = Arc::new(runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap());

    let engine = engine::Builder::new().runtime(rt.clone()).build();
    let _task_id = engine.send_request(
        vec![
            DownloadRequest::new(file1, url.clone()),
            DownloadRequest::new(file2, url.clone()),
            DownloadRequest::new(file3, url.clone())
        ]
    );
    while true {
        thread::sleep(Duration::from_millis(1000));
        println!("{:#?}", engine.poll_state_all().iter().map(|(task_id, state)| {
            format!("{:?}: {}", task_id, match state {
                task::TaskState::Pending => "Pending".to_string(),
                task::TaskState::Downloading { total, streamed } => format!("{}% {}/{}", streamed.clone() as f32 / total.unwrap() as f32 * 100 as f32, streamed, total.unwrap()),
                task::TaskState::Finished => "Finished".to_string(),
                task::TaskState::Failed => "Failed".to_string(),
            })
        }).collect::<Vec<String>>());
    }
}