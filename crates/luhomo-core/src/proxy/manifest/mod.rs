/// manifest module is for managing the runtime manifest of the proxy core.
pub mod mihomo;

use bytes::Bytes;
use serde::Serialize;

use crate::proxy::global_args::ProxyRunningArguments;

pub trait ProxyCoreManifest {
    async fn merge_runtime_manifest<C>(
        &self,
        config: impl AsRef<C>,
        args: &ProxyRunningArguments,
    ) -> Result<Bytes, std::io::Error>
    where
        C: Serialize;
}
