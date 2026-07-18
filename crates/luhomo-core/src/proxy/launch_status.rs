use std::path::PathBuf;
use std::sync::Arc;

use bon::Builder;

use crate::proxy::core_type::ProxyCoreType;
use crate::proxy::global_args::ProxyRunningArguments;
use crate::proxy::launch_err::ProxyCoreError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Builder)]
pub struct LaunchContext {
    pub core_type: ProxyCoreType,

    /// 运行 proxy core 传入的用户配置 id ([`ConfigurationItem::uuid`])，用于生成运行时 YAML 文件名。
    pub config_identity: uuid::Uuid,

    #[builder(into)]
    pub runtime_dir: PathBuf,

    #[builder(default)]
    pub running_args: ProxyRunningArguments,

    #[builder(default = true)]
    pub auto_restart: bool,

    #[builder(default = 0)]
    pub current_attempts: u32,

    /// 最近一次写入运行时 YAML 时使用的源配置内容哈希。
    ///
    /// 与 [`Self::previous_running_args`] 一起判断是否可跳过重新 merge 写盘：
    /// runtime YAML 语义上由 `source + ProxyRunningArguments` 唯一决定。
    #[builder(into)]
    pub source_content_hash: Option<String>,

    /// 最近一次写入运行时 YAML 时使用的运行参数。
    pub previous_running_args: Option<ProxyRunningArguments>,
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

/// 代理核心启动实例，作为 [`ProxyCoreExecution::launch_once`] 的返回值.
pub struct LaunchingInstance {
    pub child: tokio::process::Child,
    pub pid: u32,
    pub api_stream: ProxyApiStream,
    pub generation: u32,
}

/// 代理核心运行状态
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ProxyCoreStatus {
    /// 正在启动
    Starting { attempt: u32 },
    /// 运行中
    Running { pid: u32, generation: u32 },
    /// 正在停止
    Stopping { restarting: bool },
    /// 已正常停止或者未启动
    Stopped,
    /// 启动或重启失败，携带完整结构化错误
    Failed { error: Arc<ProxyCoreError> },
    /// 异常崩溃退出
    Crashed { exit_code: Option<i32> },
}

impl From<ProxyCoreError> for ProxyCoreStatus {
    fn from(error: ProxyCoreError) -> Self {
        match error {
            ProxyCoreError::ExitedBeforeReady { exit_code } => Self::Crashed { exit_code },
            other => Self::Failed { error: Arc::new(other) },
        }
    }
}

impl ProxyCoreStatus {
    /// 由错误生成状态；调用方仍保留原错误所有权。
    pub fn from_error(error: &ProxyCoreError) -> Self {
        Self::from(error.clone())
    }
}
