#![allow(dead_code)]

use crate::prompt_cache::{PromptCacheStats, TrackedPromptState};
use crate::prompt_cache_store::{PromptCacheStore, StoredCompletion};

/// Dolt-backed prompt cache store.
///
/// Stores completion entries, stats, and tracked prompt state in a Dolt
/// database using the schema defined in `PROMPT_CACHE_SCHEMA_DESIGN.md`.
/// Replaces [`FilePromptCacheStore`](crate::prompt_cache_store::FilePromptCacheStore)
/// for environments where versioned, queryable storage is preferred.
#[derive(Debug, Clone)]
pub struct DoltPromptCacheStore {
    /// Connection string or path to the Dolt database.
    pub connection: String,
}

impl DoltPromptCacheStore {
    #[must_use]
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
        }
    }
}

impl PromptCacheStore for DoltPromptCacheStore {
    fn read_completion(
        &self,
        _session_id: &str,
        _request_hash: &str,
    ) -> Option<StoredCompletion> {
        None // Unimplemented: DoltPromptCacheStore::read_completion
    }

    fn write_completion(
        &self,
        _session_id: &str,
        _request_hash: &str,
        _entry: &StoredCompletion,
    ) {
        // Unimplemented: DoltPromptCacheStore::write_completion
    }

    fn delete_completion(&self, _session_id: &str, _request_hash: &str) {
        // Unimplemented: DoltPromptCacheStore::delete_completion
    }

    fn load_state(
        &self,
        _session_id: &str,
    ) -> (PromptCacheStats, Option<TrackedPromptState>) {
        // Unimplemented: DoltPromptCacheStore::load_state
        (PromptCacheStats::default(), None)
    }

    fn persist_state(
        &self,
        _session_id: &str,
        _stats: &PromptCacheStats,
        _previous: Option<&TrackedPromptState>,
    ) {
        // Unimplemented: DoltPromptCacheStore::persist_state
    }
}
