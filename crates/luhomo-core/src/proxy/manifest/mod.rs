/// manifest module is for managing the runtime manifest of the proxy core.
pub mod mihomo;

use bytes::Bytes;
use std::future::Future;

use crate::proxy::global_args::ProxyRunningArguments;

pub trait ProxyCoreManifest {
    /// Merge the source configuration with the runtime arguments.
    ///
    /// Runtime arguments are authoritative: every setting represented by
    /// [`ProxyRunningArguments`] must match `args` in the resulting manifest.
    /// In particular, an omitted optional argument clears the corresponding
    /// setting inherited from the source configuration.
    fn merge_runtime_manifest(
        &self,
        config: impl AsRef<[u8]> + Send,
        args: &ProxyRunningArguments,
    ) -> impl Future<Output = Result<Bytes, std::io::Error>> + Send;
}
