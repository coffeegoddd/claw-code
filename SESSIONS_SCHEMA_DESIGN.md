# Sessions Schema Design: Dolt Migration

> Proposed Dolt schema to replace the file-backed `SessionStore` and `Session`
> persistence layer. See `SOTW_PERSISTENCE.md` for the full persistence catalog.

## Guiding Decisions

1. **Content blocks stay denormalized as JSON** — they are always loaded and
   stored with their parent message, never queried independently. A separate
   `content_blocks` table would add a join on every load for no query benefit.

2. **Compaction and fork metadata live on `sessions`** — they are 0-1 per
   session and always loaded with session metadata. No reason for separate
   tables.

3. **Workspace isolation via Dolt branches** — replaces the filesystem
   fingerprint-directory scheme. Each workspace operates on its own
   `workspace/<fingerprint>` branch (see `DOLT_BRANCHING_STRATEGY.md`).
   No `workspace_fingerprint` column needed on workspace-scoped tables.

4. **Explicit `ordinal` on messages** — replaces implicit array-index /
   line-order in JSONL.

5. **File rotation and atomic writes become unnecessary** — Dolt commits give
   atomicity and `dolt diff` / `dolt log` give history for free.

6. **`updated_at_ms` on sessions is a maintained column** — updated on every
   message append, replacing the filesystem mtime that `list_sessions`
   currently reads.

---

## Tables

### 1. `sessions`

One row per session. Replaces the `session_meta` JSONL record, file existence,
`SessionFork` struct, and `SessionCompaction` struct.

```sql
CREATE TABLE sessions (
    session_id                    VARCHAR(128)    NOT NULL,
    workspace_root                TEXT,
    version                       INT UNSIGNED    NOT NULL DEFAULT 1,
    created_at_ms                 BIGINT UNSIGNED NOT NULL,
    updated_at_ms                 BIGINT UNSIGNED NOT NULL,

    -- Fork lineage (NULL when not a fork)
    fork_parent_session_id        VARCHAR(128),
    fork_branch_name              VARCHAR(256),

    -- Compaction state (NULL when never compacted)
    compaction_count              INT UNSIGNED,
    compaction_removed_msg_count  INT UNSIGNED,
    compaction_summary            LONGTEXT,

    PRIMARY KEY (session_id),
    INDEX idx_updated (updated_at_ms DESC),
    INDEX idx_fork_parent (fork_parent_session_id)
);
```

### 2. `messages`

One row per conversation turn. Replaces the `"message"` JSONL records.

```sql
CREATE TABLE messages (
    session_id                    VARCHAR(128)  NOT NULL,
    ordinal                       INT UNSIGNED  NOT NULL,
    role                          ENUM('system', 'user', 'assistant', 'tool') NOT NULL,
    blocks_json                   JSON          NOT NULL,

    -- Token usage (nullable; typically only present on assistant messages)
    input_tokens                  INT UNSIGNED,
    output_tokens                 INT UNSIGNED,
    cache_creation_input_tokens   INT UNSIGNED,
    cache_read_input_tokens       INT UNSIGNED,

    PRIMARY KEY (session_id, ordinal),
    CONSTRAINT fk_messages_session
        FOREIGN KEY (session_id) REFERENCES sessions(session_id)
        ON DELETE CASCADE
);
```

### 3. `prompt_history`

One row per user prompt entry. Replaces the `"prompt_history"` JSONL records.

```sql
CREATE TABLE prompt_history (
    session_id                    VARCHAR(128)    NOT NULL,
    timestamp_ms                  BIGINT UNSIGNED NOT NULL,
    text                          LONGTEXT        NOT NULL,

    PRIMARY KEY (session_id, timestamp_ms),
    CONSTRAINT fk_prompt_session
        FOREIGN KEY (session_id) REFERENCES sessions(session_id)
        ON DELETE CASCADE
);
```

---

## `blocks_json` Column Format

Same structure as the current `ContentBlock::to_json()` output, stored as a
JSON array. This preserves backward compatibility with the existing
serialization and avoids a `content_blocks` join table.

```json
[
  {
    "type": "text",
    "text": "hello world"
  },
  {
    "type": "tool_use",
    "id": "tu_1",
    "name": "Bash",
    "input": "{\"command\":\"ls\"}"
  },
  {
    "type": "tool_result",
    "tool_use_id": "tu_1",
    "tool_name": "Bash",
    "output": "file.txt",
    "is_error": false
  }
]
```

---

## Operation Mapping

How each current file-backed operation maps to Dolt queries.

### Create session

**File-backed:** `Session::new()` + `save_to_path()`

**Dolt:**
```sql
INSERT INTO sessions (
    session_id, workspace_root,
    version, created_at_ms, updated_at_ms
) VALUES (?, ?, 1, ?, ?);
```

### Append message

**File-backed:** `OpenOptions::append` one JSONL line

**Dolt:**
```sql
INSERT INTO messages (session_id, ordinal, role, blocks_json,
    input_tokens, output_tokens,
    cache_creation_input_tokens, cache_read_input_tokens)
VALUES (?, ?, ?, ?, ?, ?, ?, ?);

UPDATE sessions SET updated_at_ms = ? WHERE session_id = ?;
```

### Append prompt entry

**File-backed:** `OpenOptions::append` one JSONL line

**Dolt:**
```sql
INSERT INTO prompt_history (session_id, timestamp_ms, text)
VALUES (?, ?, ?);
```

### Load session

**File-backed:** `fs::read_to_string` + parse all JSONL lines

**Dolt:**
```sql
SELECT * FROM sessions WHERE session_id = ?;

SELECT ordinal, role, blocks_json,
       input_tokens, output_tokens,
       cache_creation_input_tokens, cache_read_input_tokens
  FROM messages
 WHERE session_id = ?
 ORDER BY ordinal;

SELECT timestamp_ms, text
  FROM prompt_history
 WHERE session_id = ?
 ORDER BY timestamp_ms;
```

### List sessions

**File-backed:** `fs::read_dir` + `Session::load_from_path` per file for
metadata extraction

**Dolt:**
```sql
SELECT s.session_id,
       s.updated_at_ms,
       s.fork_parent_session_id,
       s.fork_branch_name,
       (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.session_id)
           AS message_count
  FROM sessions s
 ORDER BY s.updated_at_ms DESC;
```

### Latest session

**File-backed:** Sort by mtime, take first

**Dolt:**
```sql
SELECT session_id FROM sessions
 ORDER BY updated_at_ms DESC
 LIMIT 1;
```

### Resolve by ID

**File-backed:** Check `{id}.jsonl` file exists

**Dolt:**
```sql
SELECT session_id FROM sessions WHERE session_id = ?;
```

### Fork session

**File-backed:** `session.fork()` (in-memory clone) + `save_to_path()`

**Dolt:**
```sql
INSERT INTO sessions (
    session_id, workspace_root,
    version, created_at_ms, updated_at_ms,
    fork_parent_session_id, fork_branch_name,
    compaction_count, compaction_removed_msg_count, compaction_summary
)
SELECT ?, workspace_root,
       version, ?, ?,
       ?, ?,
       compaction_count, compaction_removed_msg_count, compaction_summary
  FROM sessions
 WHERE session_id = ?;

INSERT INTO messages (session_id, ordinal, role, blocks_json,
    input_tokens, output_tokens,
    cache_creation_input_tokens, cache_read_input_tokens)
SELECT ?, ordinal, role, blocks_json,
       input_tokens, output_tokens,
       cache_creation_input_tokens, cache_read_input_tokens
  FROM messages
 WHERE session_id = ?;

INSERT INTO prompt_history (session_id, timestamp_ms, text)
SELECT ?, timestamp_ms, text
  FROM prompt_history
 WHERE session_id = ?;
```

### Record compaction

**File-backed:** `session.record_compaction()` + `save_to_path()`

**Dolt:**
```sql
UPDATE sessions
   SET compaction_count = ?,
       compaction_removed_msg_count = ?,
       compaction_summary = ?
 WHERE session_id = ?;

DELETE FROM messages
 WHERE session_id = ?
   AND ordinal < ?;
```

### Rotation / cleanup

**File-backed:** `rotate_session_file_if_needed()` +
`cleanup_rotated_logs()`

**Dolt:** **Eliminated.** Dolt versioning replaces file rotation entirely.
Previous states are recoverable via `dolt log` / `dolt diff`.

### Workspace isolation

**File-backed:** Directory `sessions/<fingerprint>/`

**Dolt:** Each workspace operates on its own `workspace/<fingerprint>` Dolt
branch. No column filter needed. See `DOLT_BRANCHING_STRATEGY.md`.

---

## Improvements Over File-Backed Store

1. **List is O(1) instead of O(n)** — the current `list_sessions` reads every
   session file to extract metadata. Dolt answers with a single indexed query.

2. **Append is a row insert** — no need to check if the file exists or is
   empty and bootstrap a full snapshot.

3. **No rotation or cleanup bookkeeping** — Dolt's commit log replaces the
   `*.rot-<timestamp>.jsonl` rotation scheme entirely.

4. **Fork is a server-side copy** — `INSERT ... SELECT` instead of loading the
   entire session into memory, cloning, and writing back out.

5. **Compaction can delete rows immediately** — currently compaction records
   metadata but old messages stay in the JSONL file until the next full
   snapshot. With Dolt, `DELETE FROM messages` is immediate, and old data is
   still recoverable via `dolt diff`.

6. **Cross-session queries become possible** — e.g., "find all sessions where
   tool X was used" or "total tokens across all sessions this week." Impossible
   with the current file format without loading every file.

---

## Source Reference

| Current code | Role |
|---|---|
| `runtime/src/session.rs` | `Session` struct, JSONL serialization, file I/O, rotation |
| `runtime/src/session_control.rs` | `SessionStore`, workspace fingerprinting, session listing/resolution |
| `runtime/src/usage.rs` | `TokenUsage` struct (4 x `u32`) |
| `runtime/src/compact.rs` | `compact_session()`, `CompactionConfig` |
| `rusty-claude-cli/src/main.rs` (lines ~4337-4468) | Duplicated session management free functions |
