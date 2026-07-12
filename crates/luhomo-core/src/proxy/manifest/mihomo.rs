use std::io::Error;
use serde::de::DeserializeOwned;
use uuid::Bytes;
use crate::proxy::global_args::ProxyRunningArguments;
use crate::proxy::manifest::ProxyCoreManifest;

pub struct MihomoCoreManifest {}

impl MihomoCoreManifest {
    pub fn new() -> Self {
        Self {}
    }
}

impl ProxyCoreManifest for MihomoCoreManifest {
    async fn merge_runtime_manifest<C>(&self, config: impl AsRef<C>, args: &ProxyRunningArguments) -> Result<Bytes, Error>
    where
        C: DeserializeOwned
    {
        todo!()
    }
}