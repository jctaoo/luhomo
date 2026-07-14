use crate::config::models::ConfigurationItem;
use crate::proxy::core_type::ProxyCoreType;
use crate::proxy::global_args::ProxyRunningArguments;
use crate::proxy::launch_err::ProxyCoreError;
use crate::proxy::launch_state::LaunchState;
use crate::proxy::launch_status::{LaunchContext, LaunchingInstance, ProxyApiStream, ProxyCoreStatus};
use bon::bon;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::watch;
use tracing::{debug, info, warn};

enum MonitorEvent {
    ShutdownRequested,
    ChildExited(std::io::Result<ExitStatus>),
}

/// 代理核心进程管理器。
///
/// 负责 proxy 内核的启动、监控、自动重启和关闭。启动配置会在构造时固化为
/// [`LaunchState`]，后台监控任务只共享该状态，不持有本对象本身。
/// 在决定不使用 proxy 内核进程时，请显式调用 [`ProxyCoreExecution::shutdown`]；
/// `Drop` 只会发送取消信号，无法等待子进程已退出。
///
/// TODO: try Windows Job Object and Linux prctl(PR_SET_PDEATHSIG, SIGKILL)
///
/// # 使用示例
///
/// ```no_run
/// use luhomo_core::config::models::{ConfigurationItem, ConfigurationSource};
/// use luhomo_core::proxy::execution::ProxyCoreExecution;
/// use luhomo_core::proxy::global_args::ProxyRunningArguments;
/// use luhomo_core::proxy::core_type::ProxyCoreType;
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let item = ConfigurationItem::builder()
///     .source(ConfigurationSource::local_file().path("config.yaml").call())
///     .display_name("example")
///     .build();
/// let mut exec = ProxyCoreExecution::builder()
///     .core_type(ProxyCoreType::Mihomo)
///     .runtime_dir("/path/to/runtime")
///     .build();
/// let _api = exec.launch(&item, b"proxies: []\n", &ProxyRunningArguments::default()).await?;
/// exec.shutdown().await?;
/// # Ok(())
/// # }
/// ```
pub struct ProxyCoreExecution {
    launch_state: Arc<LaunchState>,
    auto_restart: bool,
    shutdown_token: Option<tokio_util::sync::CancellationToken>,
    monitor_handle: Option<tokio::task::JoinHandle<()>>,
}

#[bon]
impl ProxyCoreExecution {
    /// 创建新的执行实例，并按内核类型查找 proxy core 可执行文件。
    #[builder]
    pub fn new(
        core_type: ProxyCoreType,
        #[builder(into)] executable: Option<PathBuf>,
        #[builder(into)] runtime_dir: Option<PathBuf>,
        #[builder(default = true)] auto_restart: bool,
    ) -> Self {
        let (status_tx, status_rx) = watch::channel(ProxyCoreStatus::Stopped);
        let executable = executable.unwrap_or_else(|| core_type.find_executable());
        let runtime_dir = runtime_dir.unwrap_or_else(|| std::env::temp_dir().join(core_type.as_ref()));
        Self {
            launch_state: Arc::new(LaunchState {
                core_type,
                executable,
                runtime_dir,
                status_tx,
                status_rx,
            }),
            auto_restart,
            shutdown_token: None,
            monitor_handle: None,
        }
    }
}

impl ProxyCoreExecution {
    /// 启动 proxy core 内核并开始后台监控。
    pub async fn launch(
        &mut self,
        configuration_item: &ConfigurationItem,
        config: impl AsRef<[u8]>,
        args: &ProxyRunningArguments,
    ) -> Result<ProxyApiStream, ProxyCoreError> {
        let config = config.as_ref().to_vec();
        let mut context = LaunchContext::builder()
            .core_type(self.launch_state.core_type.clone())
            .config_identity(configuration_item.uuid)
            .runtime_dir(self.launch_state.runtime_dir.clone())
            .running_args(args.clone())
            .auto_restart(self.auto_restart)
            .build();
        let LaunchingInstance {
            mut child,
            pid,
            api_stream,
            generation,
        } = self
            .launch_state
            .launch_once(&mut context, Some(config.clone()))
            .await?;

        let shutdown_token = tokio_util::sync::CancellationToken::new();
        self.shutdown_token = Some(shutdown_token.clone());
        let status_tx = self.launch_state.status_tx.clone();
        let auto_restart = context.auto_restart;
        let launch_state = Arc::clone(&self.launch_state);

        info!(pid, generation, "starting proxy core monitor task");
        let handle = tokio::spawn(async move {
            loop {
                match Self::wait_for_event(&mut child, &shutdown_token).await {
                    MonitorEvent::ShutdownRequested => {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        status_tx.send_replace(ProxyCoreStatus::Stopped);
                        break;
                    }
                    MonitorEvent::ChildExited(result) => match result {
                        Ok(exit_status) => {
                            if shutdown_token.is_cancelled() {
                                status_tx.send_replace(ProxyCoreStatus::Stopped);
                                break;
                            }
                            status_tx.send_replace(ProxyCoreStatus::Crashed {
                                exit_code: exit_status.code(),
                            });
                            if !auto_restart {
                                break;
                            }
                            let shutdown_requested = tokio::select! {
                                biased;
                                _ = shutdown_token.cancelled() => true,
                                _ = tokio::time::sleep(Duration::from_secs(1)) => false,
                            };
                            if shutdown_requested || shutdown_token.is_cancelled() {
                                status_tx.send_replace(ProxyCoreStatus::Stopped);
                                break;
                            }
                            match launch_state.launch_once(&mut context, Some(config.clone())).await {
                                Ok(LaunchingInstance {
                                    child: new_child,
                                    pid,
                                    api_stream: _,
                                    generation,
                                }) => {
                                    child = new_child;
                                    status_tx.send_replace(ProxyCoreStatus::Running { pid, generation });
                                    info!(pid, generation, "proxy core restarted and API is ready");
                                }
                                Err(error) => {
                                    if shutdown_token.is_cancelled() {
                                        status_tx.send_replace(ProxyCoreStatus::Stopped);
                                    } else {
                                        warn!(?error, "failed to restart proxy core");
                                        status_tx.send_replace(ProxyCoreStatus::Failed {
                                            message: error.to_string(),
                                        });
                                    }
                                    break;
                                }
                            }
                        }
                        Err(_) => {
                            status_tx.send_replace(if shutdown_token.is_cancelled() {
                                ProxyCoreStatus::Stopped
                            } else {
                                ProxyCoreStatus::Crashed { exit_code: None }
                            });
                            break;
                        }
                    },
                }
            }
        });

        self.launch_state
            .status_tx
            .send_replace(ProxyCoreStatus::Running { pid, generation });
        self.monitor_handle = Some(handle);
        info!(pid, generation, "proxy core API is ready");
        Ok(api_stream)
    }

    pub fn status(&self) -> ProxyCoreStatus {
        self.launch_state.status_rx.borrow().clone()
    }

    pub fn status_watcher(&self) -> watch::Receiver<ProxyCoreStatus> {
        self.launch_state.status_rx.clone()
    }

    /// 发送关闭信号并等待监控任务清理子进程。
    pub async fn shutdown(&mut self) -> Result<(), ProxyCoreError> {
        match *self.launch_state.status_rx.borrow() {
            ProxyCoreStatus::Stopping { .. } | ProxyCoreStatus::Stopped => return Err(ProxyCoreError::NotRunning),
            _ => {}
        }
        debug!(status = ?*self.launch_state.status_rx.borrow(), "attempts to shut down proxy core");
        self.launch_state
            .status_tx
            .send_replace(ProxyCoreStatus::Stopping { restarting: false });
        if let Some(token) = &self.shutdown_token {
            info!("shutting down proxy core");
            token.cancel();
        }
        if let Some(handle) = self.monitor_handle.take() {
            handle.await.map_err(ProxyCoreError::MonitorTaskFailed)?;
        }
        self.shutdown_token = None;
        info!("proxy core stopped");
        Ok(())
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
        if let Some(token) = &self.shutdown_token {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_applies_launch_configuration_before_state_is_shared() {
        let execution = ProxyCoreExecution::builder()
            .core_type(ProxyCoreType::Mihomo)
            .executable("custom-mihomo")
            .runtime_dir("custom-runtime")
            .auto_restart(false)
            .build();
        assert_eq!(execution.launch_state.executable, PathBuf::from("custom-mihomo"));
        assert_eq!(execution.launch_state.runtime_dir, PathBuf::from("custom-runtime"));
        assert!(!execution.auto_restart);
    }
}
