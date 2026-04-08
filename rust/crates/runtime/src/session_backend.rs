#![allow(dead_code)]

use std::fmt::{Display, Formatter};

use crate::session::{
    ConversationMessage, Session, SessionCompaction, SessionError, SessionPromptEntry,
};
use crate::session_control::ManagedSessionSummary;

/// Abstraction over session persistence backends.
///
/// The file-backed [`SessionStore`](crate::session_control::SessionStore) is the
/// original implementation. [`DoltSessionBackend`](crate::dolt_session_backend::DoltSessionBackend)
/// is the planned replacement that stores sessions in a Dolt database.
/// Abstraction over session persistence backends.
///
/// Workspace isolation is handled internally by each backend — the file
/// backend uses directory fingerprinting, the Dolt backend uses branches
/// (see `DOLT_BRANCHING_STRATEGY.md`). Callers never pass workspace
/// identifiers.
pub trait SessionBackend {
    /// Persist a newly created session (metadata only, no messages yet).
    fn create_session(&self, session: &Session) -> Result<(), SessionBackendError>;

    /// Load a full session by ID: metadata, messages, prompt history.
    fn load_session(&self, session_id: &str) -> Result<Session, SessionBackendError>;

    /// Append a single conversation message to an existing session.
    fn append_message(
        &self,
        session_id: &str,
        message: &ConversationMessage,
    ) -> Result<(), SessionBackendError>;

    /// Append a prompt history entry to an existing session.
    fn append_prompt_entry(
        &self,
        session_id: &str,
        entry: &SessionPromptEntry,
    ) -> Result<(), SessionBackendError>;

    /// Persist a full snapshot of the session, replacing any prior state.
    fn save_snapshot(&self, session: &Session) -> Result<(), SessionBackendError>;

    /// Update compaction metadata and optionally remove messages below a given
    /// ordinal.
    fn update_compaction(
        &self,
        session_id: &str,
        compaction: &SessionCompaction,
        remove_messages_before_ordinal: Option<u32>,
    ) -> Result<(), SessionBackendError>;

    /// Fork a session: create a new session that copies messages and prompt
    /// history from `source_id`, recording fork lineage.
    fn fork_session(
        &self,
        source_id: &str,
        new_session_id: &str,
        branch_name: Option<&str>,
    ) -> Result<Session, SessionBackendError>;

    /// List sessions sorted by last-modified descending.
    fn list_sessions(&self) -> Result<Vec<ManagedSessionSummary>, SessionBackendError>;

    /// Resolve a session reference (ID, path, or alias like "latest") to a
    /// concrete session ID.
    fn resolve_reference(
        &self,
        reference: &str,
    ) -> Result<String, SessionBackendError>;

    /// Return true if a session with this ID exists in the backend.
    fn session_exists(&self, session_id: &str) -> Result<bool, SessionBackendError>;

    /// Delete a session and all associated data.
    fn delete_session(&self, session_id: &str) -> Result<(), SessionBackendError>;

    /// Return a human-readable description of where sessions are stored.
    ///
    /// For a file backend this is the directory path; for Dolt it is a
    /// connection URI.
    fn storage_location(&self) -> String;

    /// Return the filesystem path for a session, if the backend is file-based.
    ///
    /// Non-file backends return `None`.
    fn session_path(&self, _session_id: &str) -> Option<std::path::PathBuf> {
        None
    }

    /// Return the most recently modified session.
    ///
    /// Default implementation delegates to [`list_sessions`](Self::list_sessions).
    fn latest_session(&self) -> Result<ManagedSessionSummary, SessionBackendError> {
        self.list_sessions()?
            .into_iter()
            .next()
            .ok_or_else(|| {
                SessionBackendError::Format(
                    "no managed sessions found\nStart `claw` to create a session, then rerun with `--resume latest`.".to_string(),
                )
            })
    }
}

/// Errors raised by [`SessionBackend`] implementations.
#[derive(Debug)]
pub enum SessionBackendError {
    /// An I/O or transport-level error.
    Io(std::io::Error),
    /// A session-layer format or data error.
    Session(SessionError),
    /// A human-readable error message (resolution failures, missing sessions, etc.).
    Format(String),
    /// The operation is not implemented by this backend.
    Unimplemented(String),
}

impl Display for SessionBackendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Session(error) => write!(f, "{error}"),
            Self::Format(message) => write!(f, "{message}"),
            Self::Unimplemented(operation) => {
                write!(f, "operation not implemented: {operation}")
            }
        }
    }
}

impl std::error::Error for SessionBackendError {}

impl From<std::io::Error> for SessionBackendError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<SessionError> for SessionBackendError {
    fn from(value: SessionError) -> Self {
        Self::Session(value)
    }
}

/// Convenience conversion so callers that hold a [`SessionBackendError`] can
/// propagate it where a [`SessionError`] is expected (lossy — non-session
/// variants become `SessionError::Format`).
impl From<SessionBackendError> for SessionError {
    fn from(value: SessionBackendError) -> Self {
        match value {
            SessionBackendError::Session(inner) => inner,
            SessionBackendError::Io(inner) => SessionError::Io(inner),
            SessionBackendError::Format(msg) | SessionBackendError::Unimplemented(msg) => {
                SessionError::Format(msg)
            }
        }
    }
}
