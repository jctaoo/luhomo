use std::collections::HashMap;

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum HttpClientError {
    Network(String),
    Json(serde_json::Error),
}

impl std::fmt::Display for HttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::Json(e) => write!(f, "json error: {e}"),
        }
    }
}

impl std::error::Error for HttpClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str) -> Result<HttpResponse, HttpClientError>;
    fn post(&self, url: &str, body: &[u8]) -> Result<HttpResponse, HttpClientError>;

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, HttpClientError> {
        let resp = self.get(url)?;
        serde_json::from_slice(&resp.body).map_err(HttpClientError::Json)
    }

    fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, HttpClientError> {
        let body = serde_json::to_vec(body).map_err(HttpClientError::Json)?;
        let resp = self.post(url, &body)?;
        serde_json::from_slice(&resp.body).map_err(HttpClientError::Json)
    }
}
