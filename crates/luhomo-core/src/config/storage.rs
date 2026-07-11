use bytes::Bytes;
use std::path::{Path, PathBuf};
use thiserror::Error;

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
        Self {
            base_dir: base_dir.into(),
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
            return Ok(Vec::new());
        }
        let content = tokio::fs::read(&path).await?;
        Ok(serde_json::from_slice(&content)?)
    }

    async fn write_indexes(
        &self,
        items: &[ConfigurationItem],
    ) -> Result<(), ConfigurationStorageError> {
        tokio::fs::create_dir_all(&self.base_dir).await?;
        let json = serde_json::to_vec_pretty(items)?;
        tokio::fs::write(self.indexes_path(), &json).await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<ConfigurationItem>, ConfigurationStorageError> {
        self.read_indexes().await
    }

    pub async fn get(&self, uuid: &uuid::Uuid) -> Result<Bytes, ConfigurationStorageError> {
        let items = self.read_indexes().await?;
        if !items.iter().any(|item| item.uuid == *uuid) {
            return Err(ConfigurationStorageError::NotFound(*uuid));
        }
        let path = self.file_path(uuid);
        let content = tokio::fs::read(&path).await?;
        Ok(Bytes::from(content))
    }

    pub async fn delete(&self, uuid: &uuid::Uuid) -> Result<(), ConfigurationStorageError> {
        let mut items = self.read_indexes().await?;
        let len_before = items.len();
        items.retain(|item| item.uuid != *uuid);
        if items.len() == len_before {
            return Err(ConfigurationStorageError::NotFound(*uuid));
        }
        self.write_indexes(&items).await?;

        let path = self.file_path(uuid);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    pub async fn update(
        &self,
        item: &ConfigurationItem,
        content: &Bytes,
    ) -> Result<(), ConfigurationStorageError> {
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

        Ok(())
    }
}
