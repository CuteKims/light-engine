use std::{future::Future, sync::{Arc, Mutex}, task::Poll};

use futures::FutureExt;
use tokio::task::{Id, JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{engine::TaskStatus, manager::TaskManager, task::{DownloadTaskResult, TaskError}};

pub struct TaskHandle {
    task_manager: TaskManager,
    jh: JoinHandle<DownloadTaskResult>,
    cancellation_token: CancellationToken
}

impl TaskHandle {
    pub fn new(task_manager: TaskManager, jh: JoinHandle<DownloadTaskResult>, cancellation_token: CancellationToken) -> Self {
        TaskHandle {
            task_manager,
            jh,
            cancellation_token
        }
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            cancellation_token: self.cancellation_token.clone()
        }
    }
    pub fn status_handle(&self) -> StatusHandle {
        StatusHandle {
            task_manager: self.task_manager.clone(),
            task_id: self.jh.id(),
        }
    }
    pub fn id(&self) -> Id {
        self.jh.id()
    }
    pub fn poll_status(&self) -> Option<TaskStatus> {
        self.task_manager.get_status(self.id())
    }
    pub fn cancel(&self) {
        self.cancellation_token.cancel()
    }
}

impl Future for TaskHandle {
    type Output = DownloadTaskResult;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        self.jh
            .poll_unpin(cx)
            .map(|output| {
                match output {
                    Ok(task_result) => {
                        match task_result {
                            Ok(_) => { return Ok(()) }
                            Err(err) => { return Err(err) }
                        }
                    },
                    Err(join_error) => {
                        if join_error.is_cancelled() { panic!("light-engine task id {} aborted unexpectedly", join_error.id()) }
                        else if join_error.is_panic() { panic!("light-engine task id {} panicked during execuation", join_error.id()) }
                    },
                }
                Ok(())
            }
        )
    }
}

pub struct StatusHandle {
    task_manager: TaskManager,
    task_id: Id,
}

impl StatusHandle {
    pub fn poll(&self) -> Option<TaskStatus> {
        self.task_manager.get_status(self.task_id)
    }
}


pub struct BatchedTaskHandle {
    task_manager: TaskManager,
    jhs: Vec<JoinHandle<DownloadTaskResult>>,
    cancellation_token: CancellationToken,
    errors: Arc<Mutex<Vec<TaskError>>>
}

impl BatchedTaskHandle {
    pub fn new(task_manager: TaskManager, jhs: Vec<JoinHandle<DownloadTaskResult>>, cancellation_token: CancellationToken) -> Self {
        BatchedTaskHandle {
            task_manager,
            jhs,
            cancellation_token,
            errors: Arc::new(Mutex::new(Vec::new()))
        }
    }
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            cancellation_token: self.cancellation_token.clone()
        }
    }
    pub fn status_handle(&self) -> BatchedStatusHandle {
        BatchedStatusHandle {
            task_manager: self.task_manager.clone(),
            task_ids: self.jhs.iter().map(|jh| jh.id()).collect(),
        }
    }
    pub fn ids(&self) -> Vec<Id> {
        self.jhs.iter().map(|jh| jh.id()).collect()
    }
    pub fn poll_statuses(&self) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(self.ids())
    }
    pub fn cancel(&self) {
        self.cancellation_token.cancel()
    }
}

impl Future for BatchedTaskHandle {
    type Output = Result<(), Vec<TaskError>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut errors = self.errors.clone();
        for jh in self.jhs.iter_mut() {
            if !jh.is_finished() {
                let poll: Poll<Result<DownloadTaskResult, JoinError>> = jh.poll_unpin(cx);
                if let Poll::Ready(Err(ref join_error)) = poll {
                    if join_error.is_cancelled() { panic!("light-engine task id {} aborted unexpectedly", join_error.id()) }
                    else if join_error.is_panic() { panic!("light-engine task id {} panicked during execuation", join_error.id()) }
                }
                match poll {
                    Poll::Ready(result) => {
                        if let Ok(Err(err)) = result {
                            Arc::get_mut(&mut errors).unwrap().get_mut().unwrap().push(err);
                            //what could happen? i dont care just yolo it
                            //FIXME: change this if anything goes wrong ¯\_(ツ)_/¯
                        }
                    },
                    Poll::Pending => {
                        return Poll::Pending
                    },
                }
            }
        }
        if Arc::get_mut(&mut errors).unwrap().get_mut().unwrap().len() == 0 {
            return Poll::Ready(Ok(()))
        }
        else {
            return Poll::Ready(Err(Arc::get_mut(&mut errors).unwrap().get_mut().unwrap().clone()))
        }
        //FIXME: i'm tired of messing with locks and shit for now someone fix this plz
    }
}

pub struct BatchedStatusHandle {
    task_manager: TaskManager,
    task_ids: Vec<Id>,
}

impl BatchedStatusHandle {
    pub fn poll(&self) -> Vec<Option<TaskStatus>> {
        self.task_manager.get_statuses(self.task_ids.clone())
    }
}


pub struct CancelHandle {
    cancellation_token: CancellationToken
}

impl CancelHandle {
    pub fn cancel(self) {
        self.cancellation_token.cancel()
    }
}