use bytes::Bytes;
use http::{HeaderMap, Response};
use serde::{de::DeserializeOwned, Serialize};

pub trait HttpClient: Send + Sync {
    type Error: std::fmt::Debug + Send + Sync;

    async fn get(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
    ) -> Result<Response<Bytes>, Self::Error>;

    async fn post(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
        body: Bytes,
    ) -> Result<Response<Bytes>, Self::Error>;

    async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
    ) -> Result<T, HttpClientErrorWrapper<Self::Error>> {
        let resp = self
            .get(url, headers)
            .await
            .map_err(HttpClientErrorWrapper::Client)?;
        let data =
            serde_json::from_slice(resp.body().as_ref()).map_err(HttpClientErrorWrapper::Json)?;
        Ok(data)
    }

    async fn post_json<T, B>(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
        body: &B,
    ) -> Result<T, HttpClientErrorWrapper<Self::Error>>
    where
        T: DeserializeOwned,
        B: Serialize + Sync,
    {
        let body = Bytes::from(serde_json::to_vec(body).map_err(HttpClientErrorWrapper::Json)?);
        let resp = self
            .post(url, headers, body)
            .await
            .map_err(HttpClientErrorWrapper::Client)?;
        let data =
            serde_json::from_slice(resp.body().as_ref()).map_err(HttpClientErrorWrapper::Json)?;
        Ok(data)
    }
}

#[derive(Debug)]
pub enum HttpClientErrorWrapper<E> {
    Client(E),
    Json(serde_json::Error),
}
