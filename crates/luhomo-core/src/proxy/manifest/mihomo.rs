use std::io::{Error, ErrorKind};

use crate::proxy::global_args::ProxyRunningArguments;
use crate::proxy::manifest::ProxyCoreManifest;
use bytes::Bytes;
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use tracing::debug;

pub struct MihomoCoreManifest {}

impl MihomoCoreManifest {
    pub fn new() -> Self {
        Self {}
    }
}

impl ProxyCoreManifest for MihomoCoreManifest {
    async fn merge_runtime_manifest(
        &self,
        config: impl AsRef<[u8]>,
        args: &ProxyRunningArguments,
    ) -> Result<Bytes, Error> {
        debug!(input_bytes = config.as_ref().len(), "merging mihomo runtime manifest");
        let mut config: Value = serde_yaml::from_slice(config.as_ref()).map_err(yaml_error)?;
        let manifest = config.as_mapping_mut().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                "mihomo configuration root must be a YAML mapping",
            )
        })?;

        // These are process-level settings, so they deliberately take precedence
        // over equally named values supplied by a subscription configuration.
        insert(manifest, "mixed-port", args.port)?;
        insert(manifest, "allow-lan", args.allow_lan)?;
        insert(manifest, "bind-address", &args.bind_address)?;
        insert(manifest, "mode", &args.mode)?;
        insert(manifest, "log-level", &args.log_level)?;
        insert(manifest, "ipv6", args.ipv6)?;

        // An omitted optional runtime argument leaves the subscription's setting
        // intact. This permits subscriptions to provide an API/UI configuration
        // unless the caller explicitly overrides it.
        if let Some(value) = &args.external_controller {
            insert(manifest, "external-controller", value)?;
        }
        if let Some(value) = &args.external_controller_unix {
            insert(manifest, "external-controller-unix", value)?;
        }
        if let Some(value) = &args.external_controller_pipe {
            insert(manifest, "external-controller-pipe", value)?;
        }
        if let Some(value) = &args.external_ui_name {
            insert(manifest, "external-ui-name", value)?;
        }
        if let Some(value) = &args.external_ui_url {
            insert(manifest, "external-ui-url", value)?;
        }

        let manifest = serde_yaml::to_string(&config).map(Bytes::from).map_err(yaml_error)?;
        debug!(output_bytes = manifest.len(), "merged mihomo runtime manifest");
        Ok(manifest)
    }
}

fn insert<T: Serialize>(manifest: &mut Mapping, key: &str, value: T) -> Result<(), Error> {
    let value = serde_yaml::to_value(value).map_err(yaml_error)?;
    manifest.insert(Value::String(key.to_owned()), value);
    Ok(())
}

fn yaml_error(error: serde_yaml::Error) -> Error {
    Error::new(ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn merges_runtime_arguments_and_preserves_other_configuration() {
        let config =
            b"mixed-port: 1234\nexternal-controller: 127.0.0.1:9090\nproxies:\n  - name: direct\n    type: direct\n";
        let args = ProxyRunningArguments::builder()
            .port(7891)
            .allow_lan(true)
            .bind_address("127.0.0.1".to_owned())
            .external_controller("127.0.0.1:19090".to_owned())
            .external_ui_name("dashboard".to_owned())
            .build();

        let output = MihomoCoreManifest::new()
            .merge_runtime_manifest(config, &args)
            .await
            .unwrap();
        let output: Value = serde_yaml::from_slice(&output).unwrap();

        assert_eq!(output["mixed-port"], 7891);
        assert_eq!(output["allow-lan"], true);
        assert_eq!(output["bind-address"], "127.0.0.1");
        assert_eq!(output["mode"], "rule");
        assert_eq!(output["log-level"], "info");
        assert_eq!(output["ipv6"], true);
        assert_eq!(output["external-controller"], "127.0.0.1:19090");
        assert_eq!(output["external-ui-name"], "dashboard");
        assert_eq!(output["proxies"][0]["name"], "direct");
    }

    #[tokio::test]
    async fn leaves_optional_subscription_settings_when_no_override_is_given() {
        let config = b"external-controller: 127.0.0.1:9090\nexternal-ui-url: https://example.test/ui.zip\n";

        let output = MihomoCoreManifest::new()
            .merge_runtime_manifest(config, &ProxyRunningArguments::default())
            .await
            .unwrap();
        let output: Value = serde_yaml::from_slice(&output).unwrap();

        assert_eq!(output["external-controller"], "127.0.0.1:9090");
        assert_eq!(output["external-ui-url"], "https://example.test/ui.zip");
    }

    #[tokio::test]
    async fn rejects_a_non_mapping_configuration_root() {
        let error = MihomoCoreManifest::new()
            .merge_runtime_manifest(b"- not a mapping\n", &ProxyRunningArguments::default())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }
}
