/// manifest module is for managing the runtime manifest of the proxy core.
pub mod mihomo;

use bytes::Bytes;

use crate::proxy::global_args::ProxyRunningArguments;

pub trait ProxyCoreManifest {
    async fn merge_runtime_manifest(
        &self,
        config: impl AsRef<[u8]>,
        args: &ProxyRunningArguments,
    ) -> Result<Bytes, std::io::Error>;
}
