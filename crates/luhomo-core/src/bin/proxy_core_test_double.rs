//! `ProxyCoreExecution` 的进程级测试替身。
//!
//! 它只识别 Mihomo 风格的 `-d`/`-f` 参数，以及集成测试写入 YAML 的
//! `external-controller` 和 `test-mode` 字段；并不具备真实代理核心功能。
//!
//! `test-mode` 控制进程生命周期：
//! - `stay-running`：绑定 API 后持续运行，等待测试侧终止；
//! - `crash-after-ready`：绑定 API 后以退出码 23 退出；
//! - `crash-once`：同一运行目录的首次启动崩溃，后续启动持续运行；
//! - `crash-once-then-no-api`：首次崩溃，后续启动不开放 API 但保持运行；
//! - `exit-before-ready`：绑定 API 前以退出码 23 退出；
//! - `stay-running-without-api`：不绑定 API，但保持运行。

use std::fs::OpenOptions;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn argument_value(name: &str) -> PathBuf {
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("missing value for {name}");
                std::process::exit(2);
            });
        }
    }
    eprintln!("missing required argument {name}");
    std::process::exit(2);
}

/// 等待父进程实际连入 API，再模拟运行中崩溃。
fn crash_after_api_is_ready(listener: TcpListener) -> ! {
    // bind 只表示端口可被连接；accept 返回才表示父进程已完成 TCP 三次握手，
    // 因而可以确认 `ensure_api_ready` 确实建立了连接。
    let _api_connection = listener.accept().unwrap_or_else(|error| {
        eprintln!("failed to accept test proxy core API connection: {error}");
        std::process::exit(2);
    });

    // accept 后稍作保留，让父进程从 TcpStream::connect 返回并完成 select；否则
    // 过快退出可能被竞态地判为“API 就绪前退出”。
    std::thread::sleep(Duration::from_millis(750));
    eprintln!("test proxy core is crashing");
    std::process::exit(23);
}

/// 永不返回；仅能由 `ProxyCoreExecution` 调用 `start_kill()` 终止整个进程。
///
/// 即使单次 `park_timeout` 到期，循环也会立刻再次 park，因此不会继续执行调用点
/// 后面的 `external-controller` 读取或其他逻辑。
fn remain_running() -> ! {
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

fn first_run(runtime_dir: &Path) -> bool {
    // 重启前后使用同一 runtime_dir，因此标记文件会保留。
    // 只有文件不存在时 create_new 才成功：首次调用返回 true，之后在相同
    // runtime_dir 的重启调用返回 false。
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(runtime_dir.join("proxy-core-test-double.first-run"))
        .is_ok()
}

/// 记录一次测试核心进程启动及其 PID，供集成测试确认重启次数和进程清理结果。
fn record_launch(runtime_dir: &Path) {
    let path = runtime_dir.join("proxy-core-test-double.launches");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|error| {
            eprintln!("failed to open {}: {error}", path.display());
            std::process::exit(2);
        });
    writeln!(file, "pid={}", std::process::id()).unwrap_or_else(|error| {
        eprintln!("failed to write {}: {error}", path.display());
        std::process::exit(2);
    });
    file.flush().unwrap_or_else(|error| {
        eprintln!("failed to flush {}: {error}", path.display());
        std::process::exit(2);
    });
}

fn config_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    // manifest 写入器会将这些顶层标量字段输出为 `key: value`。
    config.lines().find_map(|line| {
        let value = line.trim_start().strip_prefix(key)?.strip_prefix(':')?.trim();
        Some(value.trim_matches(['\'', '"']))
    })
}

fn main() {
    let runtime_dir = argument_value("-d");
    record_launch(&runtime_dir);
    let config_path = argument_value("-f");
    let config = std::fs::read_to_string(&config_path).unwrap_or_else(|error| {
        eprintln!("failed to read {}: {error}", config_path.display());
        std::process::exit(2);
    });
    let mode = config_value(&config, "test-mode").unwrap_or("stay-running");

    if mode == "exit-before-ready" {
        eprintln!("test proxy core exits before opening its API");
        std::process::exit(23);
    }

    // 这些模式刻意保持进程存活但不提供监听器，使 LaunchState 的 API 就绪检查
    // 失败，并由其负责终止子进程。
    if mode == "stay-running-without-api" || (mode == "crash-once-then-no-api" && !first_run(&runtime_dir)) {
        remain_running();
    }

    let address = config_value(&config, "external-controller").unwrap_or_else(|| {
        eprintln!("external-controller is required by the test proxy core");
        std::process::exit(2);
    });
    let listener = TcpListener::bind(address).unwrap_or_else(|error| {
        eprintln!("failed to bind {address}: {error}");
        std::process::exit(2);
    });
    println!("test proxy core API ready at {address}");

    match mode {
        // API 已可连接后主动退出，用于验证运行中的 core 崩溃处理。
        "crash-after-ready" => crash_after_api_is_ready(listener),
        // 首次运行创建标记后崩溃；重启时会落入下一条持续运行分支。
        "crash-once" if first_run(&runtime_dir) => crash_after_api_is_ready(listener),
        // 此分支只会在 crash-once-then-no-api 的首次运行命中；第二次运行已在
        // 上方进入 remain_running，借此模拟“自动重启后 API 不可用”。
        "crash-once-then-no-api" => crash_after_api_is_ready(listener),
        // crash-once 的第二次及以后运行，以及普通正常运行场景：保持 API 可用，
        // 等待 ProxyCoreExecution 在 shutdown 时终止该进程。
        "crash-once" | "stay-running" => remain_running(),
        // 未定义模式应立即失败，避免测试配置写错时伪装成正常核心。
        unknown => {
            eprintln!("unknown test-mode: {unknown}");
            std::process::exit(2);
        }
    }
}
