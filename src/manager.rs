use std::{collections::HashMap, sync::mpsc, thread};

use tokio::{runtime, sync::oneshot};
use uuid::Uuid;

use crate::task::{DownloadTask, TaskState};

#[derive(Clone)]
pub struct TaskManager {
    command_tx: mpsc::Sender<TaskManagerCommand>
}

impl TaskManager {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel::<TaskManagerCommand>();
        thread::Builder::new().name("light-engine-manager-thread".to_string()).spawn(move || {
            let mut states: HashMap<Uuid, TaskState> = HashMap::new();
            for received in command_rx {
                match received {
                    TaskManagerCommand::PollStateAll { poll_result_tx } => {
                        poll_result_tx.send(states.clone()).unwrap();
                    },
                    TaskManagerCommand::PollState { task_id, poll_result_tx } => {
                        let mut polled_states: HashMap<Uuid, TaskState> = HashMap::new();
                        task_id.into_iter().for_each(|task_id| {
                            polled_states.insert(task_id,states.get(&task_id).unwrap().clone());
                        });
                        poll_result_tx.send(polled_states).unwrap();
                    },
                    TaskManagerCommand::Dispatch { tasks } => {
                        tasks.into_iter().for_each(|(id, task)| {
                            task.exec(id.clone());
                            states.insert(
                                id.clone(),
                                TaskState::Pending
                            );
                        });
                    },
                    TaskManagerCommand::Report { task_id, latest_state } => {
                        if let Some(_progress) = states.get_mut(&task_id) {
                            *_progress = latest_state
                        }
                    },
                    TaskManagerCommand::Close => {
                        break;
                    }
                }
            }
        }).unwrap();
        
        return TaskManager {
            command_tx,
        }
    }

    pub fn dispatch(&self, tasks: Vec<DownloadTask>) -> Vec<Uuid> {
        let mut task_id: Vec<Uuid> = Vec::new();
        let tasks = tasks
            .into_iter()
            .map(|task| {
                let id = Uuid::new_v4();
                task_id.push(id);
                (id, task)
            })
            .collect::<HashMap<Uuid, DownloadTask>>();
        self.command_tx.send(TaskManagerCommand::Dispatch { tasks }).unwrap();
        task_id
    }

    pub fn poll_state(&self, task_id: Vec<Uuid>) -> HashMap<Uuid, TaskState> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<HashMap<Uuid, TaskState>>();
        self.command_tx.send(TaskManagerCommand::PollState { task_id, poll_result_tx }).unwrap();
        let result = poll_result_rx.blocking_recv().unwrap();
        result
    }

    pub fn poll_state_all(&self) -> HashMap<Uuid, TaskState> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<HashMap<Uuid, TaskState>>();
        self.command_tx.send(TaskManagerCommand::PollStateAll { poll_result_tx }).unwrap();
        let result = poll_result_rx.blocking_recv().unwrap();
        result
    }

    pub fn report_state(&self, task_id: Uuid, latest_state: TaskState) {
        self.command_tx.send(TaskManagerCommand::Report { task_id, latest_state }).unwrap();
    }
}

enum TaskManagerCommand {
    PollStateAll {
        poll_result_tx: oneshot::Sender<HashMap<Uuid, TaskState>>
    },
    PollState {
        task_id: Vec<Uuid>,
        poll_result_tx: oneshot::Sender<HashMap<Uuid, TaskState>>
    },
    Dispatch {
        tasks: HashMap<Uuid, DownloadTask>
    },
    Report {
        task_id: Uuid,
        latest_state: TaskState
    },
    Close
}