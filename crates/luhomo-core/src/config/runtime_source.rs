use std::sync::Arc;

use bytes::Bytes;

use crate::config::ConfigurationManager;
use crate::config::storage::{ConfigurationStorage, ConfigurationStorageError};

/// 运行时按配置 UUID 加载最新配置内容的窄接口。
///
/// 供 proxy 启动 / 自动重启使用，避免进程生命周期依赖完整的配置管理能力。
pub trait RuntimeConfigSource: Send + Sync {
    fn load(
        &self,
        id: &uuid::Uuid,
    ) -> impl std::future::Future<Output = Result<Bytes, ConfigurationStorageError>> + Send;
}

impl RuntimeConfigSource for ConfigurationStorage {
    fn load(
        &self,
        id: &uuid::Uuid,
    ) -> impl std::future::Future<Output = Result<Bytes, ConfigurationStorageError>> + Send {
        async move { self.get(id).await }
    }
}

impl<T> RuntimeConfigSource for T
where
    T: ConfigurationManager + Send + Sync,
{
    fn load(
        &self,
        id: &uuid::Uuid,
    ) -> impl std::future::Future<Output = Result<Bytes, ConfigurationStorageError>> + Send {
        async move { self.storage().get(id).await }
    }
}

impl<T> RuntimeConfigSource for Arc<T>
where
    T: RuntimeConfigSource + ?Sized,
{
    fn load(
        &self,
        id: &uuid::Uuid,
    ) -> impl std::future::Future<Output = Result<Bytes, ConfigurationStorageError>> + Send {
        async move { (**self).load(id).await }
    }
}

/// 固定内容的配置源，适合测试或一次性启动。
#[derive(Debug, Clone)]
pub struct StaticRuntimeConfigSource {
    content: Bytes,
}

impl StaticRuntimeConfigSource {
    pub fn new(content: impl AsRef<[u8]>) -> Self {
        Self {
            content: Bytes::copy_from_slice(content.as_ref()),
        }
    }

    pub fn content(&self) -> &Bytes {
        &self.content
    }
}

impl RuntimeConfigSource for StaticRuntimeConfigSource {
    fn load(
        &self,
        _id: &uuid::Uuid,
    ) -> impl std::future::Future<Output = Result<Bytes, ConfigurationStorageError>> + Send {
        let content = self.content.clone();
        async move { Ok(content) }
    }
}
