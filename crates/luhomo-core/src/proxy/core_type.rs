use crate::proxy::manifest::ProxyCoreManifest;
use std::path::{Path, PathBuf};
use tracing::{debug, trace};

/// This enum represents the different types of proxy cores that can be used in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyCoreType {
    /// https://wiki.metacubex.one/
    Mihomo,
}

impl AsRef<str> for ProxyCoreType {
    fn as_ref(&self) -> &str {
        match self {
            ProxyCoreType::Mihomo => "mihomo",
        }
    }
}

impl ProxyCoreType {
    pub fn find_executable(&self) -> PathBuf {
        match self {
            ProxyCoreType::Mihomo => find_mihomo_executable(),
        }
    }

    pub fn get_manifest(&self) -> impl ProxyCoreManifest {
        match self {
            ProxyCoreType::Mihomo => crate::proxy::manifest::mihomo::MihomoCoreManifest::new(),
        }
    }

    /// 构建 proxy core 的运行参数。
    ///
    /// Mihomo 的 `-d` 指定运行目录，`-f` 指定配置文件。
    pub fn build_running_args(
        &self,
        core_running_dir: impl AsRef<Path>,
        target_cfg_file: impl AsRef<Path>,
    ) -> Vec<String> {
        match self {
            ProxyCoreType::Mihomo => vec![
                "-d".to_string(),
                core_running_dir.as_ref().to_string_lossy().to_string(),
                "-f".to_string(),
                target_cfg_file.as_ref().to_string_lossy().to_string(),
            ],
        }
    }
}

/// 查找 mihomo 可执行文件
///
/// 按优先级搜索：
/// 1. 环境变量 `MIHOMO_PATH`
/// 2. 当前可执行文件同目录（`./mihomo` / `./mihomo.exe`）
/// 3. Cargo 构建输出目录（`target/debug/` / `target/release/`）
/// 4. 仅返回文件名，依赖系统 `PATH`
fn find_mihomo_executable() -> PathBuf {
    #[cfg(windows)]
    let executable_name = "mihomo.exe";
    #[cfg(not(windows))]
    let executable_name = "mihomo";

    if let Ok(path) = std::env::var("MIHOMO_PATH") {
        let p = PathBuf::from(path);
        trace!(path = %p.display(), source = "MIHOMO_PATH", "checking mihomo executable");
        if p.exists() {
            debug!(path = %p.display(), source = "MIHOMO_PATH", "found mihomo executable");
            return p;
        }
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let p = dir.join(executable_name);
        trace!(path = %p.display(), source = "current executable directory", "checking mihomo executable");
        if p.exists() {
            debug!(path = %p.display(), source = "current executable directory", "found mihomo executable");
            return p;
        }
    }

    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = PathBuf::from(manifest);
        if let Some(root) = find_cargo_workspace_root(&manifest_dir) {
            for profile in &["debug", "release"] {
                let p = root.join("target").join(profile).join(executable_name);
                trace!(path = %p.display(), source = "Cargo target directory", "checking mihomo executable");
                if p.exists() {
                    debug!(path = %p.display(), source = "Cargo target directory", "found mihomo executable");
                    return p;
                }
            }
        }
    }

    let fallback = PathBuf::from(executable_name);
    debug!(path = %fallback.display(), source = "PATH", "using mihomo executable fallback");
    fallback
}

fn find_cargo_workspace_root(manifest_dir: &Path) -> Option<&Path> {
    manifest_dir.ancestors().find(|directory| {
        std::fs::read_to_string(directory.join("Cargo.toml")).is_ok_and(|cargo_toml| cargo_toml.contains("[workspace]"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mihomo_running_args_include_runtime_dir_and_config_file() {
        let args = ProxyCoreType::Mihomo.build_running_args("runtime", "runtime/config.yaml");

        assert_eq!(args, ["-d", "runtime", "-f", "runtime/config.yaml"]);
    }
}
