#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::config::{ConfigError, ConfigSource};
use crate::config_store::{ConfigLayer, ConfigStore};
use crate::json::JsonValue;

/// Dolt-backed configuration store.
///
/// Stores config layers in a Dolt database using the schema defined in
/// `CONFIG_STORE_SCHEMA_DESIGN.md`. Each layer (user, project, local) is a
/// row with a JSON blob. The application merges layers at load time using
/// the same `deep_merge_objects()` logic as the file-backed store.
#[derive(Debug, Clone)]
pub struct DoltConfigStore {
    /// Connection string or path to the Dolt database.
    pub connection: String,
}

impl DoltConfigStore {
    #[must_use]
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
        }
    }
}

impl ConfigStore for DoltConfigStore {
    fn load_layers(
        &self,
        _workspace_fingerprint: Option<&str>,
    ) -> Result<Vec<ConfigLayer>, ConfigError> {
        Err(ConfigError::Parse(
            "DoltConfigStore::load_layers: unimplemented".to_string(),
        ))
    }

    fn load_layer(
        &self,
        _scope: ConfigSource,
        _workspace_fingerprint: Option<&str>,
    ) -> Result<Option<ConfigLayer>, ConfigError> {
        Err(ConfigError::Parse(
            "DoltConfigStore::load_layer: unimplemented".to_string(),
        ))
    }

    fn store_layer(
        &self,
        _scope: ConfigSource,
        _workspace_fingerprint: Option<&str>,
        _object: &BTreeMap<String, JsonValue>,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::Parse(
            "DoltConfigStore::store_layer: unimplemented".to_string(),
        ))
    }

    fn delete_layer(
        &self,
        _scope: ConfigSource,
        _workspace_fingerprint: Option<&str>,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::Parse(
            "DoltConfigStore::delete_layer: unimplemented".to_string(),
        ))
    }
}
