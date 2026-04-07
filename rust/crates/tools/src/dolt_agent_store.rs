#![allow(dead_code)]

use crate::agent_store::{AgentManifest, AgentStore, AgentStoreError};

/// Dolt-backed agent store.
///
/// Stores agent manifests, lane events, and output in a Dolt database using
/// the schema defined in `AGENT_STORE_SCHEMA_DESIGN.md`. Replaces
/// [`FileAgentStore`](crate::agent_store::FileAgentStore) for environments
/// where versioned, queryable storage is preferred.
#[derive(Debug, Clone)]
pub struct DoltAgentStore {
    /// Connection string or path to the Dolt database.
    pub connection: String,
}

impl DoltAgentStore {
    #[must_use]
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
        }
    }
}

impl AgentStore for DoltAgentStore {
    fn create_agent(&self, _manifest: &AgentManifest) -> Result<(), AgentStoreError> {
        Err(AgentStoreError::Unimplemented(
            "DoltAgentStore::create_agent".to_string(),
        ))
    }

    fn load_agent(&self, _agent_id: &str) -> Result<Option<AgentManifest>, AgentStoreError> {
        Err(AgentStoreError::Unimplemented(
            "DoltAgentStore::load_agent".to_string(),
        ))
    }

    fn update_agent(&self, _manifest: &AgentManifest) -> Result<(), AgentStoreError> {
        Err(AgentStoreError::Unimplemented(
            "DoltAgentStore::update_agent".to_string(),
        ))
    }

    fn write_output(&self, _agent_id: &str, _content: &str) -> Result<(), AgentStoreError> {
        Err(AgentStoreError::Unimplemented(
            "DoltAgentStore::write_output".to_string(),
        ))
    }

    fn append_output(&self, _agent_id: &str, _suffix: &str) -> Result<(), AgentStoreError> {
        Err(AgentStoreError::Unimplemented(
            "DoltAgentStore::append_output".to_string(),
        ))
    }

    fn list_agents(&self) -> Result<Vec<AgentManifest>, AgentStoreError> {
        Err(AgentStoreError::Unimplemented(
            "DoltAgentStore::list_agents".to_string(),
        ))
    }
}
