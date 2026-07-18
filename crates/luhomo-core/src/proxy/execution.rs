use crate::config::RuntimeConfigSource;
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
use tracing::{Instrument, debug, info, instrument, warn};

enum MonitorEvent {
    ShutdownRequested,
    ChildExited(std::io::Result<ExitStatus>),
}

/// 代理核心进程管理器。
///
/// 负责 proxy（比如 mihomo）内核的启动、监控、自动重启和关闭。启动配置会在构造时固化为
/// [`LaunchState`]，后台监控任务只共享该状态，不持有本对象本身。
/// 在决定不使用 proxy 内核进程时，请显式调用 [`ProxyCoreExecution::shutdown`]；
/// `Drop` 只会发送取消信号，无法等待子进程已退出。
///
/// TODO: try Windows Job Object and Linux prctl(PR_SET_PDEATHSIG, SIGKILL)
///
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
    #[instrument(name = "exec.new", skip(executable, runtime_dir), fields(core = %core_type.as_ref(), auto_restart))]
    pub fn new(
        /// 要启动的 proxy core 类型。
        core_type: ProxyCoreType,
        /// 指定 proxy core 可执行文件路径；默认按内核类型自动查找。
        #[builder(into)]
        executable: Option<PathBuf>,
        /// 指定代理核心的运行目录。
        ///
        /// 运行时 YAML 文件以及 `logs/{core}.stdout.log`、
        /// `logs/{core}.stderr.log` 都会写入该目录。
        #[builder(into)]
        runtime_dir: Option<PathBuf>,
        /// 是否在进程崩溃后自动重启，默认启用。
        #[builder(default = true)]
        auto_restart: bool,
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
    /// 启动 proxy core 内核。
    ///
    /// 通过 [`RuntimeConfigSource`] 按配置 UUID 加载最新内容，与 [`ProxyRunningArguments`]
    /// 合并后写入运行时 YAML 文件，然后启动一个后台监控任务。监控任务利用 [`Child::wait`]
    /// 等待进程退出；若退出并非由 [`ProxyCoreExecution::shutdown`] 触发，则在启用自动重启时
    /// 再次从 source 加载配置并尝试重新启动。
    ///
    /// 返回已就绪且已连接的 API 流。
    ///
    /// # 状态与错误
    ///
    /// 在进入 [`ProxyCoreStatus::Starting`] 之前的失败（例如加载配置源、可执行文件不存在、
    /// 准备运行目录或写入运行时 YAML）**不会**更新 status，仅通过 `Err` 返回
    /// [`ProxyCoreError`]。调用方应同时检查返回值与 [`Self::status`] / [`Self::status_watcher`]。
    /// 一旦已发布 `Starting`，后续启动失败会将 status 置为 `Failed` 或 `Crashed`。
    #[instrument(name = "exec.launch", skip(self, configuration_item, config_source, args), fields(core = %self.launch_state.core_type.as_ref()))]
    pub async fn launch<S>(
        &mut self,
        configuration_item: &ConfigurationItem,
        config_source: Arc<S>,
        args: &ProxyRunningArguments,
    ) -> Result<ProxyApiStream, ProxyCoreError>
    where
        S: RuntimeConfigSource + 'static,
    {
        info!(
            configuration = %configuration_item.display_name,
            uuid = %configuration_item.uuid,
            "launching proxy core"
        );
        let mut context = LaunchContext::builder()
            .core_type(self.launch_state.core_type.clone())
            .config_identity(configuration_item.uuid)
            .runtime_dir(self.launch_state.runtime_dir.clone())
            .running_args(args.clone())
            .auto_restart(self.auto_restart)
            .build();
        let config = config_source
            .load(&context.config_identity)
            .await
            .map_err(ProxyCoreError::config_source)?;
        let LaunchingInstance {
            mut child,
            pid,
            api_stream,
            generation,
        } = self.launch_state.launch_once(&mut context, config).await?;

        let shutdown_token = tokio_util::sync::CancellationToken::new();
        self.shutdown_token = Some(shutdown_token.clone());
        let status_tx = self.launch_state.status_tx.clone();
        let auto_restart = context.auto_restart;
        let launch_state = Arc::clone(&self.launch_state);

        info!(pid, generation, "starting proxy core monitor task");
        let monitor_span = tracing::Span::current();
        let handle = tokio::spawn(
            async move {
                let mut current_pid = pid;
                loop {
                    match Self::wait_for_event(&mut child, &shutdown_token).await {
                        MonitorEvent::ShutdownRequested => {
                            debug!(pid = current_pid, "proxy core monitor received shutdown request");
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
                                let exit_code = exit_status.code();
                                warn!(pid = current_pid, ?exit_code, "proxy core process exited unexpectedly");
                                status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code });
                                if !auto_restart {
                                    info!(pid = current_pid, "proxy core auto restart is disabled");
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
                                info!(
                                    pid = current_pid,
                                    next_attempt = context.current_attempts.saturating_add(1),
                                    "restarting proxy core after unexpected exit"
                                );
                                let config = tokio::select! {
                                    biased;
                                    _ = shutdown_token.cancelled() => {
                                        status_tx.send_replace(ProxyCoreStatus::Stopped);
                                        break;
                                    }
                                    result = config_source.load(&context.config_identity) => {
                                        match result {
                                            Ok(config) => config,
                                            Err(error) => {
                                                warn!(?error, "failed to load configuration for proxy core restart");
                                                status_tx.send_replace(ProxyCoreStatus::from(
                                                    ProxyCoreError::config_source(error),
                                                ));
                                                break;
                                            }
                                        }
                                    }
                                };
                                match launch_state.launch_once(&mut context, config).await {
                                    Ok(LaunchingInstance {
                                        child: new_child,
                                        pid,
                                        api_stream: _,
                                        generation,
                                    }) => {
                                        child = new_child;
                                        current_pid = pid;
                                        status_tx.send_replace(ProxyCoreStatus::Running { pid, generation });
                                        info!(pid, generation, "proxy core restart completed");
                                    }
                                    Err(error) => {
                                        if shutdown_token.is_cancelled() {
                                            status_tx.send_replace(ProxyCoreStatus::Stopped);
                                        } else {
                                            warn!(?error, "failed to restart proxy core");
                                            status_tx.send_replace(ProxyCoreStatus::from(error));
                                        }
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                status_tx.send_replace(if shutdown_token.is_cancelled() {
                                    ProxyCoreStatus::Stopped
                                } else {
                                    warn!(pid = current_pid, ?error, "failed to wait for proxy core process");
                                    ProxyCoreStatus::from(ProxyCoreError::spawn_failed(error))
                                });
                                break;
                            }
                        },
                    }
                }
            }
            .instrument(monitor_span),
        );

        self.launch_state
            .status_tx
            .send_replace(ProxyCoreStatus::Running { pid, generation });
        self.monitor_handle = Some(handle);
        Ok(api_stream)
    }

    /// 获取当前运行状态。
    pub fn status(&self) -> ProxyCoreStatus {
        self.launch_state.status_rx.borrow().clone()
    }

    /// 获取状态变更的监听器。
    ///
    /// 每次状态变化时返回，调用方可轮询或使用 `changed()` 异步等待。
    pub fn status_watcher(&self) -> watch::Receiver<ProxyCoreStatus> {
        self.launch_state.status_rx.clone()
    }

    /// 关闭代理核心。
    ///
    /// 发送关闭信号给监控任务，由监控任务 kill 进程并清理。
    #[instrument(name = "exec.shutdown", skip(self))]
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
            handle.await.map_err(ProxyCoreError::monitor_task_failed)?;
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
        // 若监控任务仍在运行，通知它关闭；Drop 无法异步等待清理完成。
        if let Some(token) = &self.shutdown_token {
            token.cancel();
        }
    }
}
