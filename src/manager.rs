use std::{collections::HashMap, sync::mpsc, thread};

use tokio::{task, sync::oneshot};
use uuid::Uuid;

use crate::{engine::TaskStatus, task::DownloadTask};

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
            let mut states: HashMap<Uuid, TaskStatus> = HashMap::new();
            for received in command_rx {
                match received {
                    TaskManagerAction::PollStateAll { poll_result_tx } => {
                        poll_result_tx.send(states.clone()).unwrap();
                    },
                    TaskManagerAction::PollState { task_id, poll_result_tx } => {
                        let polled_states = task_id.into_iter().map(|task_id| {
                            states.get(&task_id).unwrap().clone()
                        }).collect();
                        poll_result_tx.send(polled_states).unwrap();
                    },
                    TaskManagerAction::Dispatch { task_id } => {
                        task_id.into_iter().for_each(|task_id| {
                            states.insert(
                                task_id.clone(),
                                TaskStatus::Pending
                            );
                        });
                    },
                    TaskManagerAction::Report { task_id, latest_state } => {
                        if let Some(state) = states.get_mut(&task_id) {
                            *state = latest_state
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

    // DownloadTask在这里被赋予task_id并启动执行任务。
    // 有点傻逼对吧，我看这逻辑也不顺眼，但是我也不知道得怎么改。
    // 现在觉得task_id其实没必要，只要send_request()能够返回一个可以追踪任务的结构体就可以了。
    pub fn dispatch(&self, tasks: Vec<DownloadTask>) -> Vec<Uuid> {
        let mut task_ids: Vec<Uuid> = Vec::new();
        let mut jh: Vec<task::JoinHandle<()>> = Vec::new();
        tasks.into_iter().for_each(|task| {
            let uuid = Uuid::new_v4();
            task_ids.push(uuid);
            jh.push(task.exec(uuid));
        });
        self.command_tx.send(TaskManagerAction::Dispatch { task_id: task_ids.clone() }).unwrap();
        task_ids
    }

    pub fn poll_status(&self, task_id: Vec<Uuid>) -> Vec<TaskStatus> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<Vec<TaskStatus>>();
        self.command_tx.send(TaskManagerAction::PollState { task_id, poll_result_tx }).unwrap();
        let result = poll_result_rx.blocking_recv().unwrap();
        result
    }

    pub fn poll_status_all(&self) -> HashMap<Uuid, TaskStatus> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<HashMap<Uuid, TaskStatus>>();
        self.command_tx.send(TaskManagerAction::PollStateAll { poll_result_tx }).unwrap();
        let result = poll_result_rx.blocking_recv().unwrap();
        result
    }

    pub fn report_status(&self, task_id: Uuid, latest_state: TaskStatus) {
        self.command_tx.send(TaskManagerAction::Report { task_id, latest_state }).unwrap();
    }
}

enum TaskManagerAction {
    PollStateAll {
        poll_result_tx: oneshot::Sender<HashMap<Uuid, TaskStatus>>
    },
    PollState {
        task_id: Vec<Uuid>,
        poll_result_tx: oneshot::Sender<Vec<TaskStatus>>
    },
    Dispatch {
        task_id: Vec<Uuid>
    },
    Report {
        task_id: Uuid,
        latest_state: TaskStatus
    },
    Close
}