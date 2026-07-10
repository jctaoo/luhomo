use bytes::Bytes;
use thiserror::Error;

use crate::config::models::{self, ConfigurationSource};
use crate::net::http::HttpClient;

#[derive(Debug, Error)]
pub enum ConfigurationFetcherError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bad response: status {0}")]
    BadResponse(http::StatusCode),
}

pub trait ConfigurationFetcher {
    type Client: HttpClient;

    fn get_client(&self) -> &Self::Client;

    async fn fetch_configuration(
        &self,
        source: &ConfigurationSource,
        _old: Option<&models::ConfigurationItem>,
    ) -> Result<Bytes, ConfigurationFetcherError> {
        match source {
            ConfigurationSource::LocalFile(path) => {
                let content = tokio::fs::read(path).await?;
                Ok(Bytes::from(content))
            }
            ConfigurationSource::RemoteUrl { url, .. } => {
                let client = self.get_client();
                let resp = client
                    .get(url.as_str(), None)
                    .await
                    .map_err(|e| ConfigurationFetcherError::Http(format!("{e:?}")))?;

                let status = resp.status();
                if !status.is_success() {
                    return Err(ConfigurationFetcherError::BadResponse(status));
                }

                Ok(resp.into_body())
            }
        }
    }
}

#[cfg(feature = "reqwest")]
pub struct RemoteConfigurationFetcher {
    client: crate::net::reqwest::ReqwestClient,
}

#[cfg(feature = "reqwest")]
impl RemoteConfigurationFetcher {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client: crate::net::reqwest::ReqwestClient(client),
        }
    }
}

#[cfg(feature = "reqwest")]
impl ConfigurationFetcher for RemoteConfigurationFetcher {
    type Client = crate::net::reqwest::ReqwestClient;

    fn get_client(&self) -> &Self::Client {
        &self.client
    }
}
