use std::{collections::HashMap, sync::{mpsc, Arc}, thread, time::Duration};

use tokio::{runtime::Runtime, sync::{oneshot, Semaphore}, time::interval};
use uuid::Uuid;

use crate::task::{DownloadTask, TaskState};

struct TokenBucket {
    sem: Arc<Semaphore>,
    jh: tokio::task::JoinHandle<()>,
}

impl TokenBucket {
    pub fn new(duration: Duration, capacity: usize, rt: Arc<Runtime>) -> Self {
        let sem = Arc::new(Semaphore::new(capacity));

        // refills the tokens at the end of each interval
        let jh = rt.spawn({
            let sem = sem.clone();
            let mut interval = interval(duration);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            async move {
                loop {
                    interval.tick().await;

                    if sem.available_permits() < capacity {
                        sem.add_permits(1);
                    }
                }
            }
        });

        Self { jh, sem }
    }

    pub async fn acquire(&self) {
        // This can return an error if the semaphore is closed, but we
        // never close it, so this error can never happen.
        let permit = self.sem.acquire().await.unwrap();
        // To avoid releasing the permit back to the semaphore, we use
        // the `SemaphorePermit::forget` method.
        permit.forget();
    }
}

impl Drop for TokenBucket {
    fn drop(&mut self) {
        // Kill the background task so it stops taking up resources when we
        // don't need it anymore.
        self.jh.abort();
    }
}

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
                        let polled_states = task_id.into_iter().map(|task_id| {
                            states.get(&task_id).unwrap().clone()
                        }).collect();
                        poll_result_tx.send(polled_states).unwrap();
                    },
                    TaskManagerCommand::Dispatch { task_id } => {
                        task_id.into_iter().for_each(|task_id| {
                            states.insert(
                                task_id.clone(),
                                TaskState::Pending
                            );
                        });
                    },
                    TaskManagerCommand::Report { task_id, latest_state } => {
                        if let Some(state) = states.get_mut(&task_id) {
                            *state = latest_state
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
        tasks.into_iter().for_each(|task| {
            let uuid = Uuid::new_v4();
            task_id.push(uuid);
            task.exec(uuid);
        });
        self.command_tx.send(TaskManagerCommand::Dispatch { task_id: task_id.clone() }).unwrap();
        task_id
    }

    pub fn poll_state(&self, task_id: Vec<Uuid>) -> Vec<TaskState> {
        let (poll_result_tx, poll_result_rx) = oneshot::channel::<Vec<TaskState>>();
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
        poll_result_tx: oneshot::Sender<Vec<TaskState>>
    },
    Dispatch {
        task_id: Vec<Uuid>
    },
    Report {
        task_id: Uuid,
        latest_state: TaskState
    },
    Close
}