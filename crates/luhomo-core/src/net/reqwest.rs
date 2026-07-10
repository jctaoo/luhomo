use bytes::Bytes;
use http::{HeaderMap, Response};

use crate::net::http::HttpClient;

impl From<reqwest::Client> for ReqwestClient {
    fn from(client: reqwest::Client) -> Self {
        ReqwestClient(client)
    }
}

pub struct ReqwestClient(pub reqwest::Client);

impl HttpClient for ReqwestClient {
    type Error = reqwest::Error;

    async fn get(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
    ) -> Result<Response<Bytes>, Self::Error> {
        let mut req = self.0.get(url);
        if let Some(h) = headers {
            req = req.headers(h);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let resp_headers = resp.headers().clone();
        let body = resp.bytes().await?;
        let mut builder = Response::builder().status(status);
        for (k, v) in resp_headers.iter() {
            builder = builder.header(k, v);
        }
        Ok(builder.body(body).expect("valid response"))
    }

    async fn post(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
        body: Bytes,
    ) -> Result<Response<Bytes>, Self::Error> {
        let mut req = self.0.post(url).body(body);
        if let Some(h) = headers {
            req = req.headers(h);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let resp_headers = resp.headers().clone();
        let body = resp.bytes().await?;
        let mut builder = Response::builder().status(status);
        for (k, v) in resp_headers.iter() {
            builder = builder.header(k, v);
        }
        Ok(builder.body(body).expect("valid response"))
    }
}
