#![allow(dead_code)]

use std::fmt::{Display, Formatter};

use crate::task_registry::{Task, TaskStatus};
use crate::{TaskPacket, TaskPacketValidationError};

/// Abstraction over task registry persistence backends.
///
/// The in-memory [`TaskRegistry`](crate::task_registry::TaskRegistry) is the
/// original implementation. [`DoltTaskRegistryBackend`](crate::dolt_task_registry_backend::DoltTaskRegistryBackend)
/// is the planned replacement that persists tasks in a Dolt database.
pub trait TaskRegistryBackend: Send + Sync {
    /// Create a new task from a prompt and optional description.
    fn create(&self, prompt: &str, description: Option<&str>) -> Result<Task, TaskRegistryError>;

    /// Create a new task from a validated [`TaskPacket`].
    fn create_from_packet(&self, packet: TaskPacket) -> Result<Task, TaskRegistryError>;

    /// Retrieve a task by ID. Returns `None` if not found.
    fn get(&self, task_id: &str) -> Result<Option<Task>, TaskRegistryError>;

    /// List all tasks, optionally filtered by status.
    fn list(&self, status_filter: Option<TaskStatus>) -> Result<Vec<Task>, TaskRegistryError>;

    /// Stop a task. Rejects tasks already in a terminal state.
    fn stop(&self, task_id: &str) -> Result<Task, TaskRegistryError>;

    /// Append a user message to a task.
    fn update(&self, task_id: &str, message: &str) -> Result<Task, TaskRegistryError>;

    /// Get the accumulated output of a task.
    fn output(&self, task_id: &str) -> Result<String, TaskRegistryError>;

    /// Append text to a task's output.
    fn append_output(&self, task_id: &str, output: &str) -> Result<(), TaskRegistryError>;

    /// Set a task's status directly.
    fn set_status(&self, task_id: &str, status: TaskStatus) -> Result<(), TaskRegistryError>;

    /// Assign a task to a team.
    fn assign_team(&self, task_id: &str, team_id: &str) -> Result<(), TaskRegistryError>;

    /// Remove a task and all associated data.
    fn remove(&self, task_id: &str) -> Result<Option<Task>, TaskRegistryError>;

    /// Return the number of tasks in the registry.
    fn len(&self) -> Result<usize, TaskRegistryError>;

    /// Return true if the registry contains no tasks.
    fn is_empty(&self) -> Result<bool, TaskRegistryError> {
        Ok(self.len()? == 0)
    }
}

/// Errors raised by [`TaskRegistryBackend`] implementations.
#[derive(Debug)]
pub enum TaskRegistryError {
    /// Task not found.
    NotFound(String),
    /// Invalid state transition (e.g., stopping an already-completed task).
    InvalidState(String),
    /// Validation error (e.g., invalid task packet).
    Validation(TaskPacketValidationError),
    /// A human-readable error message.
    Format(String),
    /// The operation is not implemented by this backend.
    Unimplemented(String),
}

impl Display for TaskRegistryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "task not found: {id}"),
            Self::InvalidState(msg) => write!(f, "{msg}"),
            Self::Validation(err) => write!(f, "{err}"),
            Self::Format(msg) => write!(f, "{msg}"),
            Self::Unimplemented(op) => write!(f, "operation not implemented: {op}"),
        }
    }
}

impl std::error::Error for TaskRegistryError {}

impl From<TaskPacketValidationError> for TaskRegistryError {
    fn from(value: TaskPacketValidationError) -> Self {
        Self::Validation(value)
    }
}

/// Convenience conversion so callers that expect `Result<_, String>` can use
/// the `?` operator with `TaskRegistryError`.
impl From<TaskRegistryError> for String {
    fn from(value: TaskRegistryError) -> Self {
        value.to_string()
    }
}
