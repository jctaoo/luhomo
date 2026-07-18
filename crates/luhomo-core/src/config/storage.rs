use bytes::Bytes;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, instrument, trace, warn};

use crate::config::models::ConfigurationItem;

#[derive(Debug, Error)]
pub enum ConfigurationStorageError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("configuration not found: {0}")]
    NotFound(uuid::Uuid),
}

pub struct ConfigurationStorage {
    base_dir: PathBuf,
}

/// TODO: The update and delete should be atomic, i.e. if the write fails, the indexes should not be updated.
impl ConfigurationStorage {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        debug!(path = %base_dir.display(), "created configuration storage");
        Self {
            base_dir,
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn indexes_path(&self) -> PathBuf {
        self.base_dir.join("indexes.json")
    }

    pub fn file_name(uuid: &uuid::Uuid) -> String {
        format!("{uuid}")
    }

    pub fn file_path(&self, uuid: &uuid::Uuid) -> PathBuf {
        self.base_dir.join(Self::file_name(uuid))
    }

    async fn read_indexes(&self) -> Result<Vec<ConfigurationItem>, ConfigurationStorageError> {
        let path = self.indexes_path();
        if !path.exists() {
            debug!(path = %path.display(), "configuration index does not exist");
            return Ok(Vec::new());
        }
        let content = tokio::fs::read(&path).await?;
        let items: Vec<ConfigurationItem> = serde_json::from_slice(&content)?;
        trace!(path = %path.display(), count = items.len(), "read configuration index");
        Ok(items)
    }

    async fn write_indexes(
        &self,
        items: &[ConfigurationItem],
    ) -> Result<(), ConfigurationStorageError> {
        tokio::fs::create_dir_all(&self.base_dir).await?;
        let json = serde_json::to_vec_pretty(items)?;
        tokio::fs::write(self.indexes_path(), &json).await?;
        debug!(count = items.len(), bytes = json.len(), "wrote configuration index");
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<ConfigurationItem>, ConfigurationStorageError> {
        let items = self.read_indexes().await?;
        debug!(path = %self.indexes_path().display(), count = items.len(), "loaded configurations from storage");
        Ok(items)
    }

    #[instrument(name = "cfgstore.get", skip(self), fields(uuid = %uuid))]
    pub async fn get(&self, uuid: &uuid::Uuid) -> Result<Bytes, ConfigurationStorageError> {
        let path = self.file_path(uuid);
        trace!(path = %path.display(), "trying to read configuration content from storage");
        let items = self.read_indexes().await?;
        if !items.iter().any(|item| item.uuid == *uuid) {
            warn!(path = %path.display(), "configuration content not found in storage index");
            return Err(ConfigurationStorageError::NotFound(*uuid));
        }
        let content = tokio::fs::read(&path).await?;
        debug!(path = %path.display(), bytes = content.len(), "loaded configuration content from storage");
        Ok(Bytes::from(content))
    }

    #[instrument(name = "cfgstore.delete", skip(self), fields(uuid = %uuid))]
    pub async fn delete(&self, uuid: &uuid::Uuid) -> Result<(), ConfigurationStorageError> {
        let path = self.file_path(uuid);
        debug!(path = %path.display(), "removing configuration from storage");
        let mut items = self.read_indexes().await?;
        let len_before = items.len();
        items.retain(|item| item.uuid != *uuid);
        if items.len() == len_before {
            warn!(path = %path.display(), "configuration not found in storage");
            return Err(ConfigurationStorageError::NotFound(*uuid));
        }
        self.write_indexes(&items).await?;

        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        debug!(path = %path.display(), "removed configuration from storage");
        Ok(())
    }

    #[instrument(name = "cfgstore.update", skip(self, item, content), fields(uuid = %item.uuid))]
    pub async fn update(
        &self,
        item: &ConfigurationItem,
        content: &Bytes,
    ) -> Result<(), ConfigurationStorageError> {
        let content_path = self.file_path(&item.uuid);
        debug!(path = %content_path.display(), bytes = content.len(), "writing configuration to storage");
        tokio::fs::create_dir_all(&self.base_dir).await?;
        tokio::fs::write(&content_path, content).await?;

        let mut items = self.read_indexes().await?;
        if let Some(existing) = items.iter_mut().find(|i| i.uuid == item.uuid) {
            *existing = item.clone();
        } else {
            items.push(item.clone());
        }
        self.write_indexes(&items).await?;

        debug!(path = %content_path.display(), count = items.len(), "stored configuration");
        Ok(())
    }
}
