use std::{fs::File, num::NonZeroU32, path::Path, sync::Arc, thread, time::Duration};

use engine::DownloadRequest;
use governor::Quota;
use tokio::runtime;

mod engine;
mod manager;
mod task;

fn main() {
    let file1 = File::create(Path::new("C:\\__Playground_created_by_CuteKims_for_testing\\file1.exe")).unwrap();
    let file2 = File::create(Path::new("C:\\__Playground_created_by_CuteKims_for_testing\\file2.exe")).unwrap();
    let file3 = File::create(Path::new("C:\\__Playground_created_by_CuteKims_for_testing\\file3.exe")).unwrap();
    let url = "https://dldir1.qq.com/qqfile/qq/QQNT/Windows/QQ_9.9.16_241023_x64_01.exe".to_string();

    let engine = engine::Builder::new().build();

    let _engine = engine.clone();

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(5000));

        engine.send_request(vec![DownloadRequest::new(file1, url.clone())]);
    
        thread::sleep(Duration::from_millis(5000));
    
        engine.send_request(vec![DownloadRequest::new(file2, url.clone())]);
    
        thread::sleep(Duration::from_millis(5000));
    
        engine.send_request(vec![DownloadRequest::new(file3, url.clone())]);
    });

    while true {
        thread::sleep(Duration::from_millis(1000));
        println!("{:#?}", _engine.poll_state_all().iter().map(|(task_id, state)| {
            format!("{:?}: {}", task_id, match state {
                task::TaskState::Pending => "Pending".to_string(),
                task::TaskState::Downloading { total, streamed } => format!("{}% {}/{}", streamed.clone() as f32 / total.unwrap() as f32 * 100 as f32, streamed, total.unwrap()),
                task::TaskState::Finishing => "Finishing".to_string(),
                task::TaskState::Finished => "Finished".to_string(),
                task::TaskState::Failed => "Failed".to_string(),
            })
        }).collect::<Vec<String>>());
    }
}