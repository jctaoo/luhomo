use std::path::{Path, PathBuf};
use crate::proxy::manifest::ProxyCoreManifest;

/// This enum represents the different types of proxy cores that can be used in the application.
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

    pub fn build_running_args(&self, target_cfg_file: impl AsRef<Path>) -> Vec<String> {
        match self {
            ProxyCoreType::Mihomo => vec![
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
        if p.exists() {
            return p;
        }
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let p = dir.join(executable_name);
        if p.exists() {
            return p;
        }
    }

    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let root = PathBuf::from(&manifest)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        for profile in &["debug", "release"] {
            let p = root.join("target").join(profile).join(executable_name);
            if p.exists() {
                return p;
            }
        }
    }

    PathBuf::from(executable_name)
}
