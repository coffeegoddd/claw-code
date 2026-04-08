# Prompt Cache Schema Design: Dolt Migration

> Proposed Dolt schema to replace the file-backed `PromptCache` persistence
> layer. See `SOTW_PERSISTENCE.md` for the full persistence catalog and
> `SESSIONS_SCHEMA_DESIGN.md` for the companion session store migration.

## Current Implementation

`PromptCache` (`api/src/prompt_cache.rs`) is a concrete struct wrapping
`Arc<Mutex<PromptCacheInner>>`. It persists three categories of data as JSON
files under `~/.claude/cache/prompt-cache/<session_id>/`:

1. **Completion entries** — one file per request hash at
   `completions/<hash>.json`. Contains the full `MessageResponse`, a
   timestamp, and a fingerprint version. TTL: 30 s.
2. **Session state** — `session-state.json` tracking `TrackedPromptState`
   (FNV-1a fingerprint hashes for model/system/tools/messages + cache-read
   token count). Used to detect cache breaks between consecutive requests.
3. **Stats** — `stats.json` with aggregate counters (hits, misses, writes,
   unexpected breaks, token totals).

All writes are fire-and-forget (`let _ =`). Cache failures are silently
swallowed — this is a performance optimization layer, not critical data.

There is no trait abstraction. There are no ad-hoc bypasses — all file I/O
goes through `persist_state()`, `write_completion_entry()`, `read_json()`, and
`write_json()` internal helpers.

---

## Guiding Decisions

1. **Merge stats + session state into one table** — both are 1:1 per session
   and always read/written together in `persist_state()`. No reason for two
   tables.

2. **Completion entries become rows, not files** — eliminates the
   one-file-per-hash directory sprawl. TTL expiry becomes a `WHERE` filter
   instead of `fs::remove_file` on read.

3. **`response_json` stays as a JSON blob** — `MessageResponse` is an opaque
   API type from the `api` crate. Normalizing it into columns would couple the
   cache schema to the Anthropic response format. The cache only ever stores
   and returns the full response, never queries inside it.

4. **Fingerprint hashes stay as `BIGINT UNSIGNED`** — they are already u64
   FNV-1a values. Storing them as integers enables range queries and is more
   compact than hex strings.

5. **No rotation or cleanup bookkeeping** — Dolt versioning replaces
   file-level cleanup. Expired entries can be reaped with a single `DELETE`.

---

## Tables

### 1. `prompt_cache_sessions`

One row per cache session. Replaces `stats.json` and `session-state.json`.

```sql
CREATE TABLE prompt_cache_sessions (
    session_id                        VARCHAR(128)    NOT NULL,

    -- Aggregate stats (replaces stats.json / PromptCacheStats)
    tracked_requests                  BIGINT UNSIGNED NOT NULL DEFAULT 0,
    completion_cache_hits             BIGINT UNSIGNED NOT NULL DEFAULT 0,
    completion_cache_misses           BIGINT UNSIGNED NOT NULL DEFAULT 0,
    completion_cache_writes           BIGINT UNSIGNED NOT NULL DEFAULT 0,
    expected_invalidations            BIGINT UNSIGNED NOT NULL DEFAULT 0,
    unexpected_cache_breaks           BIGINT UNSIGNED NOT NULL DEFAULT 0,
    total_cache_creation_input_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
    total_cache_read_input_tokens     BIGINT UNSIGNED NOT NULL DEFAULT 0,
    last_cache_creation_input_tokens  INT UNSIGNED,
    last_cache_read_input_tokens      INT UNSIGNED,
    last_request_hash                 VARCHAR(64),
    last_completion_cache_key         VARCHAR(64),
    last_break_reason                 TEXT,
    last_cache_source                 VARCHAR(32),

    -- Tracked prompt state (replaces session-state.json / TrackedPromptState)
    prev_observed_at_unix_secs        BIGINT UNSIGNED,
    prev_fingerprint_version          INT UNSIGNED,
    prev_model_hash                   BIGINT UNSIGNED,
    prev_system_hash                  BIGINT UNSIGNED,
    prev_tools_hash                   BIGINT UNSIGNED,
    prev_messages_hash                BIGINT UNSIGNED,
    prev_cache_read_input_tokens      INT UNSIGNED,

    PRIMARY KEY (session_id)
);
```

### 2. `prompt_cache_entries`

One row per cached completion. Replaces the `completions/<hash>.json` files.

```sql
CREATE TABLE prompt_cache_entries (
    session_id              VARCHAR(128)    NOT NULL,
    request_hash            VARCHAR(64)     NOT NULL,
    cached_at_unix_secs     BIGINT UNSIGNED NOT NULL,
    fingerprint_version     INT UNSIGNED    NOT NULL,
    response_json           JSON            NOT NULL,

    PRIMARY KEY (session_id, request_hash),
    INDEX idx_expiry (session_id, cached_at_unix_secs),
    CONSTRAINT fk_entries_session
        FOREIGN KEY (session_id) REFERENCES prompt_cache_sessions(session_id)
        ON DELETE CASCADE
);
```

---

## Operation Mapping

### Create / initialize cache session

**File-backed:** `PromptCache::with_config()` reads `stats.json` +
`session-state.json`, defaults if missing.

**Dolt:**
```sql
INSERT IGNORE INTO prompt_cache_sessions (session_id)
VALUES (?);
```

### Lookup completion

**File-backed:** `read_json(completions/<hash>.json)`, check fingerprint
version, check TTL, delete file if expired.

**Dolt:**
```sql
SELECT response_json
  FROM prompt_cache_entries
 WHERE session_id = ?
   AND request_hash = ?
   AND fingerprint_version = ?
   AND cached_at_unix_secs >= ?;  -- now_unix_secs - ttl_secs
```

On miss or expired:
```sql
DELETE FROM prompt_cache_entries
 WHERE session_id = ?
   AND request_hash = ?
   AND (fingerprint_version != ? OR cached_at_unix_secs < ?);

UPDATE prompt_cache_sessions
   SET completion_cache_misses = completion_cache_misses + 1,
       last_completion_cache_key = ?
 WHERE session_id = ?;
```

On hit:
```sql
UPDATE prompt_cache_sessions
   SET completion_cache_hits = completion_cache_hits + 1,
       last_completion_cache_key = ?,
       last_cache_creation_input_tokens = ?,
       last_cache_read_input_tokens = ?,
       last_request_hash = ?,
       last_cache_source = 'completion-cache',
       total_cache_creation_input_tokens
           = total_cache_creation_input_tokens + ?,
       total_cache_read_input_tokens
           = total_cache_read_input_tokens + ?,
       prev_observed_at_unix_secs = ?,
       prev_fingerprint_version = ?,
       prev_model_hash = ?,
       prev_system_hash = ?,
       prev_tools_hash = ?,
       prev_messages_hash = ?,
       prev_cache_read_input_tokens = ?
 WHERE session_id = ?;
```

### Record response (write to cache)

**File-backed:** `write_completion_entry()` + `persist_state()`

**Dolt:**
```sql
REPLACE INTO prompt_cache_entries
    (session_id, request_hash, cached_at_unix_secs,
     fingerprint_version, response_json)
VALUES (?, ?, ?, ?, ?);

UPDATE prompt_cache_sessions
   SET tracked_requests = tracked_requests + 1,
       completion_cache_writes = completion_cache_writes + 1,
       -- stats fields ...
       -- prev_ fields ...
       -- break fields if detected ...
 WHERE session_id = ?;
```

### Record usage (stats only, no response caching)

**File-backed:** `record_usage_internal()` with `response = None`

**Dolt:** Same `UPDATE prompt_cache_sessions` as above, without the
`INSERT INTO prompt_cache_entries`.

### Expire stale entries

**File-backed:** Entries are deleted one at a time on read when TTL is
exceeded.

**Dolt:** Can be batched:
```sql
DELETE FROM prompt_cache_entries
 WHERE cached_at_unix_secs < ?;  -- now_unix_secs - max_ttl
```

This can run periodically or on session init. The `SELECT` in lookup already
filters by timestamp, so stale rows are invisible even before reaping.

### Read stats

**File-backed:** `PromptCache::stats()` returns in-memory clone.

**Dolt:**
```sql
SELECT * FROM prompt_cache_sessions WHERE session_id = ?;
```

---

## Improvements Over File-Backed Cache

1. **No directory sprawl** — eliminates the `completions/<hash>.json`
   one-file-per-entry pattern. Thousands of tiny JSON files become rows in a
   single table.

2. **Batch expiry** — a single `DELETE WHERE` replaces per-read file deletion.
   Can run periodically rather than on every cache miss.

3. **Atomic stats updates** — `UPDATE ... SET x = x + 1` is atomic. The
   file-backed version reads the full stats file, modifies in memory, and
   rewrites — racy under concurrent access (mitigated today by the in-process
   `Mutex`, but not across processes).

4. **Cross-session analytics** — queries like "total cache hit rate across all
   sessions" or "which sessions have the most unexpected breaks" become
   trivial SQL.

5. **No filesystem path sanitization** — the `sanitize_path_segment()`
   function and `MAX_SANITIZED_LENGTH` logic become unnecessary. Session IDs
   are just column values.

---

## Source Reference

| Current code | Role |
|---|---|
| `api/src/prompt_cache.rs` | `PromptCache` struct, all file I/O, TTL logic, fingerprinting, break detection |
| `api/src/providers/anthropic.rs` | Owns `Option<PromptCache>`, calls `lookup_completion` / `record_response` / `record_usage` |
| `api/src/client.rs` | `ProviderClient` enum dispatch for `with_prompt_cache`, `prompt_cache_stats`, `take_last_prompt_cache_record` |
| `rusty-claude-cli/src/main.rs:6323` | Constructs `PromptCache::new(session_id)` and passes to client |
| `tools/src/lib.rs:4041-4055` | Converts `PromptCacheRecord` → `PromptCacheEvent` (duplicated in CLI) |
