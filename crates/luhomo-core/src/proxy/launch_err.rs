use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// 代理核心错误
#[derive(Error, Debug, Clone)]
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
    SpawnFailed(#[source] Arc<std::io::Error>),

    #[error("failed to read or write runtime config: {0}")]
    ConfigError(#[source] Arc<std::io::Error>),

    #[error("failed to load configuration content: {0}")]
    ConfigSource(#[source] Arc<crate::config::storage::ConfigurationStorageError>),

    #[error("failed to redirect proxy core output: {0}")]
    OutputRedirectFailed(#[source] Arc<std::io::Error>),

    #[error("monitor task failed: {0}")]
    MonitorTaskFailed(#[source] Arc<tokio::task::JoinError>),

    #[error("socket channel check failed: {0}")]
    SocketChannelCheckFailed(#[source] Arc<std::io::Error>),

    #[error("socket channel check timed out")]
    SocketChannelCheckTimeout(#[source] Arc<tokio::time::error::Elapsed>),

    #[error("no proxy core API endpoint is configured")]
    ApiEndpointNotConfigured,
}

impl ProxyCoreError {
    pub(crate) fn spawn_failed(error: std::io::Error) -> Self {
        Self::SpawnFailed(Arc::new(error))
    }

    pub(crate) fn config_error(error: std::io::Error) -> Self {
        Self::ConfigError(Arc::new(error))
    }

    pub(crate) fn config_source(error: crate::config::storage::ConfigurationStorageError) -> Self {
        Self::ConfigSource(Arc::new(error))
    }

    pub(crate) fn output_redirect_failed(error: std::io::Error) -> Self {
        Self::OutputRedirectFailed(Arc::new(error))
    }

    pub(crate) fn monitor_task_failed(error: tokio::task::JoinError) -> Self {
        Self::MonitorTaskFailed(Arc::new(error))
    }

    pub(crate) fn socket_channel_check_failed(error: std::io::Error) -> Self {
        Self::SocketChannelCheckFailed(Arc::new(error))
    }

    pub(crate) fn socket_channel_check_timeout(error: tokio::time::error::Elapsed) -> Self {
        Self::SocketChannelCheckTimeout(Arc::new(error))
    }
}
