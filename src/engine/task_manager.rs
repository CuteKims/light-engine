pub enum Priority {
    High,
    Normal,
    Low
}

pub struct DownloadTask {
    id: String,
    priority: Priority
}

pub struct TaskManager {
    tasks: Vec<DownloadTask>
}

impl TaskManager {
    fn new() -> TaskManager {
        return TaskManager { tasks: vec![] }
    }
}