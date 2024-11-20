use std::sync::Arc;

use dashmap::DashMap;
use tokio::task::Id;

use crate::engine::{TaskStatus, TaskStatusType};

#[derive(Clone)]
pub struct TaskManager {
    tasks: Arc<DashMap<Id, TaskStatus>>
}

impl TaskManager {
    pub fn new() -> Self {
        return Self { tasks: Arc::new(DashMap::new()) }
    }

    pub fn new_task(&self, task_id: Id) {
        self.tasks.insert(task_id, TaskStatus {retries: 0, status_type: TaskStatusType::Pending});
    }
    pub fn new_tasks(&self, task_ids: Vec<Id>) {
        task_ids.into_iter().for_each(|task_id| {
            self.tasks.insert(task_id, TaskStatus {retries: 0, status_type: TaskStatusType::Pending});
        });
    }

    pub fn get_status(&self, task_id: Id) -> Option<TaskStatus> {
        let status = self.tasks.get(&task_id).as_deref().cloned();
        if let Some(ref status) = status {
            if status.status_type == TaskStatusType::Finished || status.status_type == TaskStatusType::Failed {
                self.tasks.remove(&task_id);
            }
        };
        status
    }

    pub fn get_statuses(&self, task_ids: Vec<Id>) -> Vec<Option<TaskStatus>> {
        task_ids
            .into_iter()
            .map(|task_id| {
                self.tasks
                    .get(&task_id)
                    .as_deref()
                    .cloned()
            })
            .collect()
    }

    pub fn update_status(&self, task_id: Id, new_status: TaskStatus) {
        if let Some(mut status) = self.tasks.get_mut(&task_id) {
            *status = new_status
        }
    }

    pub fn delete_task(&self, task_id: Id) {
        self.tasks.remove(&task_id);
    }

    pub fn delete_tasks(&self, task_ids: Vec<Id>) {
        task_ids.into_iter().for_each(|task_id| {
            self.tasks.remove(&task_id);
        });
    }
}