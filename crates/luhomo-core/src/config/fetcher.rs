use bytes::Bytes;
use http::{
    HeaderMap,
    header::{HeaderValue, USER_AGENT},
};
use thiserror::Error;
use tracing::{debug, info};

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
    #[error("invalid user-agent header: {0}")]
    InvalidUserAgent(#[source] http::header::InvalidHeaderValue),
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
                debug!(path, "reading local configuration");
                let content = tokio::fs::read(path).await?;
                info!(path, bytes = content.len(), "read local configuration");
                Ok(Bytes::from(content))
            }
            ConfigurationSource::RemoteUrl { url, user_agent, .. } => {
                let client = self.get_client();
                let headers = user_agent_headers(user_agent)?;
                info!(
                    scheme = url.scheme(),
                    host = url.host_str(),
                    has_custom_user_agent = user_agent.is_some(),
                    "fetching remote configuration"
                );
                let resp = client
                    .get(url.as_str(), headers)
                    .await
                    .map_err(|e| ConfigurationFetcherError::Http(format!("{e:?}")))?;

                let status = resp.status();
                if !status.is_success() {
                    return Err(ConfigurationFetcherError::BadResponse(status));
                }

                let body = resp.into_body();
                info!(status = %status, bytes = body.len(), "fetched remote configuration");
                Ok(body)
            }
        }
    }
}

fn user_agent_headers(user_agent: &Option<String>) -> Result<Option<HeaderMap>, ConfigurationFetcherError> {
    let Some(user_agent) = user_agent else {
        return Ok(None);
    };

    let value = HeaderValue::from_str(user_agent).map_err(ConfigurationFetcherError::InvalidUserAgent)?;
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, value);
    Ok(Some(headers))
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
