//! 使用方式：`cargo run -p luhomo-core --example play [-- -v|--verbose]`
//!
//! 启动前请确保 mihomo 可执行文件可用；可通过 `MIHOMO_PATH` 指定其路径。

use std::io::{self, Write};

use luhomo_core::{
    config::{
        ConfigurationManager, LocalConfigurationManager,
        models::{ConfigurationItem, ConfigurationSource, UpdateStrategy},
    }, proxy::{
        core_type::ProxyCoreType, execution::ProxyCoreExecution, global_args::ProxyRunningArguments, launch_status::ProxyApiStream,
    },
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing_subscriber::EnvFilter;
use url::Url;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(logging_filter())
        .with_target(false)
        .init();

    let storage_dir = std::env::temp_dir().join("luhomo-play-config");
    println!("配置存储目录: {}", storage_dir.display());
    let manager = LocalConfigurationManager::new(storage_dir.clone(), reqwest::Client::new());
    let items = manager.list().await?;
    let item = if items.is_empty() {
        add_subscription(&manager).await?
    } else {
        match select_configuration(&items)? {
            Some(item) => item,
            None => add_subscription(&manager).await?,
        }
    };
    let controller = read_input_with_default("API 控制器地址 [127.0.0.1:9090]: ", "127.0.0.1:9090")?;
    let args = ProxyRunningArguments::builder().external_controller(controller).build();
    let runtime_dir = storage_dir.join("mihomo-runtime");
    println!("Mihomo 运行目录: {}", runtime_dir.display());
    let mut execution = ProxyCoreExecution::builder()
        .core_type(ProxyCoreType::Mihomo)
        .runtime_dir(runtime_dir)
        .build();
    let manager = std::sync::Arc::new(manager);
    let api_stream = execution.launch(&item, manager, &args).await?;

    match &api_stream {
        ProxyApiStream::Tcp(_) => println!("mihomo 已启动，已连接 TCP API。"),
        ProxyApiStream::Local(_) => println!("mihomo 已启动，已连接本地 API。"),
    }

    println!("按 Enter 或 Ctrl+C 停止 mihomo。");
    wait_for_stop_signal().await?;
    execution.shutdown().await?;
    println!("mihomo 已停止。");

    Ok(())
}

fn logging_filter() -> EnvFilter {
    let verbose = std::env::args().skip(1).any(|arg| matches!(arg.as_str(), "-v" | "--verbose"));
    let default_level = if verbose {
        "trace"
    } else if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };

    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("luhomo_core={default_level}")))
}

async fn wait_for_stop_signal() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    tokio::select! {
        result = stdin.read_line(&mut line) => {
            result?;
        }
        result = tokio::signal::ctrl_c() => {
            result?;
            println!("收到 Ctrl+C，正在停止 mihomo...");
        }
    }

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

async fn add_subscription(
    manager: &LocalConfigurationManager,
) -> Result<ConfigurationItem, Box<dyn std::error::Error>> {
    println!("没有已缓存的配置，请输入订阅链接。");
    let subscription_url = Url::parse(&read_input("订阅链接: ")?)?;
    let source = ConfigurationSource::RemoteUrl {
        url: subscription_url,
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
    Ok(manager.add(source).await?)
}

fn select_configuration(items: &[ConfigurationItem]) -> io::Result<Option<ConfigurationItem>> {
    println!("发现已缓存的配置：");
    for (index, item) in items.iter().enumerate() {
        println!("  {}. {} ({})", index + 1, item.display_name, item.uuid);
    }
    let add_index = items.len() + 1;
    println!("  {add_index}. 新增配置");

    loop {
        let input = read_input(&format!("请选择配置 [1-{add_index}]: "))?;
        match input.parse::<usize>() {
            Ok(index) if (1..=items.len()).contains(&index) => return Ok(Some(items[index - 1].clone())),
            Ok(index) if index == add_index => return Ok(None),
            _ => println!("请输入 1 到 {add_index} 之间的编号。"),
        }
    }
}
