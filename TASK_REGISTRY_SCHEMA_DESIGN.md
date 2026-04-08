# Task Registry Schema Design: Dolt Migration

> Proposed Dolt schema to replace the file-backed todo store
> (`.clawd-todos.json`) and persist the currently in-memory `TaskRegistry`.
> See `SOTW_PERSISTENCE.md` for the full persistence catalog.

## Current Implementation

### Todo Store

`execute_todo_write()` in `tools/src/lib.rs` is the single function that
handles all todo persistence. It reads the existing JSON array, validates the
new state, and atomically overwrites the file.

- **Path:** `<cwd>/.clawd-todos.json` (overridable via `$CLAWD_TODO_STORE`)
- **Format:** JSON array of `TodoItem { content, active_form, status }`
- **Status values:** `pending`, `in_progress`, `completed`
- **Behavior:** Auto-clears to `[]` when all todos reach `completed`
- **Interface:** None — single function, direct `std::fs` I/O

### Task Registry

`TaskRegistry` in `runtime/src/task_registry.rs` is an in-memory
`HashMap<String, Task>` behind `Arc<Mutex>`. It is initialized as a global
singleton and **never persisted to disk** — all tasks are lost on session exit.

- **Struct:** `Task { task_id, prompt, description, task_packet, status, created_at, updated_at, messages, output, team_id }`
- **Status values:** `created`, `running`, `completed`, `failed`, `stopped`
- **Messages:** `Vec<TaskMessage { role, content, timestamp }>`
- **TaskPacket:** Optional structured work spec with objective, scope, repo, policies
- **Interface:** `TaskRegistry` struct with CRUD methods, no trait abstraction

Neither system has a trait abstraction or swappable backend.

---

## Guiding Decisions

1. **Keep todos and tasks as separate tables** — they are genuinely different
   things. Todos are a user-facing checklist the LLM maintains. Tasks are
   programmatic sub-agent work items. They have different lifecycles, different
   status enums, and different consumers.

2. **Persist the TaskRegistry** — currently in-memory and lost on exit. Dolt
   makes it durable, enabling cross-session task visibility ("what did my
   agents work on yesterday?").

3. **Task messages as a child table** — they are append-only and queryable by
   timestamp, making them a natural fit for rows rather than a JSON blob.

4. **Task output as a TEXT column** — it is a single append-only string per
   task. No need for a separate table.

5. **TaskPacket as JSON** — it is a structured blob always loaded and saved as
   a unit, never queried field-by-field. A JSON column avoids a 1:1 join table
   with 8 columns.

6. **Workspace isolation via Dolt branches** — todos and tasks are
   project-scoped. Each workspace operates on its own
   `workspace/<fingerprint>` branch (see `DOLT_BRANCHING_STRATEGY.md`).
   No `workspace_fingerprint` column needed.

---

## Tables

### 1. `todos`

One row per todo item. Replaces `.clawd-todos.json`.

```sql
CREATE TABLE todos (
    ordinal                INT UNSIGNED    NOT NULL,
    content                TEXT            NOT NULL,
    active_form            TEXT            NOT NULL,
    status                 ENUM('pending', 'in_progress', 'completed') NOT NULL,

    PRIMARY KEY (ordinal)
);
```

### 2. `tasks`

One row per task. Persists the in-memory `TaskRegistry`.

```sql
CREATE TABLE tasks (
    task_id                VARCHAR(128)    NOT NULL,
    prompt                 LONGTEXT        NOT NULL,
    description            TEXT,
    task_packet_json       JSON,
    status                 ENUM('created', 'running', 'completed',
                                'failed', 'stopped')          NOT NULL,
    output                 LONGTEXT        NOT NULL DEFAULT '',
    team_id                VARCHAR(128),
    created_at             BIGINT UNSIGNED NOT NULL,
    updated_at             BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (task_id),
    INDEX idx_status (status),
    INDEX idx_team (team_id)
);
```

### 3. `task_messages`

One row per message within a task. Append-only.

```sql
CREATE TABLE task_messages (
    task_id                VARCHAR(128)    NOT NULL,
    ordinal                INT UNSIGNED    NOT NULL,
    role                   VARCHAR(32)     NOT NULL,
    content                LONGTEXT        NOT NULL,
    timestamp              BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (task_id, ordinal),
    CONSTRAINT fk_task_messages
        FOREIGN KEY (task_id) REFERENCES tasks(task_id)
        ON DELETE CASCADE
);
```

---

## `task_packet_json` Column Format

Same structure as the `TaskPacket` Rust struct serialization:

```json
{
  "objective": "Ship task packet support",
  "scope": "runtime/task system",
  "repo": "claw-code-parity",
  "branch_policy": "origin/main only",
  "acceptance_tests": ["cargo test --workspace"],
  "commit_policy": "single commit",
  "reporting_contract": "print commit sha",
  "escalation_policy": "manual escalation"
}
```

---

## Operation Mapping: Todos

### Read todos

**File-backed:** `fs::read_to_string(".clawd-todos.json")` then parse JSON
array.

**Dolt:**
```sql
SELECT ordinal, content, active_form, status
  FROM todos
 ORDER BY ordinal;
```

### Write todos (full replacement)

**File-backed:** `fs::write(".clawd-todos.json", serde_json::to_string_pretty(&todos))`

**Dolt:**
```sql
DELETE FROM todos;

INSERT INTO todos (ordinal, content, active_form, status)
VALUES (0, ?, ?, ?),
       (1, ?, ?, ?),
       ...;
```

### Auto-clear on all completed

**File-backed:** If all todos have status `completed`, write `[]` to the file.

**Dolt:**
```sql
DELETE FROM todos
 WHERE NOT EXISTS (
       SELECT 1 FROM todos t2
        WHERE t2.status != 'completed'
   );
```

### Workspace isolation

**File-backed:** `$CLAWD_TODO_STORE` env var or `<cwd>/.clawd-todos.json`.

**Dolt:** Each workspace operates on its own Dolt branch. Env var override
becomes unnecessary.

---

## Operation Mapping: Tasks

### Create task

**In-memory:** `TaskRegistry::create(prompt, description)`

**Dolt:**
```sql
INSERT INTO tasks (
    task_id, prompt, description,
    task_packet_json, status, created_at, updated_at
) VALUES (?, ?, ?, ?, 'created', ?, ?);
```

### Create task from packet

**In-memory:** `TaskRegistry::create_from_packet(packet)`

**Dolt:**
```sql
INSERT INTO tasks (
    task_id, prompt, description,
    task_packet_json, status, created_at, updated_at
) VALUES (?, ?, ?, ?, 'created', ?, ?);
```

Where `prompt` = `packet.objective`, `description` = `packet.scope`, and
`task_packet_json` = serialized `TaskPacket`.

### Get task

**In-memory:** `TaskRegistry::get(task_id)`

**Dolt:**
```sql
SELECT * FROM tasks WHERE task_id = ?;

SELECT ordinal, role, content, timestamp
  FROM task_messages
 WHERE task_id = ?
 ORDER BY ordinal;
```

### List tasks

**In-memory:** `TaskRegistry::list(status_filter)`

**Dolt:**
```sql
SELECT * FROM tasks
   [WHERE status = ?]
 ORDER BY created_at DESC;
```

### Set status

**In-memory:** `TaskRegistry::set_status(task_id, status)`

**Dolt:**
```sql
UPDATE tasks
   SET status = ?, updated_at = ?
 WHERE task_id = ?;
```

### Update (append message)

**In-memory:** `TaskRegistry::update(task_id, message)`

**Dolt:**
```sql
INSERT INTO task_messages (task_id, ordinal, role, content, timestamp)
VALUES (?,
        (SELECT COALESCE(MAX(ordinal) + 1, 0)
           FROM task_messages WHERE task_id = ?),
        'user', ?, ?);

UPDATE tasks SET updated_at = ? WHERE task_id = ?;
```

### Append output

**In-memory:** `TaskRegistry::append_output(task_id, text)`

**Dolt:**
```sql
UPDATE tasks
   SET output = CONCAT(output, ?),
       updated_at = ?
 WHERE task_id = ?;
```

### Assign team

**In-memory:** `TaskRegistry::assign_team(task_id, team_id)`

**Dolt:**
```sql
UPDATE tasks
   SET team_id = ?, updated_at = ?
 WHERE task_id = ?;
```

### Stop task

**In-memory:** `TaskRegistry::stop(task_id)` — rejects terminal states.

**Dolt:**
```sql
UPDATE tasks
   SET status = 'stopped', updated_at = ?
 WHERE task_id = ?
   AND status NOT IN ('completed', 'failed', 'stopped');
```

Returns an error if zero rows affected (task missing or already terminal).

### Remove task

**In-memory:** `TaskRegistry::remove(task_id)`

**Dolt:**
```sql
DELETE FROM tasks WHERE task_id = ?;
```

Cascades to `task_messages`.

### Get output

**In-memory:** `TaskRegistry::output(task_id)`

**Dolt:**
```sql
SELECT output FROM tasks WHERE task_id = ?;
```

---

## Improvements Over Current Implementation

1. **Tasks survive sessions** — the biggest win. Currently tasks vanish when
   the CLI exits. With Dolt, "show me all failed tasks from today" becomes a
   query.

2. **Todo history via Dolt versioning** — currently the file is overwritten
   atomically with no history. `dolt diff` shows what changed between writes.

3. **Cross-project queries** — "which projects have open todos?" is a single
   query across workspaces instead of scanning filesystem directories.

4. **No file cleanup logic** — the auto-clear-on-all-completed behavior
   becomes a `DELETE WHERE` instead of reading, parsing, and rewriting a JSON
   file.

5. **Task messages are queryable** — "find all tasks where I sent a message
   containing X" is impossible with the in-memory `HashMap`, trivial with SQL.

6. **Atomic status transitions** — the `WHERE status NOT IN (...)` guard on
   stop/update is enforced at the database level, eliminating race conditions
   that the `Mutex` currently prevents only within a single process.

---

## Source Reference

| Current code | Role |
|---|---|
| `tools/src/lib.rs` (lines 2067-2081) | `TodoItem`, `TodoStatus` structs |
| `tools/src/lib.rs` (lines 2905-2949) | `execute_todo_write()` — all todo persistence |
| `tools/src/lib.rs` (lines 2979-2985) | `todo_store_path()` — path resolution |
| `runtime/src/task_registry.rs` | `TaskRegistry`, `Task`, `TaskStatus`, `TaskMessage` |
| `runtime/src/task_packet.rs` | `TaskPacket`, `ValidatedPacket` |
