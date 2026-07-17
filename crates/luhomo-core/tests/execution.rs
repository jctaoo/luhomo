use luhomo_core::proxy::global_args::ProxyRunningArguments;
use luhomo_core::proxy::launch_err::ProxyCoreError;
use luhomo_core::proxy::launch_status::{ProxyApiStream, ProxyCoreStatus};
use std::time::Duration;
mod support;

use support::*;

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

    execution.shutdown().await.unwrap();
    assert_recorded_processes_are_exited(&runtime).await;
    assert_eq!(runtime.stdout_lines(), [api_ready_output(&address)]);
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
    assert_recorded_processes_are_exited(&runtime).await;
    assert_eq!(runtime.stdout_lines(), [api_ready_output(&address)]);
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
    assert_recorded_processes_are_exited(&runtime).await;
    assert_eq!(
        runtime.stdout_lines(),
        [api_ready_output(&address), api_ready_output(&address)]
    );
    assert!(matches!(execution.status(), ProxyCoreStatus::Stopped));
}

#[tokio::test]
async fn reuses_the_runtime_yaml_when_its_hash_matches_during_restart() {
    // 首次启动会写入运行时 YAML；`crash-once` 触发自动重启后，第二次 launch_once
    // 应验证该文件哈希匹配并复用它，而不是再次合并、写入同一路径。
    let _guard = PROCESS_TEST_LOCK.lock().await;
    let runtime = TestRuntime::new();
    let address = unused_tcp_address();
    let configuration = configuration();
    let runtime_yaml = runtime.path().join(format!("{}.yaml", configuration.uuid));
    let mut execution = execution(&runtime, true);
    let mut statuses = execution.status_watcher();

    execution
        .launch(
            &configuration,
            b"test-mode: crash-once\n",
            &running_arguments(&address),
        )
        .await
        .unwrap();
    let first_write_time = std::fs::metadata(&runtime_yaml)
        .expect("first launch should write runtime YAML")
        .modified()
        .expect("runtime YAML should expose a modification time");

    wait_for_status(&mut statuses, |status| {
        matches!(status, ProxyCoreStatus::Running { generation: 2, .. })
    })
    .await;

    let restart_write_time = std::fs::metadata(&runtime_yaml)
        .expect("restart should retain runtime YAML")
        .modified()
        .expect("runtime YAML should expose a modification time");
    assert_eq!(
        restart_write_time, first_write_time,
        "a matching runtime YAML hash must skip rewriting the file"
    );

    execution.shutdown().await.unwrap();
    assert_recorded_processes_are_exited(&runtime).await;
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
    assert_recorded_processes_are_exited(&runtime).await;
    assert!(runtime.stdout_lines().is_empty());
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
    assert!(runtime.recorded_pids().is_empty());
    assert!(runtime.stdout_lines().is_empty());
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
    assert_recorded_processes_are_exited(&runtime).await;
    assert!(runtime.stdout_lines().is_empty());
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
    // API 端点校验立即失败，Child 会在测试替身的 main 开始前被终止，因而不应
    // 留下任何已登记的测试核心 PID。
    assert!(runtime.recorded_pids().is_empty());
    assert!(runtime.stdout_lines().is_empty());
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
    assert_recorded_processes_are_exited(&runtime).await;
    assert_eq!(runtime.stdout_lines(), [api_ready_output(&address)]);
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
    assert_recorded_processes_are_exited(&runtime).await;
    assert_eq!(runtime.stdout_lines(), [api_ready_output(&address)]);
    assert!(matches!(execution.status(), ProxyCoreStatus::Stopped));
}
