# Agent Store Schema Design: Dolt Migration

> Proposed Dolt schema to replace the file-backed agent store
> (`.clawd-agents/` directory with per-agent `.json` manifests and `.md`
> output files). See `SOTW_PERSISTENCE.md` for the full persistence catalog.

## Current Implementation

### Agent Manifest

`execute_agent_with_spawn()` in `tools/src/lib.rs` creates an `AgentOutput`
struct serialized to `<store_dir>/<agent_id>.json`. The manifest tracks agent
identity, status, timestamps, lane events, blockers, and derived state.
Updated on completion by `persist_agent_terminal_state()` via
`write_agent_manifest()`.

- **Path:** `<cwd or 2-ancestors-up>/.clawd-agents/<agent_id>.json`
  (overridable via `$CLAWD_AGENT_STORE`)
- **Format:** Pretty-printed JSON with camelCase keys

### Agent Output

A markdown file at `<store_dir>/<agent_id>.md` is created with a header
section on spawn and progressively appended with result sections on
completion via `append_agent_output()`.

### Lane Events

Each agent tracks a `Vec<LaneEvent>` recording state transitions (started,
finished, blocked, failed, commit_created, etc.). Events carry timestamps,
optional failure classification, and optional data payloads (e.g., commit
provenance).

### Interface

There is no trait abstraction. All persistence is direct `std::fs::write` /
`OpenOptions::append` in three functions:
- `write_agent_manifest()` — full JSON rewrite of manifest
- `append_agent_output()` — append to markdown file
- `persist_agent_terminal_state()` — orchestrates both on completion

There are no runtime read paths — manifests are only read in tests.

---

## Guiding Decisions

1. **Agents as rows, lane events as a child table** — lane events are
   append-only and independently useful for querying ("find all agents that
   created commits", "which agents hit merge conflicts?").

2. **Output text as a column, not a file** — the `.md` content is a single
   append-only string. A `LONGTEXT` column is simpler than managing a
   separate file, and Dolt versioning replaces the append-only semantics.

3. **Manifest file and output file paths become unnecessary** — these are
   filesystem artifacts. In Dolt, agents are identified by `agent_id` and
   the files don't exist. Stored as nullable columns for migration
   compatibility.

4. **Commit provenance as JSON on lane events** — only `commit_created`
   events carry provenance data. A JSON column on the lane events table
   avoids a rarely-populated join table.

5. **Workspace isolation via Dolt branches** — agents are project-scoped.
   Each workspace operates on its own `workspace/<fingerprint>` branch
   (see `DOLT_BRANCHING_STRATEGY.md`). No `workspace_fingerprint` column
   needed.

6. **Timestamps as strings** — the current implementation stores timestamps
   as ISO8601 strings (seconds since epoch). The schema preserves this for
   compatibility but also adds a `BIGINT UNSIGNED` `created_at_ms` on the
   events table for efficient range queries.

---

## Tables

### 1. `agents`

One row per agent. Replaces the `<agent_id>.json` manifest files.

```sql
CREATE TABLE agents (
    agent_id               VARCHAR(128)    NOT NULL,
    name                   VARCHAR(256)    NOT NULL,
    description            TEXT            NOT NULL,
    subagent_type          VARCHAR(64),
    model                  VARCHAR(128),
    status                 VARCHAR(32)     NOT NULL,
    output                 LONGTEXT        NOT NULL DEFAULT '',
    created_at             VARCHAR(64)     NOT NULL,
    started_at             VARCHAR(64),
    completed_at           VARCHAR(64),
    derived_state          VARCHAR(64)     NOT NULL,
    error                  TEXT,
    current_blocker_json   JSON,

    PRIMARY KEY (agent_id),
    INDEX idx_status (status),
    INDEX idx_derived_state (derived_state)
);
```

### 2. `agent_lane_events`

One row per lane event. Replaces the `lane_events` array in the manifest JSON.

```sql
CREATE TABLE agent_lane_events (
    agent_id               VARCHAR(128)    NOT NULL,
    ordinal                INT UNSIGNED    NOT NULL,
    event                  VARCHAR(64)     NOT NULL,
    status                 VARCHAR(32)     NOT NULL,
    emitted_at             VARCHAR(64)     NOT NULL,
    failure_class          VARCHAR(64),
    detail                 TEXT,
    data_json              JSON,

    PRIMARY KEY (agent_id, ordinal),
    INDEX idx_event_type (event),
    INDEX idx_failure_class (failure_class),
    CONSTRAINT fk_lane_events_agent
        FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        ON DELETE CASCADE
);
```

---

## `current_blocker_json` Column Format

```json
{
  "failureClass": "merge_conflict",
  "detail": "branch diverged from main after rebase"
}
```

`NULL` when agent is not blocked.

## `data_json` Column Format (on lane events)

Only populated for `lane.commit.created` events:

```json
{
  "commit": "a1b2c3d",
  "branch": "feature/foo",
  "worktree": "/path/to/worktree",
  "canonicalCommit": "a1b2c3d4e5f6",
  "supersededBy": null,
  "lineage": []
}
```

`NULL` for all other event types.

---

## Operation Mapping

### Create agent

**File-backed:** `execute_agent_with_spawn()` creates `.md` output file and
`.json` manifest.

**Dolt:**
```sql
INSERT INTO agents (
    agent_id, name, description,
    subagent_type, model, status, output,
    created_at, started_at, derived_state
) VALUES (?, ?, ?, ?, ?, ?, 'running', ?, ?, ?, 'working');

INSERT INTO agent_lane_events (
    agent_id, ordinal, event, status, emitted_at
) VALUES (?, 0, 'lane.started', 'running', ?);
```

The initial output (markdown header + prompt) is stored in the `output`
column instead of a separate `.md` file.

### Update manifest on completion

**File-backed:** `persist_agent_terminal_state()` appends to `.md` and
rewrites `.json`.

**Dolt (success):**
```sql
UPDATE agents
   SET status = 'completed',
       completed_at = ?,
       derived_state = ?,
       output = CONCAT(output, ?),
       current_blocker_json = NULL
 WHERE agent_id = ?;

INSERT INTO agent_lane_events (agent_id, ordinal, event, status, emitted_at, detail)
VALUES (?, ?, 'lane.finished', 'completed', ?, ?);

-- If commit provenance detected:
INSERT INTO agent_lane_events (agent_id, ordinal, event, status, emitted_at, detail, data_json)
VALUES (?, ?, 'lane.commit.created', 'completed', ?, ?, ?);
```

**Dolt (failure):**
```sql
UPDATE agents
   SET status = 'failed',
       completed_at = ?,
       derived_state = ?,
       error = ?,
       output = CONCAT(output, ?),
       current_blocker_json = ?
 WHERE agent_id = ?;

INSERT INTO agent_lane_events (agent_id, ordinal, event, status, emitted_at, failure_class, detail)
VALUES (?, ?, 'lane.blocked', 'blocked', ?, ?, ?);

INSERT INTO agent_lane_events (agent_id, ordinal, event, status, emitted_at, failure_class, detail)
VALUES (?, ?, 'lane.failed', 'failed', ?, ?, ?);
```

### Append output

**File-backed:** `append_agent_output()` opens file in append mode.

**Dolt:**
```sql
UPDATE agents
   SET output = CONCAT(output, ?)
 WHERE agent_id = ?;
```

### Read agent manifest

**File-backed:** `fs::read_to_string` + `serde_json::from_str` (tests only).

**Dolt:**
```sql
SELECT * FROM agents WHERE agent_id = ?;

SELECT * FROM agent_lane_events
 WHERE agent_id = ?
 ORDER BY ordinal;
```

### List agents in workspace

**File-backed:** Not implemented (no runtime discovery from disk).

**Dolt:**
```sql
SELECT agent_id, name, status, derived_state, created_at, completed_at
  FROM agents
 ORDER BY created_at DESC;
```

### Query agents by failure class

**File-backed:** Impossible without loading every manifest.

**Dolt:**
```sql
SELECT a.agent_id, a.name, e.detail
  FROM agents a
  JOIN agent_lane_events e ON a.agent_id = e.agent_id
 WHERE e.failure_class = 'merge_conflict';
```

### Query commit provenance

**File-backed:** Impossible.

**Dolt:**
```sql
SELECT a.agent_id, a.name, e.data_json
  FROM agents a
  JOIN agent_lane_events e ON a.agent_id = e.agent_id
 WHERE e.event = 'lane.commit.created';
```

---

## Improvements Over File-Backed Store

1. **Agent discovery becomes a query** — currently there is no runtime
   discovery of agents from disk. Dolt enables listing, filtering, and
   searching agents by status, failure class, or derived state.

2. **Lane event analytics** — "how many agents hit merge conflicts this
   week?" or "which agents created commits?" are trivial queries against
   `agent_lane_events`.

3. **No two-file coordination** — currently each agent has a `.json` manifest
   and a `.md` output file that must be kept in sync. Dolt stores both as
   columns in one row.

4. **Commit provenance tracking** — commit SHAs extracted from agent output
   become queryable structured data instead of buried in markdown text.

5. **History via Dolt versioning** — `dolt diff` shows how agent state
   evolved over time. Currently the manifest is overwritten and the old state
   is lost.

6. **No filesystem path management** — `agent_store_dir()`, path sanitization,
   and the 2-ancestor workspace detection become unnecessary for metadata.

---

## Source Reference

| Current code | Role |
|---|---|
| `tools/src/lib.rs` (lines 2357-2385) | `AgentOutput` struct (the manifest) |
| `tools/src/lib.rs` (lines 3252-3334) | `execute_agent_with_spawn()` — agent creation |
| `tools/src/lib.rs` (lines 3505-3513) | `write_agent_manifest()` — persist manifest JSON |
| `tools/src/lib.rs` (lines 3515-3557) | `persist_agent_terminal_state()` — completion update |
| `tools/src/lib.rs` (lines 3634-3643) | `append_agent_output()` — append to output file |
| `tools/src/lib.rs` (lines 4204-4213) | `agent_store_dir()` — store path resolution |
| `tools/src/lib.rs` (lines 4215-4221) | `make_agent_id()` — ID generation |
| `runtime/src/lane_events.rs` | `LaneEvent`, `LaneEventBlocker`, `LaneCommitProvenance`, enums |
