#![allow(dead_code)]

use std::fmt::Debug;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use runtime::{dedupe_superseded_commit_events, LaneEvent, LaneEventBlocker};
use serde::{Deserialize, Serialize};

/// Persisted agent manifest. This is the single source-of-truth type for agent
/// metadata, used across all store backends and the agent execution pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "subagentType")]
    pub subagent_type: Option<String>,
    pub model: Option<String>,
    pub status: String,
    #[serde(rename = "outputFile")]
    pub output_file: String,
    #[serde(rename = "manifestFile")]
    pub manifest_file: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "startedAt", skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(rename = "completedAt", skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(rename = "laneEvents", default, skip_serializing_if = "Vec::is_empty")]
    pub lane_events: Vec<LaneEvent>,
    #[serde(rename = "currentBlocker", skip_serializing_if = "Option::is_none")]
    pub current_blocker: Option<LaneEventBlocker>,
    #[serde(rename = "derivedState")]
    pub derived_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Low-level storage operations for the agent store.
///
/// This trait abstracts the read/write layer so that agent lifecycle logic
/// (spawn, completion, failure classification) can be shared across backends.
///
/// [`FileAgentStore`] is the original file-backed implementation.
/// [`DoltAgentStore`](crate::dolt_agent_store::DoltAgentStore) is the planned
/// Dolt replacement.
pub trait AgentStore: Debug + Send + Sync {
    /// Persist an initial agent manifest (on creation/spawn).
    fn create_agent(&self, manifest: &AgentManifest) -> Result<(), AgentStoreError>;

    /// Load an agent manifest by ID.
    fn load_agent(&self, agent_id: &str) -> Result<Option<AgentManifest>, AgentStoreError>;

    /// Persist an updated agent manifest (overwrites prior state).
    fn update_agent(&self, manifest: &AgentManifest) -> Result<(), AgentStoreError>;

    /// Write the initial output content (markdown header + prompt).
    fn write_output(&self, agent_id: &str, content: &str) -> Result<(), AgentStoreError>;

    /// Append text to an agent's output.
    fn append_output(&self, agent_id: &str, suffix: &str) -> Result<(), AgentStoreError>;

    /// List all agents in the store. Returns manifests sorted by creation
    /// time descending.
    fn list_agents(&self) -> Result<Vec<AgentManifest>, AgentStoreError>;

    /// Return a human-readable description of where agents are stored.
    fn storage_location(&self) -> String;

    /// Return the filesystem path for an agent's manifest, if file-based.
    fn manifest_path(&self, _agent_id: &str) -> Option<PathBuf> {
        None
    }

    /// Return the filesystem path for an agent's output, if file-based.
    fn output_path(&self, _agent_id: &str) -> Option<PathBuf> {
        None
    }
}

/// Errors raised by [`AgentStore`] implementations.
#[derive(Debug)]
pub enum AgentStoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Format(String),
    Unimplemented(String),
}

impl std::fmt::Display for AgentStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Format(msg) => write!(f, "{msg}"),
            Self::Unimplemented(op) => write!(f, "operation not implemented: {op}"),
        }
    }
}

impl std::error::Error for AgentStoreError {}

impl From<std::io::Error> for AgentStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for AgentStoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<AgentStoreError> for String {
    fn from(value: AgentStoreError) -> Self {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// File-backed implementation (original behavior)
// ---------------------------------------------------------------------------

/// File-backed agent store.
///
/// Stores agent manifests as `<agent_id>.json` and output as `<agent_id>.md`
/// in the agent store directory.
#[derive(Debug, Clone)]
pub struct FileAgentStore {
    store_dir: PathBuf,
}

impl FileAgentStore {
    pub fn new(store_dir: impl Into<PathBuf>) -> Result<Self, AgentStoreError> {
        let store_dir = store_dir.into();
        fs::create_dir_all(&store_dir)?;
        Ok(Self { store_dir })
    }

    /// Resolve the store directory using the same logic as `agent_store_dir()`.
    pub fn from_env() -> Result<Self, AgentStoreError> {
        let dir = resolve_agent_store_dir()?;
        Self::new(dir)
    }

    fn resolve_manifest_path(&self, agent_id: &str) -> PathBuf {
        self.store_dir.join(format!("{agent_id}.json"))
    }

    fn resolve_output_path(&self, agent_id: &str) -> PathBuf {
        self.store_dir.join(format!("{agent_id}.md"))
    }
}

impl AgentStore for FileAgentStore {
    fn create_agent(&self, manifest: &AgentManifest) -> Result<(), AgentStoreError> {
        let mut normalized = manifest.clone();
        normalized.lane_events = dedupe_superseded_commit_events(&normalized.lane_events);
        // Populate file paths for the stored manifest.
        normalized.manifest_file = self
            .resolve_manifest_path(&manifest.agent_id)
            .display()
            .to_string();
        normalized.output_file = self
            .resolve_output_path(&manifest.agent_id)
            .display()
            .to_string();
        let json = serde_json::to_string_pretty(&normalized)?;
        fs::write(self.resolve_manifest_path(&manifest.agent_id), json)?;
        Ok(())
    }

    fn load_agent(&self, agent_id: &str) -> Result<Option<AgentManifest>, AgentStoreError> {
        let path = self.resolve_manifest_path(agent_id);
        match fs::read_to_string(&path) {
            Ok(contents) => Ok(Some(serde_json::from_str(&contents)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(AgentStoreError::Io(error)),
        }
    }

    fn update_agent(&self, manifest: &AgentManifest) -> Result<(), AgentStoreError> {
        self.create_agent(manifest)
    }

    fn write_output(&self, agent_id: &str, content: &str) -> Result<(), AgentStoreError> {
        fs::write(self.resolve_output_path(agent_id), content)?;
        Ok(())
    }

    fn append_output(&self, agent_id: &str, suffix: &str) -> Result<(), AgentStoreError> {
        let path = self.resolve_output_path(agent_id);
        let mut file = fs::OpenOptions::new().append(true).open(&path)?;
        file.write_all(suffix.as_bytes())?;
        Ok(())
    }

    fn list_agents(&self) -> Result<Vec<AgentManifest>, AgentStoreError> {
        let mut agents = Vec::new();
        let entries = match fs::read_dir(&self.store_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(agents),
            Err(error) => return Err(AgentStoreError::Io(error)),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(contents) = fs::read_to_string(&path) {
                if let Ok(manifest) = serde_json::from_str::<AgentManifest>(&contents) {
                    agents.push(manifest);
                }
            }
        }
        agents.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(agents)
    }

    fn storage_location(&self) -> String {
        self.store_dir.display().to_string()
    }

    fn manifest_path(&self, agent_id: &str) -> Option<PathBuf> {
        Some(self.resolve_manifest_path(agent_id))
    }

    fn output_path(&self, agent_id: &str) -> Option<PathBuf> {
        Some(self.resolve_output_path(agent_id))
    }
}

// ---------------------------------------------------------------------------
// Path resolution (matches agent_store_dir() in lib.rs)
// ---------------------------------------------------------------------------

fn resolve_agent_store_dir() -> Result<PathBuf, AgentStoreError> {
    if let Ok(path) = std::env::var("CLAWD_AGENT_STORE") {
        return Ok(PathBuf::from(path));
    }
    let cwd = std::env::current_dir()?;
    if let Some(workspace_root) = cwd.ancestors().nth(2) {
        return Ok(workspace_root.join(".clawd-agents"));
    }
    Ok(cwd.join(".clawd-agents"))
}
