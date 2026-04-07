#![allow(dead_code)]

use crate::task_registry::{Task, TaskStatus};
use crate::task_registry_backend::{TaskRegistryBackend, TaskRegistryError};
use crate::TaskPacket;

/// Dolt-backed task registry.
///
/// Persists tasks, messages, and output in a Dolt database using the schema
/// defined in `TASK_REGISTRY_SCHEMA_DESIGN.md`. Replaces the in-memory
/// [`TaskRegistry`](crate::task_registry::TaskRegistry) for environments where
/// durable, queryable task storage is preferred.
#[derive(Debug, Clone)]
pub struct DoltTaskRegistryBackend {
    /// Connection string or path to the Dolt database.
    pub connection: String,
}

impl DoltTaskRegistryBackend {
    #[must_use]
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
        }
    }
}

impl TaskRegistryBackend for DoltTaskRegistryBackend {
    fn create(&self, _prompt: &str, _description: Option<&str>) -> Result<Task, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::create".to_string(),
        ))
    }

    fn create_from_packet(&self, _packet: TaskPacket) -> Result<Task, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::create_from_packet".to_string(),
        ))
    }

    fn get(&self, _task_id: &str) -> Result<Option<Task>, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::get".to_string(),
        ))
    }

    fn list(&self, _status_filter: Option<TaskStatus>) -> Result<Vec<Task>, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::list".to_string(),
        ))
    }

    fn stop(&self, _task_id: &str) -> Result<Task, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::stop".to_string(),
        ))
    }

    fn update(&self, _task_id: &str, _message: &str) -> Result<Task, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::update".to_string(),
        ))
    }

    fn output(&self, _task_id: &str) -> Result<String, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::output".to_string(),
        ))
    }

    fn append_output(&self, _task_id: &str, _output: &str) -> Result<(), TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::append_output".to_string(),
        ))
    }

    fn set_status(&self, _task_id: &str, _status: TaskStatus) -> Result<(), TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::set_status".to_string(),
        ))
    }

    fn assign_team(&self, _task_id: &str, _team_id: &str) -> Result<(), TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::assign_team".to_string(),
        ))
    }

    fn remove(&self, _task_id: &str) -> Result<Option<Task>, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::remove".to_string(),
        ))
    }

    fn len(&self) -> Result<usize, TaskRegistryError> {
        Err(TaskRegistryError::Unimplemented(
            "DoltTaskRegistryBackend::len".to_string(),
        ))
    }
}
