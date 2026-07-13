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
        replace_or_insert(manifest, "mixed-port", args.port)?;
        replace_or_insert(manifest, "allow-lan", args.allow_lan)?;
        replace_or_insert(manifest, "bind-address", &args.bind_address)?;
        replace_or_insert(manifest, "mode", &args.mode)?;
        replace_or_insert(manifest, "log-level", &args.log_level)?;
        replace_or_insert(manifest, "ipv6", args.ipv6)?;

        // TUN is an application-level runtime capability. Override the fields
        // required for the desktop TUN mode, while preserving any additional
        // TUN options supplied by the subscription.
        inject_tun(manifest)?;
        inject_profile(manifest)?;

        // An omitted optional runtime argument leaves the subscription's setting
        // intact. This permits subscriptions to provide an API/UI configuration
        // unless the caller explicitly overrides it.
        if let Some(value) = &args.external_controller {
            replace_or_insert(manifest, "external-controller", value)?;
        }
        if let Some(value) = &args.external_controller_unix {
            replace_or_insert(manifest, "external-controller-unix", value)?;
        }
        if let Some(value) = &args.external_controller_pipe {
            replace_or_insert(manifest, "external-controller-pipe", value)?;
        }
        if let Some(value) = &args.external_ui_name {
            replace_or_insert(manifest, "external-ui-name", value)?;
        }
        if let Some(value) = &args.external_ui_url {
            replace_or_insert(manifest, "external-ui-url", value)?;
        }

        let manifest = serde_yaml::to_string(&config).map(Bytes::from).map_err(yaml_error)?;
        debug!(output_bytes = manifest.len(), "merged mihomo runtime manifest");
        Ok(manifest)
    }
}

fn replace_or_insert<T: Serialize>(
    manifest: &mut Mapping,
    key: &str,
    value: T,
) -> Result<(), Error> {
    let value = serde_yaml::to_value(value).map_err(yaml_error)?;
    manifest.insert(Value::String(key.to_owned()), value);
    Ok(())
}

fn inject_tun(manifest: &mut Mapping) -> Result<(), Error> {
    let key = Value::String("tun".to_owned());
    let tun = manifest
        .entry(key)
        .or_insert_with(|| Value::Mapping(Mapping::new()));

    let tun = tun.as_mapping_mut().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidData,
            "mihomo tun configuration must be a YAML mapping",
        )
    })?;

    replace_or_insert(tun, "enable", true)?;
    replace_or_insert(tun, "stack", "mixed")?;
    replace_or_insert(tun, "auto-route", true)?;
    replace_or_insert(tun, "auto-detect-interface", true)?;
    replace_or_insert(tun, "strict-route", true)?;
    replace_or_insert(
        tun,
        "dns-hijack",
        vec!["any:53", "tcp://any:53"],
    )?;

    Ok(())
}

fn inject_profile(manifest: &mut Mapping) -> Result<(), Error> {
    let key = Value::String("profile".to_owned());
    let profile = manifest
        .entry(key)
        .or_insert_with(|| Value::Mapping(Mapping::new()));

    let profile = profile.as_mapping_mut().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidData,
            "mihomo profile configuration must be a YAML mapping",
        )
    })?;

    replace_or_insert(profile, "store-selected", true)?;
    replace_or_insert(profile, "store-fake-ip", true)?;

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
    async fn injects_tun_and_preserves_additional_tun_settings() {
        let config = br#"
tun:
  enable: false
  stack: gvisor
  device: custom-tun
  mtu: 1400
"#;

        let output = MihomoCoreManifest::new()
            .merge_runtime_manifest(config, &ProxyRunningArguments::default())
            .await
            .unwrap();
        let output: Value = serde_yaml::from_slice(&output).unwrap();

        assert_eq!(output["tun"]["enable"], true);
        assert_eq!(output["tun"]["stack"], "mixed");
        assert_eq!(output["tun"]["auto-route"], true);
        assert_eq!(output["tun"]["auto-detect-interface"], true);
        assert_eq!(output["tun"]["strict-route"], true);
        assert_eq!(
            output["tun"]["dns-hijack"],
            serde_yaml::to_value(vec!["any:53", "tcp://any:53"]).unwrap()
        );
        assert_eq!(output["tun"]["device"], "custom-tun");
        assert_eq!(output["tun"]["mtu"], 1400);
    }

    #[tokio::test]
    async fn inserts_tun_when_subscription_has_no_tun_configuration() {
        let output = MihomoCoreManifest::new()
            .merge_runtime_manifest(b"proxies: []\n", &ProxyRunningArguments::default())
            .await
            .unwrap();
        let output: Value = serde_yaml::from_slice(&output).unwrap();

        assert_eq!(output["tun"]["enable"], true);
        assert_eq!(output["tun"]["stack"], "mixed");
    }

    #[tokio::test]
    async fn rejects_non_mapping_tun_configuration() {
        let error = MihomoCoreManifest::new()
            .merge_runtime_manifest(b"tun: false\n", &ProxyRunningArguments::default())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn injects_profile_defaults_and_preserves_additional_profile_settings() {
        let config = br#"
profile:
  store-selected: false
  store-fake-ip: false
  custom-setting: keep
"#;

        let output = MihomoCoreManifest::new()
            .merge_runtime_manifest(config, &ProxyRunningArguments::default())
            .await
            .unwrap();
        let output: Value = serde_yaml::from_slice(&output).unwrap();

        assert_eq!(output["profile"]["store-selected"], true);
        assert_eq!(output["profile"]["store-fake-ip"], true);
        assert_eq!(output["profile"]["custom-setting"], "keep");
    }

    #[tokio::test]
    async fn rejects_non_mapping_profile_configuration() {
        let error = MihomoCoreManifest::new()
            .merge_runtime_manifest(b"profile: false\n", &ProxyRunningArguments::default())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
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
