# State of the World: Data Persistence in claw-code

> Catalog of every location where data is persisted to a file or store.
> Goal: inform a future migration from file-based storage to Dolt.

## Overview

claw-code has **zero database dependencies**. All persistence is file-based,
using `std::fs` (or `tokio::fs`) and `serde_json`. Data lands in JSON, JSONL,
or Markdown files scattered across home-directory and project-directory trees.

---

## 1. Session / Conversation History

| Field | Value |
|-------|-------|
| **Path** | `<cwd>/.claw/sessions/<workspace_hash>/<session-id>.jsonl` |
| **Format** | JSONL (append-only) |
| **Source** | `runtime/src/session.rs` (lines 76-100, 528, 1021-1094) |
| **Source** | `runtime/src/session_control.rs` (lines 10-62) |

**Data stored:**
- Conversation messages (user, assistant, tool)
- Tool use records
- Token usage statistics
- Compaction / summarization records
- Fork / branch metadata
- Prompt history entries

**Behavior:**
- Atomic writes via temp-file + `fs::rename`.
- Auto-rotation when a session file exceeds 256 KB.
- Keeps a maximum of 3 rotated files (`*.rot-<timestamp>.jsonl`).
- Workspace-fingerprinted directories (FNV-1a hash of canonical path) to
  prevent collisions between parallel instances.
- Legacy `.json` format also supported for reads.

---

## 2. Prompt Cache

| Field | Value |
|-------|-------|
| **Path** | `~/.claude/cache/prompt-cache/<session_id>/completions/<hash>.json` |
| **Format** | JSON (one file per cached response) |
| **Source** | `api/src/prompt_cache.rs` (lines 421-427) |

**Data stored:**
- Cached API responses with metadata
- Request fingerprints (model, system, tools, messages hashes)
- Aggregate stats: hits, misses, writes, unexpected cache breaks
- Session-level prompt state (`session-state.json`)

**Behavior:**
- TTL-based expiration (30 s for completions, 5 min for prompts).
- FNV-1a hashing for request deduplication.
- Cache-break detection.

---

## 3. OAuth Credentials

| Field | Value |
|-------|-------|
| **Path** | `~/.claw/credentials.json` (or `$CLAW_CONFIG_HOME/credentials.json`) |
| **Format** | JSON |
| **Source** | `runtime/src/oauth.rs` (lines 265-299) |

**Data stored:**
- `accessToken`, `refreshToken`, `expiresAt`, `scopes`

**Functions:** `load_oauth_credentials()`, `save_oauth_credentials()`,
`clear_oauth_credentials()`.

---

## 4. Runtime Configuration (multi-level merge)

| Field | Value |
|-------|-------|
| **Paths** | `~/.claw/settings.json`, `<cwd>/.claw.json`, `<cwd>/.claw/settings.json`, `<cwd>/.claw/settings.local.json` |
| **Format** | JSON |
| **Source** | `runtime/src/config.rs` (lines 234-290, 580-586) |

**Data stored:**
- MCP server configurations
- Permission rules and default mode
- Plugin settings
- Hook definitions (`preToolUse`, `postToolUse`, `postToolUseFailure`)
- Provider fallback chains
- Trusted roots

**Resolution order (highest precedence last):**
1. `~/.claw.json` (legacy user)
2. `~/.claw/settings.json` (user home)
3. `<cwd>/.claw.json` (legacy project)
4. `<cwd>/.claw/settings.json` (project)
5. `<cwd>/.claw/settings.local.json` (machine-local overrides)

---

## 5. Worker Boot State

| Field | Value |
|-------|-------|
| **Path** | `<cwd>/.claw/worker-state.json` |
| **Format** | JSON |
| **Source** | `runtime/src/worker_boot.rs` (lines 580-614) |

**Data stored:**
- Worker ID and status
- Trust-gate state
- Prompt-in-flight flag
- Last event type
- `updated_at` timestamp and `seconds_since_update`

**Behavior:** Atomic writes via temp-file + `fs::rename`.

---

## 6. Plugin Registry

| Field | Value |
|-------|-------|
| **Path** | `~/.claw/plugins/installed.json` |
| **Format** | JSON |
| **Source** | `plugins/src/lib.rs` (lines 374, 1461-1485) |

**Data stored:**
- Installed plugin records: kind, id, name, version, description,
  install_path, source, installed/updated timestamps.

---

## 7. Todo / Task Store

| Field | Value |
|-------|-------|
| **Path** | `<cwd>/.clawd-todos.json` (overridable via `$CLAWD_TODO_STORE`) |
| **Format** | JSON |
| **Source** | `tools/src/lib.rs` (lines 2980-2985) |

**Data stored:** Project-scoped task/todo state.

---

## 8. Agent Store

| Field | Value |
|-------|-------|
| **Path** | `<cwd>/.clawd-agents/` (overridable via `$CLAWD_AGENT_STORE`) |
| **Format** | JSON files in a directory |
| **Source** | `tools/src/lib.rs` (lines 4204-4212) |

**Data stored:** Agent/sub-agent registry and state.
Workspace detection checks up to 2 ancestor directories.

---

## 9. Tool State

| Field | Value |
|-------|-------|
| **Path** | `<cwd>/.claw/tool-state/*.json` (e.g., `plan-mode.json`) |
| **Format** | JSON |
| **Source** | `tools/src/lib.rs` (lines 5043, 7560-7638) |

**Data stored:** Per-tool mode/state blobs.

---

## 10. Telemetry Events

| Field | Value |
|-------|-------|
| **Path** | Configurable JSONL file |
| **Format** | JSONL (append-only) |
| **Source** | `telemetry/src/lib.rs` (line 252) |

**Data stored:** Analytics events and session traces.

---

## 11. Instruction Files (read-only)

| Field | Value |
|-------|-------|
| **Paths** | `CLAUDE.md`, `CLAUDE.local.md`, `.claw/CLAUDE.md`, `.claw/instructions.md` |
| **Source** | `runtime/src/prompt.rs` (lines 203-236) |

These are user-authored Markdown files loaded into prompts.
The application **reads but never writes** them.

---

## 12. Ancillary File Writes

These are not persistent stores but do touch the filesystem:

| What | Path | Source |
|------|------|--------|
| Sandbox dirs | `<cwd>/.sandbox-home/`, `<cwd>/.sandbox-tmp/` | `runtime/src/bash.rs:240-241` |
| MCP wrapper scripts | Temp dirs | `runtime/src/mcp_stdio.rs`, `runtime/src/mcp_tool_bridge.rs` |
| Skill/plugin install | `~/.claw/plugins/`, `.claude-plugin/` | `commands/src/lib.rs:2845-2851, 4022-4068` |
| Agent terminal output | Appended to file | `tools/src/lib.rs:3637-3641` |
| `.gitignore` entries | `<cwd>/.gitignore` | `rusty-claude-cli/src/init.rs:390` |
| Provider `.env` | Temp dir | `api/src/providers/mod.rs:547-549` |

---

## Migration Priority for Dolt

### High value (structured data, query/history needs)

1. **Session store** -- append-only conversation history with rotation,
   workspace isolation, and fork metadata. Biggest win: versioned history,
   branching, SQL queries over conversations.
2. **Prompt cache** -- TTL-based response cache with stats. Good fit for a
   table with expiry queries.
3. **Todo / Agent stores** -- project-scoped structured data, currently
   scattered JSON files.
4. **Plugin registry** -- global install records.

### Medium value (simple key-value, less query benefit)

5. **Configuration** -- multi-level settings merge could be rows in a config
   table, but the merge/precedence semantics need careful mapping.
6. **Worker boot state** -- ephemeral single-row state.
7. **Tool state** -- small per-tool JSON blobs.

### Low value / keep as files

8. **OAuth credentials** -- sensitive single-record data, better left as a
   local file (or moved to OS keychain).
9. **Telemetry** -- append-only log, may be better suited to a log sink.
10. **Instruction files** -- user-authored Markdown, read-only by the app.
