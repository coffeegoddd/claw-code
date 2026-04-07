# Configuration Store Schema Design: Dolt Migration

> Proposed Dolt schema to replace the file-backed configuration loader.
> See `SOTW_PERSISTENCE.md` for the full persistence catalog.

## Current Implementation

### Read-Only Multi-Level Merge

`ConfigLoader` in `runtime/src/config.rs` discovers up to 5 config files,
reads them with `fs::read_to_string`, validates each, then deep-merges them
in precedence order. The merged result is parsed into typed feature configs.

**The config system never writes files.** All `fs::write` calls in `config.rs`
are test-only. Config files are authored by humans or other tools.

### Precedence Order (lowest to highest)

| Scope | Path | Override behavior |
|-------|------|-------------------|
| User (legacy) | `~/.claw.json` | Base layer |
| User | `~/.claw/settings.json` | Overrides legacy |
| Project (legacy) | `<cwd>/.claw.json` | Overrides user |
| Project | `<cwd>/.claw/settings.json` | Overrides project legacy |
| Local | `<cwd>/.claw/settings.local.json` | Overrides everything |

### Merge Semantics

- **Nested objects:** recursive deep merge (both levels' keys are preserved,
  conflicts resolved by higher precedence)
- **Primitives and arrays:** higher precedence overwrites entirely
- **MCP servers:** per-server-name overwrite with scope tracking
- **Hooks:** accumulate and deduplicate (append, not overwrite)

### Config Sections

The merged JSON contains these top-level keys:

`$schema`, `enabledPlugins`, `env`, `hooks`, `mcpServers`, `model`, `oauth`,
`permissionMode`, `permissions`, `plugins`, `sandbox`, `aliases`,
`providerFallbacks`, `trustedRoots`

### Interface

There is no trait abstraction. `ConfigLoader` is a concrete struct with
`discover()` and `load()` methods. `RuntimeConfig` holds the merged result.

---

## Guiding Decisions

1. **Store raw config layers, not the merged result** — Dolt stores each
   config entry (user, project, local) as a separate row. The application
   merges them at load time, exactly as today. This preserves the layering
   semantics and avoids encoding merge logic in SQL.

2. **Config values as JSON blobs** — each config entry is a full JSON object.
   Storing individual keys as rows would lose the nested structure and make
   deep merge harder to reason about. A single JSON column per layer is
   simpler and matches the current file-per-layer model.

3. **Scope as an enum column** — `User`, `Project`, `Local` map directly to
   `ConfigSource`. Precedence is determined by the enum ordering, not by
   row position.

4. **Workspace scoping for project/local configs** — user configs are global,
   but project and local configs are per-workspace. A
   `workspace_fingerprint` column (nullable for user scope) provides
   isolation.

5. **Config history via Dolt versioning** — `dolt diff` shows when and how
   config changed. Currently there is no history since the files are just
   overwritten by editors.

---

## Tables

### 1. `config_entries`

One row per config layer. Replaces the 5 config files.

```sql
CREATE TABLE config_entries (
    scope                  ENUM('user', 'project', 'local') NOT NULL,
    workspace_fingerprint  VARCHAR(16),
    config_json            JSON          NOT NULL,
    updated_at_ms          BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (scope, workspace_fingerprint),
    INDEX idx_workspace (workspace_fingerprint)
);
```

**Notes:**
- `workspace_fingerprint` is `NULL` for `user` scope (global config)
- `workspace_fingerprint` is set for `project` and `local` scopes
- `config_json` holds the full JSON object for that layer (same content
  as the corresponding file today)
- Legacy files (`~/.claw.json`, `<cwd>/.claw.json`) are folded into
  their modern equivalents (`user`, `project`) at migration time

---

## Operation Mapping

### Discover config entries

**File-backed:** `ConfigLoader::discover()` returns 5 `ConfigEntry` paths.

**Dolt:**
```sql
SELECT scope, config_json
  FROM config_entries
 WHERE workspace_fingerprint IS NULL
    OR workspace_fingerprint = ?
 ORDER BY FIELD(scope, 'user', 'project', 'local');
```

Returns up to 3 rows (user, project, local) in precedence order.

### Load and merge config

**File-backed:** `ConfigLoader::load()` reads each file, validates, deep-merges.

**Dolt:** Application code does the same merge, just reading from query
results instead of files:

```
1. SELECT rows ordered by precedence
2. For each row: validate config_json, then deep_merge_objects()
3. Parse feature configs from merged result
4. Return RuntimeConfig
```

The `deep_merge_objects()`, `merge_mcp_servers()`, hook accumulation, and
all feature parsers remain unchanged in application code.

### Read single config layer

**File-backed:** `fs::read_to_string(path)` for one file.

**Dolt:**
```sql
SELECT config_json
  FROM config_entries
 WHERE scope = ?
   AND (workspace_fingerprint IS NULL OR workspace_fingerprint = ?);
```

### Write config layer

**File-backed:** Not done by the application — config files are
human-authored.

**Dolt:** When external tools or the user wants to update config:
```sql
REPLACE INTO config_entries (scope, workspace_fingerprint, config_json, updated_at_ms)
VALUES (?, ?, ?, ?);
```

This enables programmatic config updates (e.g., a CLI `claw config set`
command) that weren't possible with the read-only file approach.

### Delete config layer

**File-backed:** Delete the file.

**Dolt:**
```sql
DELETE FROM config_entries
 WHERE scope = ?
   AND (workspace_fingerprint IS NULL OR workspace_fingerprint = ?);
```

### List all config for a workspace

**File-backed:** Scan 5 paths, check which exist.

**Dolt:**
```sql
SELECT scope, config_json, updated_at_ms
  FROM config_entries
 WHERE workspace_fingerprint IS NULL
    OR workspace_fingerprint = ?
 ORDER BY FIELD(scope, 'user', 'project', 'local');
```

### Query config across workspaces

**File-backed:** Impossible — would require scanning all project directories.

**Dolt:**
```sql
SELECT workspace_fingerprint, config_json
  FROM config_entries
 WHERE scope = 'project'
   AND JSON_EXTRACT(config_json, '$.model') IS NOT NULL;
```

---

## Improvements Over File-Backed Config

1. **Config history** — `dolt log` / `dolt diff` shows when config changed
   and what changed. Currently there is no history — files are just
   overwritten.

2. **Cross-workspace queries** — "which projects override the model?" or
   "which projects have custom MCP servers?" become SQL queries.

3. **Programmatic updates** — the current system is read-only from the
   application's perspective. Dolt enables a `claw config set` command that
   writes structured config without hand-editing JSON.

4. **No legacy file handling** — the two legacy paths (`~/.claw.json`,
   `<cwd>/.claw.json`) are folded into their modern equivalents at migration
   time. The `is_legacy_config` special cases in `read_optional_json_object`
   become unnecessary.

5. **Consistent validation** — config can be validated on write (at INSERT
   time) rather than only on read. Bad config never enters the store.

6. **Local config stays local** — `local` scope rows can be excluded from
   Dolt remote sync via branch policies, preserving the gitignore semantics
   of `settings.local.json`.

---

## What Stays in Application Code

The entire merge pipeline stays unchanged:

- `deep_merge_objects()` — recursive JSON merge
- `merge_mcp_servers()` — per-server-name overwrite with scope tracking
- `RuntimeHookConfig::extend()` — hook accumulation with deduplication
- All `parse_optional_*` functions — feature config extraction
- `config_validate::validate_config_file()` — per-layer validation

The only change is the data source: rows from a Dolt query instead of files
from `fs::read_to_string`.

---

## Source Reference

| Current code | Role |
|---|---|
| `runtime/src/config.rs` (lines 57-62) | `RuntimeConfig` struct |
| `runtime/src/config.rs` (lines 235-239) | `ConfigLoader` struct |
| `runtime/src/config.rs` (lines 34-39) | `ConfigSource` enum |
| `runtime/src/config.rs` (lines 49-54) | `ConfigEntry` struct |
| `runtime/src/config.rs` (lines 263-290) | `discover()` — file discovery |
| `runtime/src/config.rs` (lines 292-346) | `load()` — load and merge logic |
| `runtime/src/config.rs` (lines 695-728) | `read_optional_json_object()` — file I/O |
| `runtime/src/config.rs` (lines 1235-1249) | `deep_merge_objects()` — merge strategy |
| `runtime/src/config.rs` (lines 730-755) | `merge_mcp_servers()` — MCP merge |
| `runtime/src/config.rs` (lines 613-626) | `RuntimeHookConfig::extend()` — hook merge |
| `runtime/src/config.rs` (lines 581-586) | `default_config_home()` |
