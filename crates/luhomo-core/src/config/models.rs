use bon::{Builder, bon};
use serde::{Deserialize, Serialize};
use serde_with::{DurationSeconds, serde_as};
use url::Url;

/// 配置更新策略
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, Builder)]
pub struct UpdateStrategy {
    /// 是否自动更新
    pub auto_update: bool,
    /// 更新间隔（秒），不设置则不自动更新
    #[serde_as(as = "Option<DurationSeconds<i64>>")]
    pub interval: Option<time::Duration>,
}

/// 配置来源
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum ConfigurationSource {
    /// 本地文件（文件路径）
    LocalFile(String),
    /// 远程订阅
    RemoteUrl {
        /// 订阅 URL
        url: Url,
        /// 更新策略
        update_strategy: UpdateStrategy,
        /// 订阅管理页面 URL，来自响应头 profile-web-page-url
        homepage: Option<Url>,
        /// 是否通过代理拉取
        use_proxy: bool,
        /// 更新订阅时使用的 HTTP User-Agent。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_agent: Option<String>,
    },
}

#[bon]
impl ConfigurationSource {
    /// 本地文件配置来源
    #[builder]
    pub fn local_file(#[builder(into)] path: String) -> Self {
        Self::LocalFile(path)
    }

    /// 远程订阅配置来源
    #[builder]
    pub fn remote_url(
        #[builder(into)] url: String,
        update_strategy: UpdateStrategy,
        #[builder(into)] homepage: Option<String>,
        use_proxy: bool,
        #[builder(into)] user_agent: Option<String>,
    ) -> Result<Self, url::ParseError> {
        let url = Url::parse(&url)?;
        let homepage = homepage.as_deref().map(Url::parse).transpose()?;
        Ok(Self::RemoteUrl {
            url,
            update_strategy,
            homepage,
            use_proxy,
            user_agent,
        })
    }
}

/// 订阅流量信息，来自响应头 subscription-userinfo
/// 格式: upload=1234; download=2234; total=1024000; expire=2218532293
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Copy, Deserialize, Serialize, Builder)]
pub struct SubscriptionInfo {
    /// 已上传流量（字节）
    pub upload: u64,
    /// 已下载流量（字节）
    pub download: u64,
    /// 总流量（字节）
    pub total: u64,
    /// 过期时间（Unix 时间戳）
    pub expire: u64,
}

/// 代理核心的配置项
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, Builder)]
pub struct ConfigurationItem {
    /// 配置项唯一标识
    #[builder(default = uuid::Uuid::new_v4())]
    pub uuid: uuid::Uuid,
    /// 配置来源
    pub source: ConfigurationSource,
    /// 显示名称
    #[builder(into)]
    pub display_name: String,
    /// 订阅流量信息
    #[builder(default)]
    pub subscription_info: SubscriptionInfo,

    /// 创建时间
    #[builder(default = time::OffsetDateTime::now_utc())]
    #[serde(with = "time::serde::timestamp")]
    pub created_at: time::OffsetDateTime,
    /// 更新时间
    #[builder(default = time::OffsetDateTime::now_utc())]
    #[serde(with = "time::serde::timestamp")]
    pub updated_at: time::OffsetDateTime,
}
