use luhomo_core::config::models::{ConfigurationItem, ConfigurationSource};
use luhomo_core::proxy::core_type::ProxyCoreType;
use luhomo_core::proxy::execution::ProxyCoreExecution;
use luhomo_core::proxy::global_args::ProxyRunningArguments;
use luhomo_core::proxy::launch_err::ProxyCoreError;
use luhomo_core::proxy::launch_status::{ProxyApiStream, ProxyCoreStatus};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

/// 进程测试共用 TCP 端口分配逻辑；串行执行以避免刚释放端口被另一用例复用。
static PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 每个用例独占的运行目录；离开作用域后删除其 YAML、日志和重启标记。
struct TestRuntime(PathBuf);

impl TestRuntime {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("luhomo-execution-test-{}", uuid::Uuid::new_v4()));
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// 返回测试替身实际启动过的进程次数。
    fn launch_count(&self) -> usize {
        std::fs::read_to_string(self.path().join("proxy-core-test-double.launch-count"))
            .map(|content| content.lines().count())
            .unwrap_or(0)
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 创建仅用于生成运行时 YAML 文件名的配置项。
fn configuration() -> ConfigurationItem {
    ConfigurationItem::builder()
        .source(ConfigurationSource::LocalFile("proxy-core-test-double.yaml".to_owned()))
        .display_name("proxy core test double")
        .build()
}

/// 获取一个当前空闲的临时 TCP 地址，写入测试核心的 `external-controller` 配置。
fn unused_tcp_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// 使用 Cargo 构建的测试替身，而不是系统安装的 mihomo。
///
/// 替身兼容 Mihomo 的 `-d`/`-f` 参数和 `external-controller` 配置，因此仍使用
/// `ProxyCoreType::Mihomo` 来测试真实的命令构造和运行时配置合并路径。
fn execution(runtime: &TestRuntime, auto_restart: bool) -> ProxyCoreExecution {
    ProxyCoreExecution::builder()
        .core_type(ProxyCoreType::Mihomo)
        .executable(env!("CARGO_BIN_EXE_proxy_core_test_double"))
        .runtime_dir(runtime.path())
        .auto_restart(auto_restart)
        .build()
}

/// 构造一个可执行文件不存在的实例，用于验证启动前的失败路径。
fn execution_with_missing_executable(runtime: &TestRuntime) -> ProxyCoreExecution {
    ProxyCoreExecution::builder()
        .core_type(ProxyCoreType::Mihomo)
        .executable(runtime.path().join("missing-mihomo.exe"))
        .runtime_dir(runtime.path())
        .build()
}

/// 配置测试替身监听的 TCP API 地址。
fn running_arguments(address: &str) -> ProxyRunningArguments {
    ProxyRunningArguments::builder()
        .external_controller(address.to_owned())
        .build()
}

/// 等待监控任务发布满足条件的状态，超过五秒则视为状态机未按预期推进。
async fn wait_for_status(
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

#[tokio::test]
async fn launches_redirects_output_rejects_a_second_launch_and_shuts_down() {
    // `stay-running` 模拟一个 API 已就绪、持续存活的正常核心。
    // 覆盖：首次启动、重复启动保护、stdout 重定向、正常关闭与重复关闭保护。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let args = running_arguments(&address);
    let mut execution = execution(&runtime, false);

    let stream = execution
        .launch(&configuration(), b"test-mode: stay-running\n", &args)
        .await
        .unwrap();
    assert!(matches!(stream, ProxyApiStream::Tcp(_)));
    assert!(matches!(
        execution.status(),
        ProxyCoreStatus::Running { generation: 1, .. }
    ));

    let second_launch = execution
        .launch(&configuration(), b"test-mode: stay-running\n", &args)
        .await;
    assert!(matches!(second_launch, Err(ProxyCoreError::AlreadyRunning { .. })));

    let stdout = std::fs::read_to_string(runtime.path().join("logs/mihomo.stdout.log")).unwrap();
    assert!(stdout.contains("test proxy core API ready"));

    execution.shutdown().await.unwrap();
    assert!(matches!(execution.status(), ProxyCoreStatus::Stopped));
    assert!(matches!(execution.shutdown().await, Err(ProxyCoreError::NotRunning)));
}

#[tokio::test]
async fn records_an_unexpected_exit_without_restarting_when_disabled() {
    // `crash-after-ready` 先开放 API，再以退出码 23 退出。
    // auto_restart=false 时，监控任务应保留 Crashed 状态而不尝试重启。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let mut execution = execution(&runtime, false);
    let mut statuses = execution.status_watcher();

    execution
        .launch(
            &configuration(),
            b"test-mode: crash-after-ready\n",
            &running_arguments(&address),
        )
        .await
        .unwrap();

    let status = wait_for_status(&mut statuses, |status| {
        matches!(status, ProxyCoreStatus::Crashed { .. })
    })
    .await;
    assert!(matches!(status, ProxyCoreStatus::Crashed { exit_code: Some(23) }));
}

#[tokio::test]
async fn restarts_after_a_crash_and_increments_the_generation() {
    // `crash-once` 仅在首次启动后退出；重启进程会保持运行。
    // 断言监控任务完成重启，并把 generation 从 1 递增为 2。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let mut execution = execution(&runtime, true);
    let mut statuses = execution.status_watcher();

    execution
        .launch(
            &configuration(),
            b"test-mode: crash-once\n",
            &running_arguments(&address),
        )
        .await
        .unwrap();

    let status = wait_for_status(&mut statuses, |status| {
        matches!(status, ProxyCoreStatus::Running { generation: 2, .. })
    })
    .await;
    assert!(matches!(status, ProxyCoreStatus::Running { generation: 2, .. }));

    execution.shutdown().await.unwrap();
    assert!(matches!(execution.status(), ProxyCoreStatus::Stopped));
}

#[tokio::test]
async fn reports_a_process_that_exits_before_its_api_is_ready() {
    // `exit-before-ready` 在绑定 API 端口前退出。
    // launch 应立即失败，而不是建立监控任务或把进程误判为正常运行。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let mut execution = execution(&runtime, false);

    let result = execution
        .launch(
            &configuration(),
            b"test-mode: exit-before-ready\n",
            &running_arguments(&address),
        )
        .await;

    assert!(matches!(
        result,
        Err(ProxyCoreError::ExitedBeforeReady { exit_code: Some(23) })
    ));
    assert!(matches!(
        execution.status(),
        ProxyCoreStatus::Crashed { exit_code: Some(23) }
    ));
}

#[tokio::test]
async fn rejects_a_missing_executable_before_starting_the_core() {
    // launch_once 必须在创建运行目录、写 YAML 或发布 Starting 前检查可执行文件。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let mut execution = execution_with_missing_executable(&runtime);

    let result = execution
        .launch(
            &configuration(),
            b"test-mode: stay-running\n",
            &running_arguments("127.0.0.1:1"),
        )
        .await;

    assert!(matches!(result, Err(ProxyCoreError::ExecutableNotFound(_))));
    assert!(matches!(execution.status(), ProxyCoreStatus::Stopped));
}

#[tokio::test]
async fn fails_startup_and_kills_the_core_when_its_api_never_becomes_ready() {
    // 进程保持存活却不监听 external-controller，模拟 core 已启动但 API 初始化失败。
    // launch_once 应杀掉该进程，并把状态从 Starting 变为 Failed。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let mut execution = execution(&runtime, false);
    let mut statuses = execution.status_watcher();

    let result = execution
        .launch(
            &configuration(),
            b"test-mode: stay-running-without-api\n",
            &running_arguments(&address),
        )
        .await;

    let error = match result {
        Err(error @ ProxyCoreError::SocketChannelCheckFailed(_)) => error,
        Err(error) => panic!("expected socket readiness failure, got {error:?}"),
        Ok(_) => panic!("launch unexpectedly succeeded"),
};
    let status = wait_for_status(&mut statuses, |status| matches!(status, ProxyCoreStatus::Failed { .. })).await;
    let ProxyCoreStatus::Failed { message } = status else {
        unreachable!("wait_for_status only returns Failed")
    };
    assert_eq!(message, error.to_string());
}

#[tokio::test]
async fn rejects_startup_without_an_api_endpoint() {
    // 缺少 external-controller 时，launch_once 不应把任何 API stream 交给调用方。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let mut execution = execution(&runtime, false);
    let mut statuses = execution.status_watcher();

    let result = execution
        .launch(
            &configuration(),
            b"test-mode: stay-running\n",
            &ProxyRunningArguments::default(),
        )
        .await;

    let error = match result {
        Err(error @ ProxyCoreError::ApiEndpointNotConfigured) => error,
        Err(error) => panic!("expected missing API endpoint error, got {error:?}"),
        Ok(_) => panic!("launch unexpectedly succeeded"),
    };
    let status = wait_for_status(&mut statuses, |status| matches!(status, ProxyCoreStatus::Failed { .. })).await;
    let ProxyCoreStatus::Failed { message } = status else {
        unreachable!("wait_for_status only returns Failed")
    };
    assert_eq!(message, error.to_string());
}

#[tokio::test]
async fn marks_the_execution_failed_when_an_automatic_restart_cannot_open_its_api() {
    // 首次启动正常并崩溃；第二次启动故意不监听 API。
    // 这覆盖监控任务的“重启 launch_once 失败 → Failed”分支。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let mut execution = execution(&runtime, true);
    let mut statuses = execution.status_watcher();

    execution
        .launch(
            &configuration(),
            b"test-mode: crash-once-then-no-api\n",
            &running_arguments(&address),
        )
        .await
        .unwrap();

    let status = wait_for_status(&mut statuses, |status| matches!(status, ProxyCoreStatus::Failed { .. })).await;
    let ProxyCoreStatus::Failed { message } = status else {
        unreachable!("wait_for_status only returns Failed")
    };
    // 底层的 io::Error 文本随操作系统变化；校验稳定的错误类型前缀即可。
    assert!(message.starts_with("socket channel check failed:"));
}

#[tokio::test]
async fn shutdown_during_restart_backoff_prevents_a_new_process_from_starting() {
    // 在 core 崩溃且监控任务的一秒退避等待期间关闭，应直接到 Stopped，不能启动第二代进程。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let mut execution = execution(&runtime, true);
    let mut statuses = execution.status_watcher();

    execution
        .launch(
            &configuration(),
            b"test-mode: crash-after-ready\n",
            &running_arguments(&address),
        )
        .await
        .unwrap();
    assert_eq!(runtime.launch_count(), 1, "首次 launch 应只启动一个测试核心进程");

    // 先确认 core 已崩溃，确保接下来的 shutdown 发生在自动重启的一秒退避期内，
    // 而不是覆盖普通的运行中停止流程。
    wait_for_status(&mut statuses, |status| {
        matches!(status, ProxyCoreStatus::Crashed { .. })
    })
    .await;
    execution.shutdown().await.unwrap();
    // 越过自动重启的退避时间后仍只能观察到首次启动，证明第二代进程没有被 spawn。
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert_eq!(runtime.launch_count(), 1, "shutdown 后不应启动第二代测试核心进程");
    assert!(matches!(execution.status(), ProxyCoreStatus::Stopped));
}
