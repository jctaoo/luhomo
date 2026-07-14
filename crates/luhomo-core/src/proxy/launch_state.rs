use crate::proxy::core_type::ProxyCoreType;
use crate::proxy::global_args::ProxyRunningArguments;
use crate::proxy::launch_err::ProxyCoreError;
use crate::proxy::launch_status::{LaunchContext, LaunchingInstance, ProxyApiStream, ProxyCoreStatus};
use crate::proxy::manifest::ProxyCoreManifest;
use crate::utils;
use interprocess::local_socket::traits::tokio::Stream as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tracing::{info, trace, warn};

/// Immutable launch configuration and shared status channels used by the
/// execution owner and its monitor task.
pub(crate) struct LaunchState {
    pub(crate) core_type: ProxyCoreType,
    pub(crate) executable: PathBuf,
    pub(crate) runtime_dir: PathBuf,
    pub(crate) status_tx: watch::Sender<ProxyCoreStatus>,
    pub(crate) status_rx: watch::Receiver<ProxyCoreStatus>,
}

impl LaunchState {
    pub(crate) async fn launch_once(
        &self,
        context: &mut LaunchContext,
        config: Option<impl AsRef<[u8]> + Send>,
    ) -> Result<LaunchingInstance, ProxyCoreError> {
        match *self.status_rx.borrow() {
            ProxyCoreStatus::Running { pid, .. } => return Err(ProxyCoreError::AlreadyRunning { pid: Some(pid) }),
            ProxyCoreStatus::Starting { .. } => return Err(ProxyCoreError::AlreadyRunning { pid: None }),
            ProxyCoreStatus::Stopping { .. } => {
                warn!("proxy core is stopping, cannot launch a new instance");
                return Err(ProxyCoreError::AlreadyRunning { pid: None });
            }
            _ => {}
        }

        if !tokio::fs::try_exists(&self.executable)
            .await
            .map_err(ProxyCoreError::ConfigError)?
        {
            return Err(ProxyCoreError::ExecutableNotFound(self.executable.clone()));
        }

        self.prepare_runtime_dir().await?;
        let log_dir = self.prepare_core_log_dir().await?;
        let runtime_config_path = runtime_config_filepath(&context.runtime_dir, context.config_identity);
        let mut need_regenerate_config = true;

        if let Some(expected_hash) = &context.core_manifest_hash {
            let config_hash = utils::hash::file_sha256(&runtime_config_path)
                .await
                .map_err(ProxyCoreError::ConfigError)?;
            if config_hash.as_str() != expected_hash {
                trace!(runtime_config_path = %runtime_config_path.display(), expected_hash = %expected_hash, actual_hash = %config_hash, "runtime config hash mismatch");
                info!(runtime_config_path = %runtime_config_path.display(), "runtime config hash mismatch, regenerating config");
            } else {
                trace!(runtime_config_path = %runtime_config_path.display(), expected_hash = %expected_hash, "runtime config hash matches, skipping regeneration");
                info!(runtime_config_path = %runtime_config_path.display(), "runtime config hash matches, skipping regeneration");
                need_regenerate_config = false;
            }
        }

        if need_regenerate_config {
            let config = config.ok_or(ProxyCoreError::NoConfigBytes)?;
            self.merge_and_write_runtime_cfg(config, &context.running_args, &runtime_config_path)
                .await?;
            context.core_manifest_hash = Some(
                utils::hash::file_sha256(&runtime_config_path)
                    .await
                    .map_err(ProxyCoreError::ConfigError)?,
            );
        }

        info!(core = self.core_type.as_ref(), "starting proxy core");
        context.current_attempts = context.current_attempts.saturating_add(1);
        let attempt = context.current_attempts;
        self.status_tx.send_replace(ProxyCoreStatus::Starting { attempt });

        let mut command = self
            .create_child_command(&runtime_config_path, &log_dir)
            .map_err(ProxyCoreError::OutputRedirectFailed)?;
        let mut child = command.spawn().map_err(ProxyCoreError::SpawnFailed)?;

        let exit_child_immediately = async move |mut child: Child, extend_err: Option<ProxyCoreError>| {
            info!("child process start to exit immediately after spawn");
            match child.wait().await {
                Ok(exit_status) => {
                    let exit_code = exit_status.code();
                    if let Some(err) = extend_err {
                        warn!(?err, exit_code, "child process exited immediately after spawn");
                        self.status_tx.send_replace(ProxyCoreStatus::Failed {
                            message: err.to_string(),
                        });
                        Err(err)
                    } else {
                        warn!(exit_code, "child process exited immediately after spawn");
                        self.status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code });
                        Err(ProxyCoreError::ExitedBeforeReady { exit_code })
                    }
                }
                Err(err) => {
                    warn!(?err, "failed to wait for child process exit status");
                    self.status_tx.send_replace(ProxyCoreStatus::Failed {
                        message: format!("failed to wait for child process exit status: {err}"),
                    });
                    Err(ProxyCoreError::SpawnFailed(err))
                }
            }
        };

        let pid = match child.id() {
            Some(pid) => pid,
            None => {
                let _ = child.start_kill();
                return exit_child_immediately(child, None).await;
            }
        };
        info!(pid, "proxy core process spawned");

        let api_stream = tokio::select! {
            biased;
            exit = child.wait() => Err(ProxyCoreError::ExitedBeforeReady {
                exit_code: exit.ok().and_then(|status| status.code()),
            }),
            result = self.ensure_api_ready(&context.running_args, Duration::from_secs(3)) => result,
        };
        let api_stream = match api_stream {
            Ok(stream) => stream,
            Err(error) => {
                warn!(?error, "proxy core API did not become ready");
                if let ProxyCoreError::ExitedBeforeReady { exit_code } = error {
                    self.status_tx.send_replace(ProxyCoreStatus::Crashed { exit_code });
                    return Err(error);
                }
                let _ = child.start_kill();
                return exit_child_immediately(child, Some(error)).await;
            }
        };

        info!(pid, "proxy core API is ready and stream is ready for use");
        Ok(LaunchingInstance {
            child,
            pid,
            api_stream,
            generation: attempt,
        })
    }

    async fn ensure_api_ready(
        &self,
        args: &ProxyRunningArguments,
        timeout: Duration,
    ) -> Result<ProxyApiStream, ProxyCoreError> {
        let mut name: Option<interprocess::local_socket::Name> = None;
        #[cfg(unix)]
        if let Some(path) = &args.external_controller_unix {
            use interprocess::local_socket::{GenericFilePath, ToFsName};
            name = Some(
                path.as_str()
                    .to_fs_name::<GenericFilePath>()
                    .map_err(ProxyCoreError::SocketChannelCheckFailed)?,
            );
        }
        #[cfg(windows)]
        if let Some(pipe) = &args.external_controller_pipe {
            use interprocess::local_socket::{GenericNamespaced, ToNsName};
            name = Some(
                pipe.as_str()
                    .to_ns_name::<GenericNamespaced>()
                    .map_err(ProxyCoreError::SocketChannelCheckFailed)?,
            );
        }
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
        if let Some(addr) = &args.external_controller {
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

    async fn merge_and_write_runtime_cfg(
        &self,
        config: impl AsRef<[u8]> + Send,
        args: &ProxyRunningArguments,
        target_path: impl AsRef<Path>,
    ) -> Result<(), ProxyCoreError> {
        let build_args = self
            .core_type
            .get_manifest()
            .merge_runtime_manifest(config, args)
            .await
            .map_err(ProxyCoreError::ConfigError)?;
        tokio::fs::write(&target_path, build_args)
            .await
            .map_err(ProxyCoreError::ConfigError)?;
        info!(target_path = %target_path.as_ref().display(), "wrote merged runtime configuration file");
        Ok(())
    }

    async fn prepare_runtime_dir(&self) -> Result<(), ProxyCoreError> {
        if tokio::fs::try_exists(&self.runtime_dir)
            .await
            .map_err(ProxyCoreError::ConfigError)?
        {
            return Ok(());
        }
        tokio::fs::create_dir_all(&self.runtime_dir)
            .await
            .map_err(ProxyCoreError::ConfigError)?;
        info!(runtime_dir = %self.runtime_dir.display(), "prepared proxy core runtime directory");
        Ok(())
    }

    async fn prepare_core_log_dir(&self) -> Result<PathBuf, ProxyCoreError> {
        let log_dir = self.runtime_dir.join("logs");
        if tokio::fs::try_exists(&log_dir)
            .await
            .map_err(ProxyCoreError::OutputRedirectFailed)?
        {
            return Ok(log_dir);
        }
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
        let running_args = self.core_type.build_running_args(&self.runtime_dir, &config_path);
        let core_name = self.core_type.as_ref();
        let stdout_log = log_dir.as_ref().join(format!("{core_name}.stdout.log"));
        let stderr_log = log_dir.as_ref().join(format!("{core_name}.stderr.log"));
        let stdout = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_log)?;
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_log)?;
        info!(core = core_name, stdout_log = %stdout_log.display(), stderr_log = %stderr_log.display(), "redirecting proxy core output");
        let mut command = Command::new(&self.executable);
        command
            .args(&running_args)
            .current_dir(&self.runtime_dir)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true);
        info!(command = ?command, "proxy core command dry run");
        Ok(command)
    }
}

fn runtime_config_filepath(runtime_dir: impl AsRef<Path>, config_identity: uuid::Uuid) -> PathBuf {
    runtime_dir.as_ref().join(format!("{config_identity}.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> LaunchState {
        let (status_tx, status_rx) = watch::channel(ProxyCoreStatus::Stopped);
        LaunchState {
            core_type: ProxyCoreType::Mihomo,
            executable: PathBuf::from("mihomo"),
            runtime_dir: std::env::temp_dir(),
            status_tx,
            status_rx,
        }
    }

    #[tokio::test]
    async fn ensure_api_ready_returns_the_connected_tcp_stream() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let accept = tokio::spawn(async move { listener.accept().await.unwrap() });
        let args = ProxyRunningArguments::builder().external_controller(address).build();
        let stream = state().ensure_api_ready(&args, Duration::from_secs(1)).await.unwrap();
        assert!(matches!(stream, ProxyApiStream::Tcp(_)));
        accept.await.unwrap();
    }

    #[tokio::test]
    async fn ensure_api_ready_rejects_missing_api_endpoint() {
        let result = state()
            .ensure_api_ready(&ProxyRunningArguments::default(), Duration::from_secs(1))
            .await;
        assert!(matches!(result, Err(ProxyCoreError::ApiEndpointNotConfigured)));
    }
}
