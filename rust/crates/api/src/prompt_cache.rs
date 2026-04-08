use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::prompt_cache_store::{FilePromptCacheStore, PromptCacheStore, StoredCompletion};
use crate::types::{MessageRequest, MessageResponse, Usage};

const DEFAULT_COMPLETION_TTL_SECS: u64 = 30;
const DEFAULT_PROMPT_TTL_SECS: u64 = 5 * 60;
const DEFAULT_BREAK_MIN_DROP: u32 = 2_000;
const MAX_SANITIZED_LENGTH: usize = 80;
const REQUEST_FINGERPRINT_VERSION: u32 = 1;
const REQUEST_FINGERPRINT_PREFIX: &str = "v1";
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Debug, Clone)]
pub struct PromptCacheConfig {
    pub session_id: String,
    pub completion_ttl: Duration,
    pub prompt_ttl: Duration,
    pub cache_break_min_drop: u32,
}

impl PromptCacheConfig {
    #[must_use]
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            completion_ttl: Duration::from_secs(DEFAULT_COMPLETION_TTL_SECS),
            prompt_ttl: Duration::from_secs(DEFAULT_PROMPT_TTL_SECS),
            cache_break_min_drop: DEFAULT_BREAK_MIN_DROP,
        }
    }
}

impl Default for PromptCacheConfig {
    fn default() -> Self {
        Self::new("default")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCachePaths {
    pub root: PathBuf,
    pub session_dir: PathBuf,
    pub completion_dir: PathBuf,
    pub session_state_path: PathBuf,
    pub stats_path: PathBuf,
}

impl PromptCachePaths {
    #[must_use]
    pub fn for_session(session_id: &str) -> Self {
        let root = base_cache_root();
        let session_dir = root.join(sanitize_path_segment(session_id));
        let completion_dir = session_dir.join("completions");
        Self {
            root,
            session_state_path: session_dir.join("session-state.json"),
            stats_path: session_dir.join("stats.json"),
            session_dir,
            completion_dir,
        }
    }

    #[must_use]
    pub fn completion_entry_path(&self, request_hash: &str) -> PathBuf {
        self.completion_dir.join(format!("{request_hash}.json"))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCacheStats {
    pub tracked_requests: u64,
    pub completion_cache_hits: u64,
    pub completion_cache_misses: u64,
    pub completion_cache_writes: u64,
    pub expected_invalidations: u64,
    pub unexpected_cache_breaks: u64,
    pub total_cache_creation_input_tokens: u64,
    pub total_cache_read_input_tokens: u64,
    pub last_cache_creation_input_tokens: Option<u32>,
    pub last_cache_read_input_tokens: Option<u32>,
    pub last_request_hash: Option<String>,
    pub last_completion_cache_key: Option<String>,
    pub last_break_reason: Option<String>,
    pub last_cache_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheBreakEvent {
    pub unexpected: bool,
    pub reason: String,
    pub previous_cache_read_input_tokens: u32,
    pub current_cache_read_input_tokens: u32,
    pub token_drop: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCacheRecord {
    pub cache_break: Option<CacheBreakEvent>,
    pub stats: PromptCacheStats,
}

#[derive(Debug, Clone)]
pub struct PromptCache {
    inner: Arc<Mutex<PromptCacheInner>>,
}

impl PromptCache {
    /// Create a prompt cache for the given session using the default
    /// file-backed store.
    #[must_use]
    pub fn new(session_id: impl Into<String>) -> Self {
        Self::with_config(PromptCacheConfig::new(session_id))
    }

    /// Create a prompt cache with custom configuration using the default
    /// file-backed store.
    #[must_use]
    pub fn with_config(config: PromptCacheConfig) -> Self {
        Self::with_store(config, Arc::new(FilePromptCacheStore))
    }

    /// Create a prompt cache backed by the given [`PromptCacheStore`]
    /// implementation.
    #[must_use]
    pub fn with_store(config: PromptCacheConfig, store: Arc<dyn PromptCacheStore>) -> Self {
        let (stats, previous) = store.load_state(&config.session_id);
        Self {
            inner: Arc::new(Mutex::new(PromptCacheInner {
                config,
                store,
                stats,
                previous,
            })),
        }
    }

    /// Return the filesystem paths for this session.
    ///
    /// This is a convenience for callers that know the underlying store is
    /// file-backed. The paths are derived from the session ID regardless of
    /// which store is in use.
    #[must_use]
    pub fn paths(&self) -> PromptCachePaths {
        let session_id = self.lock().config.session_id.clone();
        PromptCachePaths::for_session(&session_id)
    }

    #[must_use]
    pub fn stats(&self) -> PromptCacheStats {
        self.lock().stats.clone()
    }

    #[must_use]
    pub fn lookup_completion(&self, request: &MessageRequest) -> Option<MessageResponse> {
        let request_hash = request_hash_hex(request);
        let (session_id, store, ttl) = {
            let inner = self.lock();
            (
                inner.config.session_id.clone(),
                Arc::clone(&inner.store),
                inner.config.completion_ttl,
            )
        };

        let entry = store.read_completion(&session_id, &request_hash);
        let Some(entry) = entry else {
            let mut inner = self.lock();
            inner.stats.completion_cache_misses += 1;
            inner.stats.last_completion_cache_key = Some(request_hash);
            persist_state(&inner);
            return None;
        };

        if entry.fingerprint_version != current_fingerprint_version() {
            let mut inner = self.lock();
            inner.stats.completion_cache_misses += 1;
            inner.stats.last_completion_cache_key = Some(request_hash.clone());
            store.delete_completion(&session_id, &request_hash);
            persist_state(&inner);
            return None;
        }

        let expired = now_unix_secs().saturating_sub(entry.cached_at_unix_secs) >= ttl.as_secs();
        let mut inner = self.lock();
        inner.stats.last_completion_cache_key = Some(request_hash.clone());
        if expired {
            inner.stats.completion_cache_misses += 1;
            store.delete_completion(&session_id, &request_hash);
            persist_state(&inner);
            return None;
        }

        inner.stats.completion_cache_hits += 1;
        apply_usage_to_stats(
            &mut inner.stats,
            &entry.response.usage,
            &request_hash,
            "completion-cache",
        );
        inner.previous = Some(TrackedPromptState::from_usage(
            request,
            &entry.response.usage,
        ));
        persist_state(&inner);
        Some(entry.response)
    }

    #[must_use]
    pub fn record_response(
        &self,
        request: &MessageRequest,
        response: &MessageResponse,
    ) -> PromptCacheRecord {
        self.record_usage_internal(request, &response.usage, Some(response))
    }

    #[must_use]
    pub fn record_usage(&self, request: &MessageRequest, usage: &Usage) -> PromptCacheRecord {
        self.record_usage_internal(request, usage, None)
    }

    fn record_usage_internal(
        &self,
        request: &MessageRequest,
        usage: &Usage,
        response: Option<&MessageResponse>,
    ) -> PromptCacheRecord {
        let request_hash = request_hash_hex(request);
        let mut inner = self.lock();
        let previous = inner.previous.clone();
        let current = TrackedPromptState::from_usage(request, usage);
        let cache_break = detect_cache_break(&inner.config, previous.as_ref(), &current);

        inner.stats.tracked_requests += 1;
        apply_usage_to_stats(&mut inner.stats, usage, &request_hash, "api-response");
        if let Some(event) = &cache_break {
            if event.unexpected {
                inner.stats.unexpected_cache_breaks += 1;
            } else {
                inner.stats.expected_invalidations += 1;
            }
            inner.stats.last_break_reason = Some(event.reason.clone());
        }

        inner.previous = Some(current);
        if let Some(response) = response {
            let stored = StoredCompletion {
                cached_at_unix_secs: now_unix_secs(),
                fingerprint_version: current_fingerprint_version(),
                response: response.clone(),
            };
            inner
                .store
                .write_completion(&inner.config.session_id, &request_hash, &stored);
            inner.stats.completion_cache_writes += 1;
        }
        persist_state(&inner);

        PromptCacheRecord {
            cache_break,
            stats: inner.stats.clone(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PromptCacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug)]
struct PromptCacheInner {
    config: PromptCacheConfig,
    store: Arc<dyn PromptCacheStore>,
    stats: PromptCacheStats,
    previous: Option<TrackedPromptState>,
}

/// Tracked prompt state persisted between requests for cache break detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TrackedPromptState {
    pub(crate) observed_at_unix_secs: u64,
    #[serde(default = "current_fingerprint_version")]
    pub(crate) fingerprint_version: u32,
    pub(crate) model_hash: u64,
    pub(crate) system_hash: u64,
    pub(crate) tools_hash: u64,
    pub(crate) messages_hash: u64,
    pub(crate) cache_read_input_tokens: u32,
}

impl TrackedPromptState {
    pub(crate) fn from_usage(request: &MessageRequest, usage: &Usage) -> Self {
        let hashes = RequestFingerprints::from_request(request);
        Self {
            observed_at_unix_secs: now_unix_secs(),
            fingerprint_version: current_fingerprint_version(),
            model_hash: hashes.model,
            system_hash: hashes.system,
            tools_hash: hashes.tools,
            messages_hash: hashes.messages,
            cache_read_input_tokens: usage.cache_read_input_tokens,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RequestFingerprints {
    model: u64,
    system: u64,
    tools: u64,
    messages: u64,
}

impl RequestFingerprints {
    fn from_request(request: &MessageRequest) -> Self {
        Self {
            model: hash_serializable(&request.model),
            system: hash_serializable(&request.system),
            tools: hash_serializable(&request.tools),
            messages: hash_serializable(&request.messages),
        }
    }
}

fn detect_cache_break(
    config: &PromptCacheConfig,
    previous: Option<&TrackedPromptState>,
    current: &TrackedPromptState,
) -> Option<CacheBreakEvent> {
    let previous = previous?;
    if previous.fingerprint_version != current.fingerprint_version {
        return Some(CacheBreakEvent {
            unexpected: false,
            reason: format!(
                "fingerprint version changed (v{} -> v{})",
                previous.fingerprint_version, current.fingerprint_version
            ),
            previous_cache_read_input_tokens: previous.cache_read_input_tokens,
            current_cache_read_input_tokens: current.cache_read_input_tokens,
            token_drop: previous
                .cache_read_input_tokens
                .saturating_sub(current.cache_read_input_tokens),
        });
    }
    let token_drop = previous
        .cache_read_input_tokens
        .saturating_sub(current.cache_read_input_tokens);
    if token_drop < config.cache_break_min_drop {
        return None;
    }

    let mut reasons = Vec::new();
    if previous.model_hash != current.model_hash {
        reasons.push("model changed");
    }
    if previous.system_hash != current.system_hash {
        reasons.push("system prompt changed");
    }
    if previous.tools_hash != current.tools_hash {
        reasons.push("tool definitions changed");
    }
    if previous.messages_hash != current.messages_hash {
        reasons.push("message payload changed");
    }

    let elapsed = current
        .observed_at_unix_secs
        .saturating_sub(previous.observed_at_unix_secs);

    let (unexpected, reason) = if reasons.is_empty() {
        if elapsed > config.prompt_ttl.as_secs() {
            (
                false,
                format!("possible prompt cache TTL expiry after {elapsed}s"),
            )
        } else {
            (
                true,
                "cache read tokens dropped while prompt fingerprint remained stable".to_string(),
            )
        }
    } else {
        (false, reasons.join(", "))
    };

    Some(CacheBreakEvent {
        unexpected,
        reason,
        previous_cache_read_input_tokens: previous.cache_read_input_tokens,
        current_cache_read_input_tokens: current.cache_read_input_tokens,
        token_drop,
    })
}

fn apply_usage_to_stats(
    stats: &mut PromptCacheStats,
    usage: &Usage,
    request_hash: &str,
    source: &str,
) {
    stats.total_cache_creation_input_tokens += u64::from(usage.cache_creation_input_tokens);
    stats.total_cache_read_input_tokens += u64::from(usage.cache_read_input_tokens);
    stats.last_cache_creation_input_tokens = Some(usage.cache_creation_input_tokens);
    stats.last_cache_read_input_tokens = Some(usage.cache_read_input_tokens);
    stats.last_request_hash = Some(request_hash.to_string());
    stats.last_cache_source = Some(source.to_string());
}

fn persist_state(inner: &PromptCacheInner) {
    inner.store.persist_state(
        &inner.config.session_id,
        &inner.stats,
        inner.previous.as_ref(),
    );
}

fn request_hash_hex(request: &MessageRequest) -> String {
    format!(
        "{REQUEST_FINGERPRINT_PREFIX}-{:016x}",
        hash_serializable(request)
    )
}

fn hash_serializable<T: Serialize>(value: &T) -> u64 {
    let json = serde_json::to_vec(value).unwrap_or_default();
    stable_hash_bytes(&json)
}

pub(crate) fn sanitize_path_segment(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        return sanitized;
    }
    let suffix = format!("-{:x}", hash_string(value));
    format!(
        "{}{}",
        &sanitized[..MAX_SANITIZED_LENGTH.saturating_sub(suffix.len())],
        suffix
    )
}

fn hash_string(value: &str) -> u64 {
    stable_hash_bytes(value.as_bytes())
}

pub(crate) fn base_cache_root() -> PathBuf {
    if let Some(config_home) = std::env::var_os("CLAUDE_CONFIG_HOME") {
        return PathBuf::from(config_home)
            .join("cache")
            .join("prompt-cache");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".claude")
            .join("cache")
            .join("prompt-cache");
    }
    std::env::temp_dir().join("claude-prompt-cache")
}

pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

const fn current_fingerprint_version() -> u32 {
    REQUEST_FINGERPRINT_VERSION
}

fn stable_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        detect_cache_break, request_hash_hex, sanitize_path_segment, PromptCache,
        PromptCacheConfig, PromptCachePaths, TrackedPromptState, REQUEST_FINGERPRINT_PREFIX,
    };
    use crate::types::{InputMessage, MessageRequest, MessageResponse, OutputContentBlock, Usage};

    fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn path_builder_sanitizes_session_identifier() {
        let paths = PromptCachePaths::for_session("session:/with spaces");
        let session_dir = paths
            .session_dir
            .file_name()
            .and_then(|value| value.to_str())
            .expect("session dir name");
        assert_eq!(session_dir, "session--with-spaces");
        assert!(paths.completion_dir.ends_with("completions"));
        assert!(paths.stats_path.ends_with("stats.json"));
        assert!(paths.session_state_path.ends_with("session-state.json"));
    }

    #[test]
    fn request_fingerprint_drives_unexpected_break_detection() {
        let request = sample_request("same");
        let previous = TrackedPromptState::from_usage(
            &request,
            &Usage {
                input_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 6_000,
                output_tokens: 0,
            },
        );
        let current = TrackedPromptState::from_usage(
            &request,
            &Usage {
                input_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 1_000,
                output_tokens: 0,
            },
        );
        let event = detect_cache_break(&PromptCacheConfig::default(), Some(&previous), &current)
            .expect("break should be detected");
        assert!(event.unexpected);
        assert!(event.reason.contains("stable"));
    }

    #[test]
    fn changed_prompt_marks_break_as_expected() {
        let previous_request = sample_request("first");
        let current_request = sample_request("second");
        let previous = TrackedPromptState::from_usage(
            &previous_request,
            &Usage {
                input_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 6_000,
                output_tokens: 0,
            },
        );
        let current = TrackedPromptState::from_usage(
            &current_request,
            &Usage {
                input_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 1_000,
                output_tokens: 0,
            },
        );
        let event = detect_cache_break(&PromptCacheConfig::default(), Some(&previous), &current)
            .expect("break should be detected");
        assert!(!event.unexpected);
        assert!(event.reason.contains("message payload changed"));
    }

    #[test]
    fn completion_cache_round_trip_persists_recent_response() {
        let _guard = test_env_lock();
        let temp_root = std::env::temp_dir().join(format!(
            "prompt-cache-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::env::set_var("CLAUDE_CONFIG_HOME", &temp_root);
        let cache = PromptCache::new("unit-test-session");
        let request = sample_request("cache me");
        let response = sample_response(42, 12, "cached");

        assert!(cache.lookup_completion(&request).is_none());
        let record = cache.record_response(&request, &response);
        assert!(record.cache_break.is_none());

        let cached = cache
            .lookup_completion(&request)
            .expect("cached response should load");
        assert_eq!(cached.content, response.content);

        let stats = cache.stats();
        assert_eq!(stats.completion_cache_hits, 1);
        assert_eq!(stats.completion_cache_misses, 1);
        assert_eq!(stats.completion_cache_writes, 1);

        let persisted_bytes =
            std::fs::read(&cache.paths().stats_path).expect("stats file should exist");
        let persisted: super::PromptCacheStats =
            serde_json::from_slice(&persisted_bytes).expect("stats should deserialize");
        assert_eq!(persisted.completion_cache_hits, 1);

        std::fs::remove_dir_all(temp_root).expect("cleanup temp root");
        std::env::remove_var("CLAUDE_CONFIG_HOME");
    }

    #[test]
    fn distinct_requests_do_not_collide_in_completion_cache() {
        let _guard = test_env_lock();
        let temp_root = std::env::temp_dir().join(format!(
            "prompt-cache-distinct-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::env::set_var("CLAUDE_CONFIG_HOME", &temp_root);
        let cache = PromptCache::new("distinct-request-session");
        let first_request = sample_request("first");
        let second_request = sample_request("second");

        let response = sample_response(42, 12, "cached");
        let _ = cache.record_response(&first_request, &response);

        assert!(cache.lookup_completion(&second_request).is_none());

        std::fs::remove_dir_all(temp_root).expect("cleanup temp root");
        std::env::remove_var("CLAUDE_CONFIG_HOME");
    }

    #[test]
    fn expired_completion_entries_are_not_reused() {
        let _guard = test_env_lock();
        let temp_root = std::env::temp_dir().join(format!(
            "prompt-cache-expired-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::env::set_var("CLAUDE_CONFIG_HOME", &temp_root);
        let cache = PromptCache::with_config(PromptCacheConfig {
            session_id: "expired-session".to_string(),
            completion_ttl: Duration::ZERO,
            ..PromptCacheConfig::default()
        });
        let request = sample_request("expire me");
        let response = sample_response(7, 3, "stale");

        let _ = cache.record_response(&request, &response);

        assert!(cache.lookup_completion(&request).is_none());
        let stats = cache.stats();
        assert_eq!(stats.completion_cache_hits, 0);
        assert_eq!(stats.completion_cache_misses, 1);

        std::fs::remove_dir_all(temp_root).expect("cleanup temp root");
        std::env::remove_var("CLAUDE_CONFIG_HOME");
    }

    #[test]
    fn sanitize_path_caps_long_values() {
        let long_value = "x".repeat(200);
        let sanitized = sanitize_path_segment(&long_value);
        assert!(sanitized.len() <= 80);
    }

    #[test]
    fn request_hashes_are_versioned_and_stable() {
        let request = sample_request("stable");
        let first = request_hash_hex(&request);
        let second = request_hash_hex(&request);
        assert_eq!(first, second);
        assert!(first.starts_with(REQUEST_FINGERPRINT_PREFIX));
    }

    fn sample_request(text: &str) -> MessageRequest {
        MessageRequest {
            model: "claude-3-7-sonnet-latest".to_string(),
            max_tokens: 64,
            messages: vec![InputMessage::user_text(text)],
            system: Some("system".to_string()),
            tools: None,
            tool_choice: None,
            stream: false,
        }
    }

    fn sample_response(
        cache_read_input_tokens: u32,
        output_tokens: u32,
        text: &str,
    ) -> MessageResponse {
        MessageResponse {
            id: "msg_test".to_string(),
            kind: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContentBlock::Text {
                text: text.to_string(),
            }],
            model: "claude-3-7-sonnet-latest".to_string(),
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: Usage {
                input_tokens: 10,
                cache_creation_input_tokens: 5,
                cache_read_input_tokens,
                output_tokens,
            },
            request_id: Some("req_test".to_string()),
        }
    }
}
