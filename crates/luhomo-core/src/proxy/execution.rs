use crate::config::models::ConfigurationItem;
use crate::proxy::core_type::ProxyCoreType;
use crate::proxy::global_args::ProxyRunningArguments;
use crate::proxy::manifest::ProxyCoreManifest;
use crate::proxy::status::ProxyCoreStatus;
use interprocess::local_socket::traits::tokio::Stream as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tracing::{info, warn};

/// 代理核心错误
#[derive(Error, Debug)]
pub enum ProxyCoreError {
    #[error("executable not found at {0}")]
    ExecutableNotFound(PathBuf),

    #[error("process is already running (pid: {0})")]
    AlreadyRunning(u32),

    #[error("process is not running")]
    NotRunning,

    #[error("process exited before its API became ready (exit code: {exit_code:?})")]
    ExitedBeforeReady { exit_code: Option<i32> },

    #[error("unknown process id")]
    UnknownPID,

    #[error("failed to spawn process: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error("failed to write runtime config: {0}")]
    ConfigError(#[source] std::io::Error),

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

enum MonitorEvent {
    ShutdownRequested,
    ChildExited(std::io::Result<ExitStatus>),
}

/// 已连接到代理核心 API 的流。
///
/// [`ProxyCoreExecution::launch`] 在启动完成时返回此连接；调用方可直接使用它发送
/// API 请求，而不必重新建立连接。
pub enum ProxyApiStream {
    /// Unix socket 或 Windows named pipe API 连接。
    Local(interprocess::local_socket::tokio::Stream),
    /// TCP API 连接。
    Tcp(tokio::net::TcpStream),
}

/// 代理核心进程管理器
///
/// 负责 proxy（比如 mihomo）内核的启动、监控、自动重启和关闭。
///
/// 在决定不使用 proxy 内核进程时，请显式调用 [`ProxyCoreExecution::shutdown`] 来关闭进程，
/// 否则在 [`ProxyCoreExecution`] 被 drop 时会尝试发送关闭信号，但无法保证进程已退出。
///
/// TODO: try Windows Job Object and Linux prctl(PR_SET_PDEATHSIG, SIGKILL)
///
/// # 使用示例
///
/// ```no_run
/// use luhomo_core::config::models::{ConfigurationItem, ConfigurationSource};
/// use luhomo_core::proxy::execution::ProxyCoreExecution;
/// use luhomo_core::proxy::global_args::ProxyRunningArguments;
/// use luhomo_core::proxy::{ProxyCoreStatus, ProxyCoreType};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let configuration_item = ConfigurationItem::builder()
///         .source(ConfigurationSource::local_file().path("config.yaml").call())
///         .display_name("example")
///         .build();
///     let config = b"proxies: []\nproxy-groups: []\nrules: []\n";
///     let args = ProxyRunningArguments::default();
///
///     let mut exec = ProxyCoreExecution::new(ProxyCoreType::Mihomo)
///         .executable("/path/to/mihomo")
///         .runtime_dir("/path/to/runtime/dir")
///         .auto_restart(true);
///
///     let _api_stream = exec.launch(&configuration_item, config, &args).await?;
///
///     // 订阅状态变化
///     let mut rx = exec.status_watcher();
///     let watcher = tokio::spawn(async move {
///         while rx.changed().await.is_ok() {
///             let status = rx.borrow().clone();
///             println!("status: {status:?}");
///             if matches!(status, ProxyCoreStatus::Stopped) {
///                 break;
///             }
///         }
///     });
///
///     // 应用结束使用内核时，显式关闭它并等待状态监听任务结束。
///     exec.shutdown().await?;
///     watcher.await?;
///     Ok(())
/// }
/// ```
pub struct ProxyCoreExecution {
    core_type: ProxyCoreType,

    executable: PathBuf,
    runtime_dir: PathBuf,
    auto_restart: bool,

    status_tx: watch::Sender<ProxyCoreStatus>,
    status_rx: watch::Receiver<ProxyCoreStatus>,

    shutdown_token: Option<tokio_util::sync::CancellationToken>,

    monitor_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ProxyCoreExecution {
    /// 创建新的执行实例，并按内核类型查找 proxy core 可执行文件。
    pub fn new(core_type: ProxyCoreType) -> Self {
        let (status_tx, status_rx) = watch::channel(ProxyCoreStatus::Idle);
        let executable = core_type.find_executable();
        let runtime_dir = std::env::temp_dir().join(core_type.as_ref());
        Self {
            core_type,
            executable,
            runtime_dir,
            auto_restart: true,
            status_tx,
            status_rx,
            shutdown_token: None,
            monitor_handle: None,
        }
    }

    /// 指定 proxy core 可执行文件路径（默认自动查找）
    pub fn executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = path.into();
        self
    }

    /// 指定代理核心的运行目录。
    ///
    /// 运行时 YAML 文件以及 `logs/{core}.stdout.log`、
    /// `logs/{core}.stderr.log` 都会写入该目录。
    pub fn runtime_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.runtime_dir = path.into();
        self
    }

    /// 是否在进程崩溃后自动重启（默认启用）
    pub fn auto_restart(mut self, enable: bool) -> Self {
        self.auto_restart = enable;
        self
    }
}

impl ProxyCoreExecution {
    /// 启动 proxy core 内核
    ///
    /// 将代理配置与 [`ProxyRunningArguments`] 合并后写入运行时 YAML 文件，
    /// 然后启动一个后台监控任务。监控任务利用 [`Child::wait`] 等待进程退出；
    /// 若退出并非由 [`ProxyCoreExecution::shutdown`] 触发，则在启用自动重启时尝试重新启动。
    ///
    /// 返回已就绪且已连接的 API 流。
    pub async fn launch(
        &mut self,
        configuration_item: &ConfigurationItem,
        config: impl AsRef<[u8]>,
        args: &ProxyRunningArguments,
    ) -> Result<ProxyApiStream, ProxyCoreError> {
        let running_pid = {
            let status = self.status_rx.borrow();

            match *status {
                ProxyCoreStatus::Running { pid } => Some(pid),
                _ => None,
            }
        };

        if let Some(pid) = running_pid {
            return Err(ProxyCoreError::AlreadyRunning(pid));
        }
        info!(core = self.core_type.as_ref(), "starting proxy core");
        if !tokio::fs::try_exists(&self.executable)
            .await
            .map_err(ProxyCoreError::ConfigError)?
        {
            return Err(ProxyCoreError::ExecutableNotFound(self.executable.clone()));
        }

        self.status_tx.send_replace(ProxyCoreStatus::Starting);

        self.prepare_runtime_dir().await?;

        // Write to a target config file
        let config_path = self
            .merge_and_write_runtime_cfg(configuration_item, config, args)
            .await?;

        let log_dir = self.prepare_core_log_dir().await?;

        // Spawn the child process
        let mut command = self
            .create_child_command(&config_path, &log_dir)
            .map_err(ProxyCoreError::OutputRedirectFailed)?;
        let mut child = command.spawn().map_err(ProxyCoreError::SpawnFailed)?;

        let pid = match child.id() {
            Some(pid) => pid,
            None => {
                let _ = child.start_kill();
                let _ = child.wait().await;

                self.status_tx
                    .send_replace(ProxyCoreStatus::Crashed { exit_code: None });

                return Err(ProxyCoreError::UnknownPID);
            }
        };
        info!(pid, "proxy core process spawned");

        let api_stream = tokio::select! {
            biased;
            exit = child.wait() => {
                let exit_code = exit.ok().and_then(|status| status.code());
                Err(ProxyCoreError::ExitedBeforeReady { exit_code })
            },
            result = self.ensure_api_ready(args, Duration::from_secs(3)) => result,
        };
        let api_stream = match api_stream {
            Ok(stream) => stream,
            Err(error) => {
                warn!(?error, "proxy core API did not become ready");
                let _ = child.start_kill();
                let _ = child.wait().await;

                match error {
                    ProxyCoreError::ExitedBeforeReady { exit_code } => {
                        self.status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code });
                    }
                    _ => {
                        self.status_tx.send_replace(ProxyCoreStatus::Idle);
                    }
                }

                return Err(error);
            }
        };

        let shutdown_token = tokio_util::sync::CancellationToken::new();
        self.shutdown_token = Some(shutdown_token.clone());

        // 启动监控任务
        let status_tx = self.status_tx.clone();
        let auto_restart = self.auto_restart;

        let handle = tokio::spawn(async move {
            loop {
                match Self::wait_for_event(&mut child, &shutdown_token).await {
                    // 手动退出
                    MonitorEvent::ShutdownRequested => {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        status_tx.send_replace(ProxyCoreStatus::Stopped);
                        break;
                    }
                    // 进程退出
                    MonitorEvent::ChildExited(result) => {
                        match result {
                            Ok(exit_status) => {
                                // 主动关闭（shutdown 已发出信号）
                                if shutdown_token.is_cancelled() {
                                    status_tx.send_replace(ProxyCoreStatus::Stopped);
                                    break;
                                }

                                let code = exit_status.code();
                                status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code: code });

                                if !auto_restart {
                                    break;
                                }

                                // 重启延迟期间继续响应关闭请求。
                                let shutdown_requested = tokio::select! {
                                    biased;
                                    _ = shutdown_token.cancelled() => {
                                        true
                                    }
                                    // TODO: use backoff strategy
                                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                        false
                                    }
                                };

                                if shutdown_requested || shutdown_token.is_cancelled() {
                                    status_tx.send_replace(ProxyCoreStatus::Stopped);
                                    break;
                                }

                                match command.spawn() {
                                    Ok(mut new_child) => {
                                        let new_pid = match new_child.id() {
                                            Some(pid) => pid,
                                            None => {
                                                let _ = new_child.start_kill();
                                                let _ = new_child.wait().await;

                                                // TODO: produce UnknownPID error
                                                status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code: None });

                                                break;
                                            }
                                        };

                                        child = new_child;
                                        status_tx.send_replace(ProxyCoreStatus::Running { pid: new_pid });
                                    }
                                    Err(_) => {
                                        status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code: None });
                                        break;
                                    }
                                }
                            }
                            Err(_) => {
                                // 通常这里不会被触发
                                if shutdown_token.is_cancelled() {
                                    status_tx.send_replace(ProxyCoreStatus::Stopped);
                                } else {
                                    status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code: None });
                                }
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.monitor_handle = Some(handle);
        self.status_tx.send_replace(ProxyCoreStatus::Running { pid });
        info!(pid, "proxy core API is ready");

        Ok(api_stream)
    }

    /// 获取当前运行状态
    pub fn status(&self) -> ProxyCoreStatus {
        self.status_rx.borrow().clone()
    }

    /// 获取状态变更的监听器
    ///
    /// 每次状态变化时返回，调用方可以轮询或使用 `changed()` 异步等待。
    pub fn status_watcher(&self) -> watch::Receiver<ProxyCoreStatus> {
        self.status_rx.clone()
    }

    /// 关闭代理核心
    ///
    /// 发送关闭信号给监控任务，由监控任务 kill 进程并清理。
    pub async fn shutdown(&mut self) -> Result<(), ProxyCoreError> {
        match *self.status_rx.borrow() {
            ProxyCoreStatus::Idle | ProxyCoreStatus::Stopped => {
                return Err(ProxyCoreError::NotRunning);
            }
            _ => {}
        }

        // 通知监控任务关闭
        if let Some(ref tx) = self.shutdown_token {
            info!("shutting down proxy core");
            tx.cancel();
        }

        // 等待监控任务结束
        if let Some(handle) = self.monitor_handle.take() {
            handle.await.map_err(ProxyCoreError::MonitorTaskFailed)?;
        }

        self.shutdown_token = None;
        info!("proxy core stopped");

        Ok(())
    }

    /// 验证 proxy core 的 API 端点是否已就绪
    ///
    /// 依次尝试 Unix socket（通过 interprocess）、Windows namedpipe（通过 interprocess）, TCP。
    /// 如果配置中未开启对应的管道，则跳过相应的检查。
    ///
    /// 如果没有配置任何 API 端点，则返回 [`ProxyCoreError::ApiEndpointNotConfigured`]。
    async fn ensure_api_ready(
        &self,
        args: &ProxyRunningArguments,
        timeout: Duration,
    ) -> Result<ProxyApiStream, ProxyCoreError> {
        // 使用 interprocess 连接本地 socket，验证对端是否就绪并返回连接。

        let mut name: Option<interprocess::local_socket::Name> = None;

        #[cfg(unix)]
        {
            if let Some(ref path) = args.external_controller_unix {
                use interprocess::local_socket::{GenericFilePath, ToFsName};

                let unix_name = path
                    .as_str()
                    .to_fs_name::<GenericFilePath>()
                    .map_err(ProxyCoreError::SocketChannelCheckFailed)?;
                name = Some(unix_name);
            }
        }
        #[cfg(windows)]
        {
            if let Some(ref pipe) = args.external_controller_pipe {
                use interprocess::local_socket::{GenericNamespaced, ToNsName};

                let windows_name = pipe
                    .as_str()
                    .to_ns_name::<GenericNamespaced>()
                    .map_err(ProxyCoreError::SocketChannelCheckFailed)?;
                name = Some(windows_name);
            }
        }

        // 如果配置了本地 socket 名称，则尝试连接
        if let Some(name) = name {
            let stream = tokio::time::timeout(timeout, async move {
                interprocess::local_socket::tokio::Stream::connect(name)
                    .await
                    .map_err(ProxyCoreError::SocketChannelCheckFailed)
            })
            .await
            .map_err(ProxyCoreError::SocketChannelCheckTimeout)??;

            return Ok(ProxyApiStream::Local(stream));
        }

        if let Some(ref addr) = args.external_controller {
            let stream = tokio::time::timeout(timeout, async move {
                tokio::net::TcpStream::connect(addr.as_str())
                    .await
                    .map_err(ProxyCoreError::SocketChannelCheckFailed)
            })
            .await
            .map_err(ProxyCoreError::SocketChannelCheckTimeout)??;

            return Ok(ProxyApiStream::Tcp(stream));
        }

        Err(ProxyCoreError::ApiEndpointNotConfigured)
    }

    /// 将传入的配置与运行参数合并，并写入临时 YAML 文件
    /// 文件写入路径为 `{runtime_dir}/{configuration uuid}.yaml`，并返回该路径。
    ///
    /// 调用方须先通过 [`Self::prepare_runtime_dir`] 准备运行目录。
    async fn merge_and_write_runtime_cfg(
        &self,
        item: &ConfigurationItem,
        config: impl AsRef<[u8]>,
        args: &ProxyRunningArguments,
    ) -> Result<PathBuf, ProxyCoreError> {
        let manifest = self.core_type.get_manifest();
        let build_args = manifest
            .merge_runtime_manifest(config, args)
            .await
            .map_err(ProxyCoreError::ConfigError)?;

        // Use the configuration item's UUID as the filename
        let target_path = self.runtime_dir.join(format!("{}.yaml", item.uuid));

        // Write the merged configuration to the target file (tokio)
        tokio::fs::write(&target_path, build_args)
            .await
            .map_err(ProxyCoreError::ConfigError)?;

        Ok(target_path)
    }

    /// 创建 proxy core 运行目录，用于保存运行时配置和其他运行产物。
    async fn prepare_runtime_dir(&self) -> Result<(), ProxyCoreError> {
        tokio::fs::create_dir_all(&self.runtime_dir)
            .await
            .map_err(ProxyCoreError::ConfigError)?;
        info!(
            runtime_dir = %self.runtime_dir.display(),
            "prepared proxy core runtime directory"
        );
        Ok(())
    }

    /// 创建 proxy core 输出日志目录。
    async fn prepare_core_log_dir(&self) -> Result<PathBuf, ProxyCoreError> {
        let log_dir = self.runtime_dir.join("logs");
        tokio::fs::create_dir_all(&log_dir)
            .await
            .map_err(ProxyCoreError::OutputRedirectFailed)?;
        info!(log_dir = %log_dir.display(), "prepared proxy core log directory");
        Ok(log_dir)
    }

    fn create_child_command(
        &self,
        config_path: impl AsRef<Path>,
        log_dir: impl AsRef<Path>,
    ) -> std::io::Result<Command> {
        let running_args = self
            .core_type
            .build_running_args(&self.runtime_dir, &config_path);
        let log_dir = log_dir.as_ref();

        let core_name = self.core_type.as_ref();
        let stdout_log = log_dir.join(format!("{core_name}.stdout.log"));
        let stderr_log = log_dir.join(format!("{core_name}.stderr.log"));
        let stdout = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_log)?;
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_log)?;
        info!(
            core = core_name,
            stdout_log = %stdout_log.display(),
            stderr_log = %stderr_log.display(),
            "redirecting proxy core output"
        );

        let mut cmd = Command::new(&self.executable);
        cmd.args(&running_args)
            .current_dir(&self.runtime_dir)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true);
        info!(command = ?cmd, "proxy core command dry run");
        Ok(cmd)
    }

    async fn wait_for_event(child: &mut Child, shutdown_token: &tokio_util::sync::CancellationToken) -> MonitorEvent {
        tokio::select! {
            biased;
            _ = shutdown_token.cancelled() => MonitorEvent::ShutdownRequested,
            result = child.wait() => MonitorEvent::ChildExited(result),
        }
    }
}

impl Drop for ProxyCoreExecution {
    fn drop(&mut self) {
        // notify the monitor task to shut down if it's still running
        if let Some(token) = &self.shutdown_token {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_api_ready_returns_the_connected_tcp_stream() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let accept = tokio::spawn(async move { listener.accept().await.unwrap() });
        let args = ProxyRunningArguments::builder().external_controller(address).build();

        let stream = ProxyCoreExecution::new(ProxyCoreType::Mihomo)
            .ensure_api_ready(&args, Duration::from_secs(1))
            .await
            .unwrap();

        assert!(matches!(stream, ProxyApiStream::Tcp(_)));
        accept.await.unwrap();
    }

    #[tokio::test]
    async fn ensure_api_ready_rejects_missing_api_endpoint() {
        let result = ProxyCoreExecution::new(ProxyCoreType::Mihomo)
            .ensure_api_ready(&ProxyRunningArguments::default(), Duration::from_secs(1))
            .await;

        assert!(matches!(result, Err(ProxyCoreError::ApiEndpointNotConfigured)));
    }
}
