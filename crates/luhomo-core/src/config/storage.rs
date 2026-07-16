use bytes::Bytes;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, warn};

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
        debug!(path = %path.display(), count = items.len(), "read configuration index");
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
        info!(count = items.len(), "loaded configurations from storage");
        Ok(items)
    }

    pub async fn get(&self, uuid: &uuid::Uuid) -> Result<Bytes, ConfigurationStorageError> {
        debug!(uuid = %uuid, "loading configuration content from storage");
        let items = self.read_indexes().await?;
        if !items.iter().any(|item| item.uuid == *uuid) {
            warn!(uuid = %uuid, "configuration content not found in storage index");
            return Err(ConfigurationStorageError::NotFound(*uuid));
        }
        let path = self.file_path(uuid);
        let content = tokio::fs::read(&path).await?;
        debug!(uuid = %uuid, bytes = content.len(), "loaded configuration content from storage");
        Ok(Bytes::from(content))
    }

    pub async fn delete(&self, uuid: &uuid::Uuid) -> Result<(), ConfigurationStorageError> {
        debug!(uuid = %uuid, "removing configuration from storage");
        let mut items = self.read_indexes().await?;
        let len_before = items.len();
        items.retain(|item| item.uuid != *uuid);
        if items.len() == len_before {
            warn!(uuid = %uuid, "configuration not found in storage");
            return Err(ConfigurationStorageError::NotFound(*uuid));
        }
        self.write_indexes(&items).await?;

        let path = self.file_path(uuid);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        info!(uuid = %uuid, "removed configuration from storage");
        Ok(())
    }

    pub async fn update(
        &self,
        item: &ConfigurationItem,
        content: &Bytes,
    ) -> Result<(), ConfigurationStorageError> {
        debug!(uuid = %item.uuid, bytes = content.len(), "writing configuration to storage");
        tokio::fs::create_dir_all(&self.base_dir).await?;

        let content_path = self.file_path(&item.uuid);
        tokio::fs::write(&content_path, content).await?;

        let mut items = self.read_indexes().await?;
        if let Some(existing) = items.iter_mut().find(|i| i.uuid == item.uuid) {
            *existing = item.clone();
        } else {
            items.push(item.clone());
        }
        self.write_indexes(&items).await?;

        info!(uuid = %item.uuid, count = items.len(), "stored configuration");
        Ok(())
    }
}
