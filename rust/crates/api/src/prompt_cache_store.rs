#![allow(dead_code)]

use std::fmt::Debug;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::prompt_cache::{PromptCachePaths, PromptCacheStats, TrackedPromptState};
use crate::types::MessageResponse;

/// A cached completion entry as stored and retrieved by a [`PromptCacheStore`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCompletion {
    pub cached_at_unix_secs: u64,
    pub fingerprint_version: u32,
    pub response: MessageResponse,
}

/// Low-level storage operations for the prompt cache.
///
/// This trait abstracts the read/write layer so that the business logic in
/// [`PromptCache`](crate::prompt_cache::PromptCache) (fingerprinting, cache
/// break detection, stats accumulation, TTL enforcement) can be shared across
/// backends.
///
/// [`FilePromptCacheStore`] is the original file-backed implementation.
/// [`DoltPromptCacheStore`](crate::dolt_prompt_cache_store::DoltPromptCacheStore)
/// is the planned Dolt replacement.
pub trait PromptCacheStore: Debug + Send + Sync {
    /// Read a cached completion entry by session and request hash.
    /// Returns `None` if the entry does not exist on disk / in the store.
    fn read_completion(&self, session_id: &str, request_hash: &str) -> Option<StoredCompletion>;

    /// Write (or overwrite) a completion entry.
    fn write_completion(&self, session_id: &str, request_hash: &str, entry: &StoredCompletion);

    /// Delete a single completion entry (e.g. on TTL expiry or version mismatch).
    fn delete_completion(&self, session_id: &str, request_hash: &str);

    /// Load the persisted stats and tracked prompt state for a session.
    /// Returns defaults if nothing has been persisted yet.
    fn load_state(&self, session_id: &str) -> (PromptCacheStats, Option<TrackedPromptState>);

    /// Persist the current stats and tracked prompt state for a session.
    fn persist_state(
        &self,
        session_id: &str,
        stats: &PromptCacheStats,
        previous: Option<&TrackedPromptState>,
    );
}

// ---------------------------------------------------------------------------
// File-backed implementation (original behavior)
// ---------------------------------------------------------------------------

/// File-backed prompt cache store.
///
/// Stores completion entries as individual JSON files under
/// `<cache_root>/<session_id>/completions/<hash>.json`, with stats and session
/// state as sibling JSON files.
#[derive(Debug, Clone)]
pub struct FilePromptCacheStore;

impl FilePromptCacheStore {
    fn paths_for(session_id: &str) -> PromptCachePaths {
        PromptCachePaths::for_session(session_id)
    }

    fn ensure_dirs(paths: &PromptCachePaths) {
        let _ = fs::create_dir_all(&paths.completion_dir);
    }
}

impl PromptCacheStore for FilePromptCacheStore {
    fn read_completion(&self, session_id: &str, request_hash: &str) -> Option<StoredCompletion> {
        let paths = Self::paths_for(session_id);
        let entry_path = paths.completion_entry_path(request_hash);
        read_json(&entry_path)
    }

    fn write_completion(&self, session_id: &str, request_hash: &str, entry: &StoredCompletion) {
        let paths = Self::paths_for(session_id);
        Self::ensure_dirs(&paths);
        let _ = write_json(&paths.completion_entry_path(request_hash), entry);
    }

    fn delete_completion(&self, session_id: &str, request_hash: &str) {
        let paths = Self::paths_for(session_id);
        let _ = fs::remove_file(paths.completion_entry_path(request_hash));
    }

    fn load_state(&self, session_id: &str) -> (PromptCacheStats, Option<TrackedPromptState>) {
        let paths = Self::paths_for(session_id);
        let stats = read_json::<PromptCacheStats>(&paths.stats_path).unwrap_or_default();
        let previous = read_json::<TrackedPromptState>(&paths.session_state_path);
        (stats, previous)
    }

    fn persist_state(
        &self,
        session_id: &str,
        stats: &PromptCacheStats,
        previous: Option<&TrackedPromptState>,
    ) {
        let paths = Self::paths_for(session_id);
        Self::ensure_dirs(&paths);
        let _ = write_json(&paths.stats_path, stats);
        if let Some(previous) = previous {
            let _ = write_json(&paths.session_state_path, previous);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared file helpers
// ---------------------------------------------------------------------------

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    fs::write(path, json)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}
