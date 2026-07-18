use std::path::PathBuf;
use thiserror::Error;

/// 代理核心错误
#[derive(Error, Debug)]
pub enum ProxyCoreError {
    #[error("executable not found at {0}")]
    ExecutableNotFound(PathBuf),

    #[error("process is already running (pid: {pid:?}) or starting")]
    AlreadyRunning { pid: Option<u32> },

    #[error("process is not running or stopping")]
    NotRunning,

    #[error("process exited before its API became ready (exit code: {exit_code:?})")]
    ExitedBeforeReady { exit_code: Option<i32> },

    #[error("failed to spawn process: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error("failed to read or write runtime config: {0}")]
    ConfigError(#[source] std::io::Error),

    #[error("failed to load configuration content: {0}")]
    ConfigSource(#[source] crate::config::storage::ConfigurationStorageError),

    #[error("failed to redirect proxy core output: {0}")]
    OutputRedirectFailed(#[source] std::io::Error),

    #[error("monitor task failed: {0}")]
    MonitorTaskFailed(#[source] tokio::task::JoinError),

    #[error("socket channel check failed: {0}")]
    SocketChannelCheckFailed(#[source] std::io::Error),

    #[error("socket channel check timed out")]
    SocketChannelCheckTimeout(#[source] tokio::time::error::Elapsed),

    #[error("no proxy core API endpoint is configured")]
    ApiEndpointNotConfigured,
}
