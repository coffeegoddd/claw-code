#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::{
    InstalledPluginRecord, InstalledPluginRegistry, PluginError, REGISTRY_FILE_NAME,
    SETTINGS_FILE_NAME,
};

/// Low-level storage operations for the plugin registry.
///
/// This trait abstracts the read/write layer so that the business logic in
/// [`PluginManager`](crate::PluginManager) (install, uninstall, discovery,
/// sync) can be shared across backends.
///
/// [`FilePluginRegistryStore`] is the original file-backed implementation.
/// [`DoltPluginRegistryStore`](crate::dolt_plugin_registry_store::DoltPluginRegistryStore)
/// is the planned Dolt replacement.
pub trait PluginRegistryStore: Debug + Send + Sync {
    /// Load the full installed plugin registry.
    /// Returns a default (empty) registry if nothing has been persisted yet.
    fn load_registry(&self) -> Result<InstalledPluginRegistry, PluginError>;

    /// Persist the full installed plugin registry, replacing any prior state.
    fn store_registry(&self, registry: &InstalledPluginRegistry) -> Result<(), PluginError>;

    /// Get a single installed plugin record by ID.
    fn get_plugin(&self, plugin_id: &str) -> Result<Option<InstalledPluginRecord>, PluginError> {
        let registry = self.load_registry()?;
        Ok(registry.plugins.get(plugin_id).cloned())
    }

    /// Insert or update a single plugin record in the registry.
    fn upsert_plugin(
        &self,
        plugin_id: &str,
        record: &InstalledPluginRecord,
    ) -> Result<(), PluginError> {
        let mut registry = self.load_registry()?;
        registry
            .plugins
            .insert(plugin_id.to_string(), record.clone());
        self.store_registry(&registry)
    }

    /// Remove a single plugin record from the registry.
    /// Returns the removed record, or `None` if the plugin was not found.
    fn remove_plugin(
        &self,
        plugin_id: &str,
    ) -> Result<Option<InstalledPluginRecord>, PluginError> {
        let mut registry = self.load_registry()?;
        let removed = registry.plugins.remove(plugin_id);
        if removed.is_some() {
            self.store_registry(&registry)?;
        }
        Ok(removed)
    }

    /// Load the enabled/disabled state for all plugins.
    fn load_enabled_state(&self) -> Result<BTreeMap<String, bool>, PluginError>;

    /// Set the enabled state for a single plugin.
    /// Pass `Some(true)` to enable, `Some(false)` to disable, or `None` to
    /// remove the override (revert to default).
    fn write_enabled_state(
        &self,
        plugin_id: &str,
        enabled: Option<bool>,
    ) -> Result<(), PluginError>;
}

// ---------------------------------------------------------------------------
// File-backed implementation (original behavior)
// ---------------------------------------------------------------------------

/// File-backed plugin registry store.
///
/// Stores the plugin registry as `installed.json` and enabled state in
/// `settings.json`, both under `config_home`.
#[derive(Debug, Clone)]
pub struct FilePluginRegistryStore {
    config_home: PathBuf,
    registry_path_override: Option<PathBuf>,
}

impl FilePluginRegistryStore {
    #[must_use]
    pub fn new(config_home: impl Into<PathBuf>) -> Self {
        Self {
            config_home: config_home.into(),
            registry_path_override: None,
        }
    }

    #[must_use]
    pub fn with_registry_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.registry_path_override = Some(path.into());
        self
    }

    fn registry_path(&self) -> PathBuf {
        self.registry_path_override
            .clone()
            .unwrap_or_else(|| self.config_home.join("plugins").join(REGISTRY_FILE_NAME))
    }

    fn settings_path(&self) -> PathBuf {
        self.config_home.join(SETTINGS_FILE_NAME)
    }
}

impl PluginRegistryStore for FilePluginRegistryStore {
    fn load_registry(&self) -> Result<InstalledPluginRegistry, PluginError> {
        let path = self.registry_path();
        match fs::read_to_string(&path) {
            Ok(contents) if contents.trim().is_empty() => Ok(InstalledPluginRegistry::default()),
            Ok(contents) => Ok(serde_json::from_str(&contents)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(InstalledPluginRegistry::default())
            }
            Err(error) => Err(PluginError::Io(error)),
        }
    }

    fn store_registry(&self, registry: &InstalledPluginRegistry) -> Result<(), PluginError> {
        let path = self.registry_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(registry)?)?;
        Ok(())
    }

    fn load_enabled_state(&self) -> Result<BTreeMap<String, bool>, PluginError> {
        let path = self.settings_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) if !contents.trim().is_empty() => contents,
            Ok(_) => return Ok(BTreeMap::new()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(PluginError::Io(error)),
        };

        let root: Value = serde_json::from_str(&contents)?;
        let enabled = root
            .as_object()
            .and_then(|obj| obj.get("enabledPlugins"))
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .filter_map(|(key, value)| value.as_bool().map(|v| (key.clone(), v)))
                    .collect()
            })
            .unwrap_or_default();

        Ok(enabled)
    }

    fn write_enabled_state(
        &self,
        plugin_id: &str,
        enabled: Option<bool>,
    ) -> Result<(), PluginError> {
        let path = self.settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut root = match fs::read_to_string(&path) {
            Ok(contents) if !contents.trim().is_empty() => serde_json::from_str::<Value>(&contents)?,
            Ok(_) => Value::Object(Map::new()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Value::Object(Map::new())
            }
            Err(error) => return Err(PluginError::Io(error)),
        };

        let object = root.as_object_mut().ok_or_else(|| {
            PluginError::InvalidManifest(format!(
                "settings file {} must contain a JSON object",
                path.display()
            ))
        })?;

        let enabled_plugins = object
            .entry("enabledPlugins")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or_else(|| {
                PluginError::InvalidManifest(
                    "enabledPlugins must be a JSON object".to_string(),
                )
            })?;

        match enabled {
            Some(value) => {
                enabled_plugins.insert(plugin_id.to_string(), Value::Bool(value));
            }
            None => {
                enabled_plugins.remove(plugin_id);
            }
        }

        fs::write(&path, serde_json::to_string_pretty(&root)?)?;
        Ok(())
    }
}
