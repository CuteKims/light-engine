use std::{collections::HashMap, sync::mpsc, thread};

use tokio::{sync::oneshot, task::Id};

use crate::engine::{TaskStatus, TaskStatusType};

#[derive(Clone)]
pub struct TaskManager {
    command_tx: mpsc::Sender<TaskManagerAction>
}

impl TaskManager {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel::<TaskManagerAction>();
        // 使用单独的线程来管理任务状态，防止因Worker线程忙而导致API阻塞。
        // 你觉得没必要？我不要你觉得，我要我觉得。
        thread::Builder::new().name("light-engine-manager-thread".to_string()).spawn(move || {
            let mut statuses: HashMap<Id, TaskStatus> = HashMap::new();
            for received in command_rx {
                match received {
                    TaskManagerAction::PollStatus { task_id, poll_result_tx } => {
                        let status = statuses
                            .get(&task_id)
                            .map(|status| { status.clone() });
                        poll_result_tx
                            .send(status)
                            .unwrap_or(());
                    },
                    TaskManagerAction::PollStatuses { task_ids, poll_result_tx } => {
                        let polled_states = task_ids.into_iter().map(|task_id| {
                            statuses.get(&task_id).map(|status| { status.clone() })
                        }).collect();
                        poll_result_tx.send(polled_states).unwrap();
                    },
                    TaskManagerAction::NewTask { task_id } => {
                        statuses.insert(task_id, TaskStatus {retries: 0, status_type: TaskStatusType::Pending});
                    }
                    TaskManagerAction::NewTasks { task_ids } => {
                        task_ids.into_iter().for_each(|task_id| {
                            statuses.insert(
                                task_id.clone(),
                                TaskStatus {retries: 0, status_type: TaskStatusType::Pending}
                            );
                        });
                    },
                    TaskManagerAction::Report { task_id, status: new_status } => {
                        if let Some(_status) = statuses.get_mut(&task_id) {
                            *_status = new_status
                        }
                    },
                    TaskManagerAction::Close => {
                        break;
                    }
                }
            }
        }).unwrap();
        
        return TaskManager {
            command_tx,
        }
    }

    pub fn new_task(&self, task_id: Id) {
        self.command_tx.send(TaskManagerAction::NewTask { task_id: task_id.clone() }).unwrap();
    }
    pub fn new_tasks(&self, task_ids: Vec<Id>) {
        self.command_tx.send(TaskManagerAction::NewTasks { task_ids: task_ids.clone() }).unwrap();
    }

    pub fn poll_status(&self, task_id: Id) -> Option<TaskStatus> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<Option<TaskStatus>>();
        self.command_tx.send(TaskManagerAction::PollStatus { task_id, poll_result_tx }).unwrap();
        let result = poll_result_rx.blocking_recv().unwrap();
        result
    }

    pub fn poll_statuses(&self, task_ids: Vec<Id>) -> Vec<Option<TaskStatus>> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<Vec<Option<TaskStatus>>>();
        self.command_tx.send(TaskManagerAction::PollStatuses { task_ids, poll_result_tx }).unwrap();
        let result = poll_result_rx.blocking_recv().unwrap();
        result
    }

    pub fn report_status(&self, task_id: Id, status: TaskStatus) {
        self.command_tx.send(TaskManagerAction::Report { task_id, status }).unwrap();
    }
}

impl Drop for TaskManager {
    fn drop(&mut self) {
        self.command_tx.send(TaskManagerAction::Close).unwrap();
    }
}

enum TaskManagerAction {
    PollStatus {
        task_id: Id,
        poll_result_tx: oneshot::Sender<Option<TaskStatus>>
    },
    PollStatuses {
        task_ids: Vec<Id>,
        poll_result_tx: oneshot::Sender<Vec<Option<TaskStatus>>>
    },
    NewTask {
        task_id: Id
    },
    NewTasks {
        task_ids: Vec<Id>
    },
    Report {
        task_id: Id,
        status: TaskStatus
    },
    Close
}