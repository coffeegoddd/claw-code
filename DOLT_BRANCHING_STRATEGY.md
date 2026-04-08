# Dolt Branching Strategy

> How workspace isolation and global vs. per-workspace data are organized
> across Dolt branches. This is internal to the Dolt backend implementations
> and never exposed to end users.

## Branch Layout

```
main                                              ← global tables
├── workspace/<fingerprint>                       ← workspace root branch
│   ├── workspace/<fingerprint>/agent-<id>        ← sub-branch (agent worktree)
│   └── workspace/<fingerprint>/<purpose>         ← sub-branch (experiments, etc.)
├── workspace/<fingerprint-2>
│   └── ...
└── ...
```

- **`main`** — holds global tables (plugins, prompt cache, user config).
  Always exists. Never deleted.
- **`workspace/<fingerprint>`** — one per workspace, forked from `main` on
  first use. Holds workspace-scoped tables (sessions, todos, tasks, agents,
  project/local config). The `<fingerprint>` is the FNV-1a hex of the
  canonical workspace root path.
- **`workspace/<fingerprint>/<purpose>`** — optional sub-branches for
  transient work (agent worktrees, config experiments). Cleaned up when done.

The `workspace/` prefix groups all workspace branches and allows sub-branches
using `/` as a hierarchical delimiter. Dolt branch names support `/` natively.

## Table Classification

### Global tables (on `main`)

| Table | Store |
|-------|-------|
| `plugins` | Plugin registry |
| `prompt_cache_sessions` | Prompt cache |
| `prompt_cache_entries` | Prompt cache |
| `config_entries` (scope = 'user') | Configuration |

### Workspace tables (on `workspace/<fingerprint>`)

| Table | Store |
|-------|-------|
| `sessions` | Session store |
| `messages` | Session store |
| `prompt_history` | Session store |
| `todos` | Todo store |
| `tasks` | Task registry |
| `task_messages` | Task registry |
| `agents` | Agent store |
| `agent_lane_events` | Agent store |
| `config_entries` (scope = 'project'/'local') | Configuration |

## Access Patterns

### Workspace-scoped reads

The Dolt backend checks out the workspace branch and queries directly:

```sql
-- Implicit: on branch workspace/<fingerprint>
SELECT * FROM sessions ORDER BY updated_at_ms DESC;
```

No `WHERE workspace_fingerprint = ?` needed — the branch provides isolation.

### Global reads from a workspace branch

Global data is read via cross-branch reference:

```sql
-- From any workspace branch:
SELECT * FROM `main`.plugins WHERE enabled = TRUE;
SELECT * FROM `main`.prompt_cache_entries WHERE session_id = ?;
```

### Global writes

Switch to `main` for global operations:

```sql
-- On main branch:
INSERT INTO plugins (...) VALUES (...);
REPLACE INTO prompt_cache_entries (...) VALUES (...);
```

### Cross-workspace analytics

Query across workspace branches when needed:

```sql
SELECT b.name, COUNT(*) AS session_count
  FROM dolt_branches b
  -- cross-branch joins via Dolt's branch-qualified table names
```

## Branch Lifecycle

### Creation

On first use of a workspace, the Dolt backend creates the branch:

```
dolt branch workspace/<fingerprint> main
```

Tables are inherited from `main` (workspace tables exist but are empty,
global tables carry their data).

### Checkout

Each Dolt backend instance holds a connection scoped to a specific branch.
The branch name is derived from the workspace fingerprint at construction
time. No user interaction required.

### Sub-branches

Created as needed (e.g., for agent worktrees):

```
dolt branch workspace/<fingerprint>/agent-<id> workspace/<fingerprint>
```

Merged or deleted when the sub-branch work completes.

### Cleanup

Stale workspace branches can be garbage-collected based on last commit
timestamp. Sub-branches are cleaned up by their creating subsystem.

## Implementation Notes

- Branch management is **internal to the Dolt backend implementations**.
  Trait interfaces have no branch or fingerprint parameters — callers are
  unaware of the branching.
- The `FileSessionBackend`, `FileConfigStore`, etc. continue to use
  filesystem directory isolation. The branching strategy is a Dolt-specific
  concern.
- The workspace fingerprint uses the same FNV-1a hash as the existing
  `session_control::workspace_fingerprint()` function.
