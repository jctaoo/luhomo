use serde::{Deserialize, Serialize};

/// 代理核心运行状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyCoreStatus {
    /// 未启动
    Idle,
    /// 正在启动
    Starting,
    /// 运行中
    Running {
        /// 进程 ID
        pid: u32,
    },
    /// 已正常停止
    Stopped,
    /// 异常崩溃退出
    Crashed {
        /// 进程退出码
        exit_code: Option<i32>,
    },
}