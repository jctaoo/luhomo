/// manifest module is for managing the runtime manifest of the proxy core.
pub mod mihomo;

use bytes::Bytes;
use std::future::Future;

use crate::proxy::global_args::ProxyRunningArguments;

pub trait ProxyCoreManifest {
    fn merge_runtime_manifest(
        &self,
        config: impl AsRef<[u8]> + Send,
        args: &ProxyRunningArguments,
    ) -> impl Future<Output = Result<Bytes, std::io::Error>> + Send;
}
