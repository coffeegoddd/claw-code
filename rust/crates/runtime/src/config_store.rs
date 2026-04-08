#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{ConfigError, ConfigSource};
use crate::json::JsonValue;

/// A raw config layer loaded from a store backend.
#[derive(Debug, Clone)]
pub struct ConfigLayer {
    pub scope: ConfigSource,
    pub object: BTreeMap<String, JsonValue>,
    /// The raw JSON source text, used for line-number reporting in
    /// validation diagnostics. Empty string if not available (e.g., Dolt).
    pub source_text: String,
}

/// Low-level storage operations for configuration data.
///
/// This trait abstracts where config layers are read from. The merge logic
/// in [`ConfigLoader`](crate::config::ConfigLoader) is unchanged — it
/// iterates the layers returned by the store and deep-merges them.
///
/// [`FileConfigStore`] is the original file-backed implementation.
/// [`DoltConfigStore`](crate::dolt_config_store::DoltConfigStore) is the
/// planned Dolt replacement.
/// Low-level storage operations for configuration data.
///
/// Workspace isolation is handled internally by each backend — the file
/// backend uses directory paths, the Dolt backend reads user config from
/// `main` and project/local config from the workspace branch (see
/// `DOLT_BRANCHING_STRATEGY.md`). Callers never pass workspace identifiers.
pub trait ConfigStore: Debug + Send + Sync {
    /// Load all config layers in precedence order (lowest first).
    /// Layers with missing or empty content are omitted.
    fn load_layers(&self) -> Result<Vec<ConfigLayer>, ConfigError>;

    /// Load a single config layer by scope. Returns `None` if the layer
    /// does not exist.
    fn load_layer(&self, scope: ConfigSource) -> Result<Option<ConfigLayer>, ConfigError>;

    /// Write a config layer. Creates or replaces the layer for the given
    /// scope.
    fn store_layer(
        &self,
        scope: ConfigSource,
        object: &BTreeMap<String, JsonValue>,
    ) -> Result<(), ConfigError>;

    /// Delete a config layer. No-op if it doesn't exist.
    fn delete_layer(&self, scope: ConfigSource) -> Result<(), ConfigError>;
}

// ---------------------------------------------------------------------------
// File-backed implementation (original behavior)
// ---------------------------------------------------------------------------

/// File-backed config store.
///
/// Reads config from the standard file locations:
/// - User: `~/.claw.json` (legacy), `~/.claw/settings.json`
/// - Project: `<cwd>/.claw.json` (legacy), `<cwd>/.claw/settings.json`
/// - Local: `<cwd>/.claw/settings.local.json`
#[derive(Debug, Clone)]
pub struct FileConfigStore {
    cwd: PathBuf,
    config_home: PathBuf,
}

impl FileConfigStore {
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>, config_home: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            config_home: config_home.into(),
        }
    }

    /// Return the ordered list of file paths to check, matching
    /// `ConfigLoader::discover()`.
    fn file_entries(&self) -> Vec<(ConfigSource, PathBuf)> {
        let user_legacy_path = self
            .config_home
            .parent()
            .map_or_else(|| PathBuf::from(".claw.json"), |p| p.join(".claw.json"));

        vec![
            (ConfigSource::User, user_legacy_path),
            (ConfigSource::User, self.config_home.join("settings.json")),
            (
                ConfigSource::Project,
                self.cwd.join(".claw.json"),
            ),
            (
                ConfigSource::Project,
                self.cwd.join(".claw").join("settings.json"),
            ),
            (
                ConfigSource::Local,
                self.cwd.join(".claw").join("settings.local.json"),
            ),
        ]
    }
}

impl ConfigStore for FileConfigStore {
    fn load_layers(&self) -> Result<Vec<ConfigLayer>, ConfigError> {
        let mut layers = Vec::new();
        for (scope, path) in self.file_entries() {
            if let Some(layer) = read_config_layer(scope, &path)? {
                layers.push(layer);
            }
        }
        Ok(layers)
    }

    fn load_layer(&self, scope: ConfigSource) -> Result<Option<ConfigLayer>, ConfigError> {
        for (entry_scope, path) in self.file_entries() {
            if entry_scope == scope {
                if let Some(layer) = read_config_layer(scope, &path)? {
                    return Ok(Some(layer));
                }
            }
        }
        Ok(None)
    }

    fn store_layer(
        &self,
        _scope: ConfigSource,
        _object: &BTreeMap<String, JsonValue>,
    ) -> Result<(), ConfigError> {
        // File-backed config is read-only from the application's perspective.
        Err(ConfigError::Parse(
            "FileConfigStore does not support writes".to_string(),
        ))
    }

    fn delete_layer(&self, _scope: ConfigSource) -> Result<(), ConfigError> {
        Err(ConfigError::Parse(
            "FileConfigStore does not support deletes".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// File I/O helpers
// ---------------------------------------------------------------------------

fn read_config_layer(
    scope: ConfigSource,
    path: &Path,
) -> Result<Option<ConfigLayer>, ConfigError> {
    let is_legacy = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == ".claw.json");

    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::Io(error)),
    };

    if contents.trim().is_empty() {
        return Ok(Some(ConfigLayer {
            scope,
            object: BTreeMap::new(),
            source_text: contents,
        }));
    }

    let parsed = match JsonValue::parse(&contents) {
        Ok(parsed) => parsed,
        Err(_) if is_legacy => return Ok(None),
        Err(error) => {
            return Err(ConfigError::Parse(format!("{}: {error}", path.display())));
        }
    };

    let Some(object) = parsed.as_object() else {
        if is_legacy {
            return Ok(None);
        }
        return Err(ConfigError::Parse(format!(
            "{}: top-level settings value must be a JSON object",
            path.display()
        )));
    };

    Ok(Some(ConfigLayer {
        scope,
        object: object.clone(),
        source_text: contents,
    }))
}
