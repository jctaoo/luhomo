#![allow(dead_code)]

use luhomo_core::config::models::{ConfigurationItem, ConfigurationSource};
use luhomo_core::proxy::core_type::ProxyCoreType;
use luhomo_core::proxy::execution::ProxyCoreExecution;
use luhomo_core::proxy::global_args::ProxyRunningArguments;
use luhomo_core::proxy::launch_status::ProxyCoreStatus;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;
use sysinfo::{Pid, System};
use tokio::sync::watch;

/// 进程测试共用 TCP 端口分配逻辑；串行执行以避免刚释放端口被另一用例复用。
pub static PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 每个用例独占的运行目录；离开作用域后删除其 YAML、日志和测试替身记录。
pub struct TestRuntime(PathBuf);

impl TestRuntime {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("luhomo-execution-test-{}", uuid::Uuid::new_v4()));
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// 返回测试替身实际启动过的进程次数。
    pub fn launch_count(&self) -> usize {
        self.recorded_pids().len()
    }

    /// 返回测试替身在本运行目录中登记的全部进程 PID。
    pub fn recorded_pids(&self) -> Vec<u32> {
        std::fs::read_to_string(self.path().join("proxy-core-test-double.launches"))
            .map(|content| {
                content
                    .lines()
                    .map(|line| line.strip_prefix("pid=").expect("invalid test-double launch record"))
                    .map(|pid| pid.parse().expect("invalid test-double PID"))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 返回测试核心重定向到日志文件的全部 stdout 行。
    pub fn stdout_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.path().join("logs/mihomo.stdout.log"))
            .map(|content| content.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 创建仅用于生成运行时 YAML 文件名的配置项。
pub fn configuration() -> ConfigurationItem {
    ConfigurationItem::builder()
        .source(ConfigurationSource::LocalFile("proxy-core-test-double.yaml".to_owned()))
        .display_name("proxy core test double")
        .build()
}

/// 获取一个当前空闲的临时 TCP 地址，写入测试核心的 `external-controller` 配置。
pub fn unused_tcp_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// 使用 Cargo 构建的测试替身，而不是系统安装的 mihomo。
pub fn execution(runtime: &TestRuntime, auto_restart: bool) -> ProxyCoreExecution {
    ProxyCoreExecution::builder()
        .core_type(ProxyCoreType::Mihomo)
        .executable(env!("CARGO_BIN_EXE_proxy_core_test_double"))
        .runtime_dir(runtime.path())
        .auto_restart(auto_restart)
        .build()
}

/// 构造一个可执行文件不存在的实例，用于验证启动前的失败路径。
pub fn execution_with_missing_executable(runtime: &TestRuntime) -> ProxyCoreExecution {
    ProxyCoreExecution::builder()
        .core_type(ProxyCoreType::Mihomo)
        .executable(runtime.path().join("missing-mihomo.exe"))
        .runtime_dir(runtime.path())
        .build()
}

/// 配置测试替身监听的 TCP API 地址。
pub fn running_arguments(address: &str) -> ProxyRunningArguments {
    ProxyRunningArguments::builder()
        .external_controller(address.to_owned())
        .build()
}

pub fn api_ready_output(address: &str) -> String {
    format!("test proxy core API ready at {address}")
}

/// 等待监控任务发布满足条件的状态，超过五秒则视为状态机未按预期推进。
pub async fn wait_for_status(
    receiver: &mut watch::Receiver<ProxyCoreStatus>,
    predicate: impl Fn(&ProxyCoreStatus) -> bool,
) -> ProxyCoreStatus {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = receiver.borrow().clone();
            if predicate(&status) {
                return status;
            }
            receiver.changed().await.expect("execution status sender was dropped");
        }
    })
    .await
    .expect("timed out waiting for proxy core status")
}

/// 确认本测试启动过的每个测试核心进程都已退出。
pub async fn assert_recorded_processes_are_exited(runtime: &TestRuntime) {
    let pids = runtime.recorded_pids();
    assert!(!pids.is_empty(), "the test double did not record any process PID");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let system = System::new_all();
        let still_running: Vec<_> = pids
            .iter()
            .copied()
            .filter(|pid| system.process(Pid::from_u32(*pid)).is_some())
            .collect();
        if still_running.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "test core processes are still running after cleanup: {still_running:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
