pub mod models;
pub mod fetcher;
pub mod storage;

use bytes::Bytes;
#[cfg(feature = "reqwest")]
use std::path::PathBuf;
use thiserror::Error;

use fetcher::{ConfigurationFetcher, ConfigurationFetcherError};
use models::{ConfigurationItem, ConfigurationSource};
use storage::{ConfigurationStorage, ConfigurationStorageError};

#[derive(Debug, Error)]
pub enum ConfigurationManagerError {
    #[error("storage: {0}")]
    Storage(#[from] ConfigurationStorageError),
    #[error("fetch: {0}")]
    Fetch(#[from] ConfigurationFetcherError),
}

pub fn source_display_name(source: &ConfigurationSource) -> String {
    match source {
        ConfigurationSource::LocalFile(path) => std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string(),
        ConfigurationSource::RemoteUrl { url, .. } => {
            url.host_str().unwrap_or(url.as_str()).to_string()
        }
    }
}

pub trait ConfigurationManager {
    type Fetcher: ConfigurationFetcher;

    fn storage(&self) -> &ConfigurationStorage;
    fn fetcher(&self) -> &Self::Fetcher;

    async fn add(
        &self,
        source: ConfigurationSource,
    ) -> Result<ConfigurationItem, ConfigurationManagerError> {
        let item = ConfigurationItem::builder()
            .source(source.clone())
            .display_name(source_display_name(&source))
            .build();

        let content = self.fetcher().fetch_configuration(&source, None).await?;
        self.storage().update(&item, &content).await?;

        Ok(item)
    }

    async fn list(&self) -> Result<Vec<ConfigurationItem>, ConfigurationManagerError> {
        Ok(self.storage().list().await?)
    }

    async fn get_content(
        &self,
        uuid: &uuid::Uuid,
    ) -> Result<Bytes, ConfigurationManagerError> {
        Ok(self.storage().get(uuid).await?)
    }

    async fn delete(&self, uuid: &uuid::Uuid) -> Result<(), ConfigurationManagerError> {
        Ok(self.storage().delete(uuid).await?)
    }

    async fn update(
        &self,
        uuid: &uuid::Uuid,
    ) -> Result<ConfigurationItem, ConfigurationManagerError> {
        let items = self.storage().list().await?;
        let item = items
            .iter()
            .find(|i| i.uuid == *uuid)
            .ok_or(ConfigurationStorageError::NotFound(*uuid))?;

        let content = self
            .fetcher()
            .fetch_configuration(&item.source, Some(item))
            .await?;

        let mut updated = item.clone();
        updated.updated_at = time::OffsetDateTime::now_utc();
        self.storage().update(&updated, &content).await?;

        Ok(updated)
    }
}

#[cfg(feature = "reqwest")]
pub struct LocalConfigurationManager {
    storage: ConfigurationStorage,
    fetcher: fetcher::RemoteConfigurationFetcher,
}

#[cfg(feature = "reqwest")]
impl LocalConfigurationManager {
    pub fn new(storage_dir: impl Into<PathBuf>, client: reqwest::Client) -> Self {
        Self {
            storage: ConfigurationStorage::new(storage_dir),
            fetcher: fetcher::RemoteConfigurationFetcher::new(client),
        }
    }

    pub fn storage_dir(&self) -> &PathBuf {
        self.storage.base_dir()
    }
}

#[cfg(feature = "reqwest")]
impl ConfigurationManager for LocalConfigurationManager {
    type Fetcher = fetcher::RemoteConfigurationFetcher;

    fn storage(&self) -> &ConfigurationStorage {
        &self.storage
    }

    fn fetcher(&self) -> &Self::Fetcher {
        &self.fetcher
    }
}
