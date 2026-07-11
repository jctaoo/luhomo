use bon::Builder;
use serde::{Deserialize, Serialize};

/// 日志级别
///
/// Clash 内核输出日志的等级，仅在控制台和控制页面输出
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    /// 静默，不输出
    Silent,
    /// 仅输出发生错误至无法使用的日志
    Error,
    /// 输出发生错误但不影响运行的日志，以及 error 级别内容
    Warning,
    /// 输出一般运行的内容，以及 error 和 warning 级别的日志
    #[default]
    Info,
    /// 尽可能的输出运行中所有的信息
    Debug,
}

/// 运行模式
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// 规则匹配
    #[default]
    Rule,
    /// 全局代理（需要在 GLOBAL 策略组选择代理/策略）
    Global,
    /// 全局直连
    Direct,
}

/// 代理运行参数
///
/// mihomo 内核的基础运行参数
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, Builder)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProxyRunningArguments {
    /// 代理监听端口
    #[builder(default = 7890)]
    pub port: u16,

    /// 允许其他设备经过 Clash 的代理端口访问互联网
    #[builder(default)]
    pub allow_lan: bool,

    /// 绑定地址，仅允许其他设备通过这个地址访问
    ///
    /// `"*"` 绑定所有 IP 地址
    /// `"192.168.31.31"` 绑定单个 IPv4 地址
    /// `"[aaaa::a8aa:ff:fe09:57d8]"` 绑定单个 IPv6 地址
    #[builder(default = default_bind_address())]
    pub bind_address: String,

    /// 运行模式：rule(规则匹配) / global(全局代理) / direct(全局直连)
    #[builder(default)]
    pub mode: Mode,

    /// 日志级别，默认 info
    #[builder(default)]
    pub log_level: LogLevel,

    /// 是否允许内核接受 IPv6 流量，默认 true
    #[builder(default = true)]
    pub ipv6: bool,

    /// 外部控制器（API）监听地址
    ///
    /// 如 `127.0.0.1:9090`，可以修改为 `0.0.0.0` 来监听所有 IP
    pub external_controller: Option<String>,

    /// Unix socket API 监听地址
    ///
    /// 从 Unix socket 访问 API 不会验证 secret，如果开启请自行保证安全问题
    pub external_controller_unix: Option<String>,

    /// Windows namedpipe API 监听地址
    ///
    /// 从 Windows namedpipe 访问 API 不会验证 secret，如果开启请自行保证安全问题
    pub external_controller_pipe: Option<String>,

    /// 自定义外部用户界面名字
    ///
    /// 合并为 `external-ui/名字`，非必须
    pub external_ui_name: Option<String>,

    /// 自定义外部用户界面下载地址
    pub external_ui_url: Option<String>,
}

impl Default for ProxyRunningArguments {
    fn default() -> Self {
        Self {
            port: 7890,
            allow_lan: false,
            bind_address: default_bind_address(),
            mode: Mode::default(),
            log_level: LogLevel::default(),
            ipv6: true,
            external_controller: None,
            external_controller_unix: None,
            external_controller_pipe: None,
            external_ui_name: None,
            external_ui_url: None,
        }
    }
}

#[inline]
fn default_bind_address() -> String {
    "*".to_string()
}
