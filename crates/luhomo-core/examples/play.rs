//! 使用方式：`cargo run -p luhomo-core --example play`
//!
//! 启动前请确保 mihomo 可执行文件可用；可通过 `MIHOMO_PATH` 指定其路径。

use std::io::{self, Write};

use luhomo_core::{
    config::{
        ConfigurationManager, LocalConfigurationManager,
        models::{ConfigurationSource, UpdateStrategy},
    },
    proxy::{
        ProxyCoreType,
        execution::{ProxyApiStream, ProxyCoreExecution},
        global_args::ProxyRunningArguments,
    },
};
use tracing_subscriber::EnvFilter;
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("luhomo_core=trace")))
        .with_target(false)
        .init();

    let subscription_url = read_input("订阅链接: ")?;
    let subscription_url = Url::parse(&subscription_url)?;

    let storage_dir = std::env::temp_dir().join("luhomo-play-config");
    println!("配置存储目录: {}", storage_dir.display());
    let manager = LocalConfigurationManager::new(storage_dir, reqwest::Client::new());
    let source = ConfigurationSource::RemoteUrl {
        url: subscription_url.clone(),
        update_strategy: UpdateStrategy {
            auto_update: false,
            interval: None,
        },
        homepage: None,
        use_proxy: false,
        // 与 Clash Party 的默认订阅请求 UA 保持一致。
        user_agent: Some(format!("mihomo.party/v{} (clash.meta)", env!("CARGO_PKG_VERSION"))),
    };

    println!("正在通过 LocalConfigurationManager 下载并缓存订阅配置...");
    let item = manager.add(source).await?;
    let content = manager.get_content(&item.uuid).await?;
    // `content` 就是订阅原始 YAML，直接作为 runtime manifest 的基础配置。

    let controller = read_input_with_default("API 控制器地址 [127.0.0.1:9090]: ", "127.0.0.1:9090")?;
    let args = ProxyRunningArguments::builder().external_controller(controller).build();
    let mut execution = ProxyCoreExecution::new(ProxyCoreType::Mihomo);
    let api_stream = execution.launch(&item, content, &args).await?;

    match &api_stream {
        ProxyApiStream::Tcp(_) => println!("mihomo 已启动，已连接 TCP API。"),
        ProxyApiStream::Local(_) => println!("mihomo 已启动，已连接本地 API。"),
    }

    println!("按 Enter 停止 mihomo。");
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    execution.shutdown().await?;
    println!("mihomo 已停止。");

    Ok(())
}

fn read_input(prompt: &str) -> io::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_owned())
}

fn read_input_with_default(prompt: &str, default: &str) -> io::Result<String> {
    let input = read_input(prompt)?;
    Ok((!input.is_empty())
        .then_some(input)
        .unwrap_or_else(|| default.to_owned()))
}
