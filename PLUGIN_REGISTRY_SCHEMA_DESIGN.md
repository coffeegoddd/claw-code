# Plugin Registry Schema Design: Dolt Migration

> Proposed Dolt schema to replace the file-backed plugin registry
> (`installed.json`) and enabled-state persistence (`settings.json`).
> See `SOTW_PERSISTENCE.md` for the full persistence catalog.

## Current Implementation

### Registry File

`PluginManager` in `plugins/src/lib.rs` owns all plugin metadata persistence.
The install registry is a `BTreeMap<String, InstalledPluginRecord>` serialized
to `~/.claw/plugins/installed.json`. It is read via `load_registry()` and
written via `store_registry()`.

- **Path:** `~/.claw/plugins/installed.json` (overridable via
  `plugins.registryPath` in settings.json)
- **Format:** JSON object with a `plugins` key mapping plugin IDs to records
- **Modified by:** `install()`, `uninstall()`, `update()`,
  `sync_bundled_plugins()`, `discover_installed_plugins_with_failures()`
  (stale cleanup)

### Enabled State

Plugin enable/disable state lives in `settings.json` under `enabledPlugins`
(a `BTreeMap<String, bool>`). Modified by `enable()`, `disable()`,
`install()`, `uninstall()` via `write_enabled_state()` →
`update_settings_json()`.

- **Path:** `~/.claw/settings.json`
- **Format:** JSON object; plugin state under `enabledPlugins` key

### Plugin Artifacts

Actual plugin directories live under `~/.claw/plugins/installed/<id>/`.
These contain plugin code, manifests, and hooks — they are filesystem
artifacts, not structured metadata. **Artifact management stays on the
filesystem** — Dolt replaces only the metadata layer.

### Interface

There is no trait abstraction for the registry store. All persistence is
direct `std::fs` calls inside `PluginManager` methods. There is a `Plugin`
trait for plugin behavior (metadata, hooks, tools) but not for storage.

There are no ad-hoc bypasses — all `installed.json` access flows through
`load_registry()` / `store_registry()`, and all `settings.json` access flows
through `update_settings_json()`.

---

## Guiding Decisions

1. **Merge registry and enabled state into one table** — currently split
   across two files (`installed.json` and `settings.json`). The enabled flag
   is always loaded alongside the registry. One table eliminates the two-file
   coordination.

2. **Plugin artifacts stay on the filesystem** — Dolt replaces the metadata
   (which plugin is installed, its version, source, enabled state) but not the
   actual plugin directories containing code and manifests.

3. **Install source as JSON** — `PluginInstallSource` is a tagged enum
   (`local_path` or `git_url`). A JSON column preserves the tagged structure
   without needing two nullable columns.

4. **No workspace scoping** — plugins are global (per user home), not
   project-scoped. No `workspace_fingerprint` column needed.

5. **Timestamps as milliseconds** — matching the existing `u128`
   `installed_at_unix_ms` and `updated_at_unix_ms` fields. Stored as
   `BIGINT UNSIGNED` (u64 range is sufficient for millisecond timestamps
   until the year 584,942,417).

---

## Tables

### 1. `plugins`

One row per installed plugin. Replaces `installed.json` and the
`enabledPlugins` section of `settings.json`.

```sql
CREATE TABLE plugins (
    plugin_id              VARCHAR(256)    NOT NULL,
    kind                   ENUM('builtin', 'bundled', 'external') NOT NULL,
    name                   VARCHAR(256)    NOT NULL,
    version                VARCHAR(128)    NOT NULL,
    description            TEXT            NOT NULL,
    install_path           TEXT            NOT NULL,
    source_json            JSON            NOT NULL,
    enabled                BOOLEAN         NOT NULL DEFAULT TRUE,
    installed_at_unix_ms   BIGINT UNSIGNED NOT NULL,
    updated_at_unix_ms     BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (plugin_id),
    INDEX idx_kind (kind),
    INDEX idx_enabled (enabled)
);
```

---

## `source_json` Column Format

Same structure as the `PluginInstallSource` tagged enum serialization:

```json
{ "type": "local_path", "path": "/home/user/my-plugin" }
```

```json
{ "type": "git_url", "url": "https://github.com/org/plugin.git" }
```

---

## Operation Mapping

### Load registry

**File-backed:** `load_registry()` reads `installed.json`, parses JSON.

**Dolt:**
```sql
SELECT plugin_id, kind, name, version, description,
       install_path, source_json, enabled,
       installed_at_unix_ms, updated_at_unix_ms
  FROM plugins;
```

### Store registry (full replacement)

**File-backed:** `store_registry()` serializes entire `InstalledPluginRegistry`
and writes to `installed.json`.

**Dolt:** Individual row operations (insert/update/delete) replace the
full-file rewrite. No single "store all" needed.

### Install plugin

**File-backed:** `install()` copies files to install_root, loads registry,
inserts record, stores registry, writes enabled state to settings.json.

**Dolt:**
```sql
INSERT INTO plugins (
    plugin_id, kind, name, version, description,
    install_path, source_json, enabled,
    installed_at_unix_ms, updated_at_unix_ms
) VALUES (?, ?, ?, ?, ?, ?, ?, TRUE, ?, ?);
```

Plugin file artifacts are still copied to `install_root` on the filesystem.

### Uninstall plugin

**File-backed:** `uninstall()` loads registry, removes record, stores
registry, removes enabled state from settings.json, deletes install_path.

**Dolt:**
```sql
DELETE FROM plugins WHERE plugin_id = ? AND kind != 'bundled';
```

Returns error if zero rows affected (not found or bundled). Plugin directory
is still deleted from the filesystem.

### Update plugin

**File-backed:** `update()` loads registry, replaces version/description/
updated_at, stores registry, replaces files on disk.

**Dolt:**
```sql
UPDATE plugins
   SET version = ?,
       description = ?,
       install_path = ?,
       source_json = ?,
       updated_at_unix_ms = ?
 WHERE plugin_id = ?;
```

Plugin files are still replaced on disk.

### Enable plugin

**File-backed:** `enable()` writes `enabledPlugins[id] = true` to
settings.json.

**Dolt:**
```sql
UPDATE plugins SET enabled = TRUE WHERE plugin_id = ?;
```

### Disable plugin

**File-backed:** `disable()` writes `enabledPlugins[id] = false` to
settings.json.

**Dolt:**
```sql
UPDATE plugins SET enabled = FALSE WHERE plugin_id = ?;
```

### List plugins

**File-backed:** `list_plugins()` discovers builtins + bundled + installed +
external, cross-references enabled state from config.

**Dolt:**
```sql
SELECT plugin_id, kind, name, version, description, enabled
  FROM plugins
 ORDER BY kind, name;
```

### List installed plugins

**File-backed:** `list_installed_plugins()` scans install_root, matches
against registry records.

**Dolt:**
```sql
SELECT * FROM plugins
 WHERE kind IN ('external', 'bundled')
 ORDER BY name;
```

Filesystem scan of install_root for manifest validation still happens at
discovery time — Dolt holds the metadata, disk holds the artifacts.

### Sync bundled plugins

**File-backed:** `sync_bundled_plugins()` scans bundled_root, compares
versions against registry, copies/updates as needed, removes stale entries.

**Dolt:**
```sql
-- Upsert for each bundled plugin found on disk
REPLACE INTO plugins (
    plugin_id, kind, name, version, description,
    install_path, source_json, enabled,
    installed_at_unix_ms, updated_at_unix_ms
) VALUES (?, 'bundled', ?, ?, ?, ?, ?, ?, ?, ?);

-- Remove stale bundled entries not found on disk
DELETE FROM plugins
 WHERE kind = 'bundled'
   AND plugin_id NOT IN (?...);
```

### Stale entry cleanup

**File-backed:** `discover_installed_plugins_with_failures()` removes registry
entries whose install_path no longer exists or has invalid manifests.

**Dolt:**
```sql
DELETE FROM plugins WHERE plugin_id = ?;
```

Called per stale entry after filesystem validation.

### Resolve enabled state

**File-backed:** Read `enabledPlugins` from settings.json, cross-reference
with plugin ID.

**Dolt:**
```sql
SELECT enabled FROM plugins WHERE plugin_id = ?;
```

---

## Improvements Over File-Backed Registry

1. **Single source of truth** — currently plugin state is split across
   `installed.json` (metadata) and `settings.json` (enabled flags). Dolt
   merges both into one table, eliminating two-file coordination bugs.

2. **No full-file rewrites** — `install()`, `uninstall()`, `update()` each
   currently load the entire registry, modify one entry, and rewrite the whole
   file. Dolt does single-row operations.

3. **Plugin history via Dolt versioning** — `dolt log` shows when plugins
   were installed, updated, or removed. Currently there is no history — the
   file is overwritten.

4. **Queryable across installations** — "which plugins are installed across
   all my machines?" becomes possible if registries are synced via Dolt
   remotes.

5. **Atomic enable/disable** — currently a race between two processes could
   corrupt `settings.json`. Dolt row-level updates are atomic.

6. **No JSON parsing on every operation** — `load_registry()` currently
   deserializes the entire file to read or modify a single entry. Dolt
   queries return only what is needed.

---

## Source Reference

| Current code | Role |
|---|---|
| `plugins/src/lib.rs` (lines 360-377) | `InstalledPluginRecord`, `InstalledPluginRegistry` structs |
| `plugins/src/lib.rs` (lines 354-357) | `PluginInstallSource` enum |
| `plugins/src/lib.rs` (lines 23-39) | `PluginKind` enum |
| `plugins/src/lib.rs` (lines 869-871) | `PluginManager` struct |
| `plugins/src/lib.rs` (lines 845-866) | `PluginManagerConfig` |
| `plugins/src/lib.rs` (lines 1051-1063) | `registry_path()`, `settings_path()` |
| `plugins/src/lib.rs` (lines 1115-1156) | `install()` |
| `plugins/src/lib.rs` (lines 1158-1174) | `enable()`, `disable()` |
| `plugins/src/lib.rs` (lines 1176-1194) | `uninstall()` |
| `plugins/src/lib.rs` (lines 1196-1232) | `update()` |
| `plugins/src/lib.rs` (lines 1356-1438) | `sync_bundled_plugins()` |
| `plugins/src/lib.rs` (lines 1461-1480) | `load_registry()`, `store_registry()` |
| `plugins/src/lib.rs` (lines 1482-1498) | `write_enabled_state()` |
| `plugins/src/lib.rs` (lines 2242-2265) | `update_settings_json()` |
| `runtime/src/config.rs` (lines 66-73) | `RuntimePluginConfig` |
