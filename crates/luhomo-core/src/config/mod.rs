pub mod fetcher;
pub mod models;
pub mod runtime_source;
pub mod storage;

pub use runtime_source::{RuntimeConfigSource, StaticRuntimeConfigSource};

use bytes::Bytes;
#[cfg(feature = "reqwest")]
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

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
        ConfigurationSource::RemoteUrl { url, .. } => url.host_str().unwrap_or(url.as_str()).to_string(),
    }
}

pub trait ConfigurationManager {
    type Fetcher: ConfigurationFetcher;

    fn storage(&self) -> &ConfigurationStorage;
    fn fetcher(&self) -> &Self::Fetcher;

    #[instrument(name = "cfgmgr.add", skip(self, source))]
    async fn add(&self, source: ConfigurationSource) -> Result<ConfigurationItem, ConfigurationManagerError> {
        let item = ConfigurationItem::builder()
            .source(source.clone())
            .display_name(source_display_name(&source))
            .build();
        info!(uuid = %item.uuid, display_name = %item.display_name, "adding configuration");

        let content = match self.fetcher().fetch_configuration(&source, None).await {
            Ok(content) => content,
            Err(error) => {
                warn!(error = %error, "failed to fetch configuration");
                return Err(error.into());
            }
        };
        self.storage().update(&item, &content).await?;

        info!(bytes = content.len(), "configuration added");

        Ok(item)
    }

    #[instrument(name = "cfgmgr.list", skip(self))]
    async fn list(&self) -> Result<Vec<ConfigurationItem>, ConfigurationManagerError> {
        debug!("listing configurations");
        let items = self.storage().list().await?;
        info!(count = items.len(), "listed configurations");
        Ok(items)
    }

    #[instrument(name = "cfgmgr.get", skip(self, uuid))]
    async fn get_content(&self, uuid: &uuid::Uuid) -> Result<Bytes, ConfigurationManagerError> {
        debug!("reading configuration content");
        let content = self.storage().get(uuid).await?;
        info!(bytes = content.len(), "read configuration content");
        Ok(content)
    }

    #[instrument(name = "cfgmgr.delete", skip(self, uuid))]
    async fn delete(&self, uuid: &uuid::Uuid) -> Result<(), ConfigurationManagerError> {
        debug!("deleting configuration");
        self.storage().delete(uuid).await?;
        info!("configuration deleted");
        Ok(())
    }

    #[instrument(name = "cfgmgr.update", skip(self, uuid))]
    async fn update(&self, uuid: &uuid::Uuid) -> Result<ConfigurationItem, ConfigurationManagerError> {
        debug!("updating configuration");
        let items = self.storage().list().await?;
        let item = items
            .iter()
            .find(|i| i.uuid == *uuid)
            .ok_or(ConfigurationStorageError::NotFound(*uuid))?;

        let content = self.fetcher().fetch_configuration(&item.source, Some(item)).await?;

        let mut updated = item.clone();
        updated.updated_at = time::OffsetDateTime::now_utc();
        self.storage().update(&updated, &content).await?;

        info!(bytes = content.len(), "configuration updated");

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

    pub fn storage_dir(&self) -> &Path {
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
