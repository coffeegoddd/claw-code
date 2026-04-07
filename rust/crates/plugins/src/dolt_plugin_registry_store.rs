#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::plugin_registry_store::PluginRegistryStore;
use crate::{InstalledPluginRecord, InstalledPluginRegistry, PluginError};

/// Dolt-backed plugin registry store.
///
/// Stores plugin records and enabled state in a Dolt database using the schema
/// defined in `PLUGIN_REGISTRY_SCHEMA_DESIGN.md`. Replaces
/// [`FilePluginRegistryStore`](crate::plugin_registry_store::FilePluginRegistryStore)
/// for environments where versioned, queryable storage is preferred.
#[derive(Debug, Clone)]
pub struct DoltPluginRegistryStore {
    /// Connection string or path to the Dolt database.
    pub connection: String,
}

impl DoltPluginRegistryStore {
    #[must_use]
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
        }
    }
}

impl PluginRegistryStore for DoltPluginRegistryStore {
    fn load_registry(&self) -> Result<InstalledPluginRegistry, PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::load_registry: unimplemented".to_string(),
        ))
    }

    fn store_registry(&self, _registry: &InstalledPluginRegistry) -> Result<(), PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::store_registry: unimplemented".to_string(),
        ))
    }

    fn get_plugin(&self, _plugin_id: &str) -> Result<Option<InstalledPluginRecord>, PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::get_plugin: unimplemented".to_string(),
        ))
    }

    fn upsert_plugin(
        &self,
        _plugin_id: &str,
        _record: &InstalledPluginRecord,
    ) -> Result<(), PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::upsert_plugin: unimplemented".to_string(),
        ))
    }

    fn remove_plugin(
        &self,
        _plugin_id: &str,
    ) -> Result<Option<InstalledPluginRecord>, PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::remove_plugin: unimplemented".to_string(),
        ))
    }

    fn load_enabled_state(&self) -> Result<BTreeMap<String, bool>, PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::load_enabled_state: unimplemented".to_string(),
        ))
    }

    fn write_enabled_state(
        &self,
        _plugin_id: &str,
        _enabled: Option<bool>,
    ) -> Result<(), PluginError> {
        Err(PluginError::CommandFailed(
            "DoltPluginRegistryStore::write_enabled_state: unimplemented".to_string(),
        ))
    }
}
