#![allow(dead_code)]

use crate::session::{ConversationMessage, Session, SessionCompaction, SessionPromptEntry};
use crate::session_backend::{SessionBackend, SessionBackendError};
use crate::session_control::ManagedSessionSummary;

/// Dolt-backed session persistence.
///
/// Stores sessions, messages, and prompt history in a Dolt database using the
/// schema defined in `SESSIONS_SCHEMA_DESIGN.md`. Replaces the file-backed
/// [`SessionStore`](crate::session_control::SessionStore) for environments
/// where versioned, queryable storage is preferred.
#[derive(Debug, Clone)]
pub struct DoltSessionBackend {
    /// Connection string or path to the Dolt database.
    pub connection: String,
}

impl DoltSessionBackend {
    #[must_use]
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
        }
    }
}

impl SessionBackend for DoltSessionBackend {
    fn create_session(
        &self,
        _session: &Session,
        _workspace_fingerprint: &str,
    ) -> Result<(), SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::create_session".to_string(),
        ))
    }

    fn load_session(&self, _session_id: &str) -> Result<Session, SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::load_session".to_string(),
        ))
    }

    fn append_message(
        &self,
        _session_id: &str,
        _message: &ConversationMessage,
    ) -> Result<(), SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::append_message".to_string(),
        ))
    }

    fn append_prompt_entry(
        &self,
        _session_id: &str,
        _entry: &SessionPromptEntry,
    ) -> Result<(), SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::append_prompt_entry".to_string(),
        ))
    }

    fn save_snapshot(&self, _session: &Session) -> Result<(), SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::save_snapshot".to_string(),
        ))
    }

    fn update_compaction(
        &self,
        _session_id: &str,
        _compaction: &SessionCompaction,
        _remove_messages_before_ordinal: Option<u32>,
    ) -> Result<(), SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::update_compaction".to_string(),
        ))
    }

    fn fork_session(
        &self,
        _source_id: &str,
        _new_session_id: &str,
        _branch_name: Option<&str>,
    ) -> Result<Session, SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::fork_session".to_string(),
        ))
    }

    fn list_sessions(
        &self,
        _workspace_fingerprint: &str,
    ) -> Result<Vec<ManagedSessionSummary>, SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::list_sessions".to_string(),
        ))
    }

    fn resolve_reference(
        &self,
        _workspace_fingerprint: &str,
        _reference: &str,
    ) -> Result<String, SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::resolve_reference".to_string(),
        ))
    }

    fn session_exists(&self, _session_id: &str) -> Result<bool, SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::session_exists".to_string(),
        ))
    }

    fn delete_session(&self, _session_id: &str) -> Result<(), SessionBackendError> {
        Err(SessionBackendError::Unimplemented(
            "DoltSessionBackend::delete_session".to_string(),
        ))
    }
}
