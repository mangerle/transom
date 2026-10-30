//! 真机冒烟程序：对一个正在运行的 Docker/Podman 守护进程逐项验证接入能力。
//!
//! 设计原理（Why）：`tests/mock_engine.rs` 只能证明协议解析正确，无法证明真实
//! 守护进程上的尺寸协商、归档路径、exec HTTP 升级、统计两次采样差分与 Compose
//! 标签聚合可用。本程序把架构文档列出的能力面按顺序跑一遍，单项失败只记录、
//! 不中断，最后按失败数决定退出码，便于一次性确认真机行为。
//!
//! 代价与局限（Trade-offs）：它假定存在常驻容器 `transom-smoke` 与 Compose 项目
//! `transom-demo`，只在容器 `/tmp` 下读写临时文件，但会真的启停容器；跟随验证
//! 要求容器主进程持续产出日志，因此夹具容器的入口命令是每秒一行的 tick 循环。
//! 因此本程序只用于人工验收，不进 CI。所有步骤都有兜底超时，守护进程挂起时
//! 程序仍能自退。
//!
//! 用法：
//! ```sh
//! cargo run -p transom-container --example docker_smoke
//! cargo run -p transom-container --example docker_smoke -- --container 名称 --image 引用
//! ```

use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use transom_container::{
    ContainerError, Engine, create_dir, discover_all, download, list_dir, open_exec,
    probe_explicit, read_file, remove_path, rename_path, stream_logs, upload,
    validate_container_path,
};
use transom_core::{
    ContainerConnection, ContainerCreateSpec, ContainerDetails, ContainerEndpoint,
    ContainerEngineKind, ContainerFileEntry, ContainerFileKind, ContainerLifecycleAction,
    ContainerPortPublish, ContainerState, ContainerStats, ContainerSummary, TerminalIo,
    TerminalSize, short_id,
};

const DEFAULT_CONTAINER: &str = "transom-smoke";
const DEFAULT_IMAGE: &str = "docker.m.daocloud.io/library/busybox:1.36";
const DEFAULT_COMPOSE_PROJECT: &str = "transom-demo";
const DEFAULT_SOCKET: &str = "/var/run/docker.sock";
/// 单步兜底超时：真实守护进程挂起时冒烟程序必须能自己退出。
const STEP_TIMEOUT: Duration = Duration::from_secs(20);
/// 容器内工作路径：只碰 `/tmp`，避免污染镜像内容。
const REMOTE_DIR: &str = "/tmp";

/// 冒烟结果台账：一行一步，失败计数决定退出码。
#[derive(Default)]
struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn pass(&mut self, step: &str, detail: impl Into<String>) {
        self.lines.push(format!("通过  {step}：{}", detail.into()));
    }

    fn fail(&mut self, step: &str, detail: impl Into<String>) {
        self.failures += 1;
        self.lines.push(format!("失败  {step}：{}", detail.into()));
    }

    /// 记录一次断言：条件不成立即失败，用于「返回值是否符合预期」。
    fn check(&mut self, passed: bool, step: &str, detail: impl Into<String>) {
        if passed {
            self.pass(step, detail);
        } else {
            self.fail(step, detail);
        }
    }

    /// 打印台账并按失败数决定退出码。
    fn finish(&self) -> ExitCode {
        for line in &self.lines {
            println!("{line}");
        }
        println!(
            "冒烟统计：共 {} 步，失败 {} 步",
            self.lines.len(),
            self.failures
        );
        if self.failures == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}

/// 带超时执行一步：超时收敛为 [`ContainerError`]，让调用方统一记录。
async fn timed<T>(
    operation: &str,
    future: impl Future<Output = Result<T, ContainerError>>,
) -> Result<T, ContainerError> {
    tokio::time::timeout(STEP_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| {
            Err(ContainerError::Http {
                operation: operation.to_string(),
                message: format!("超过 {} 秒未返回", STEP_TIMEOUT.as_secs()),
            })
        })
}

/// 执行一步：成功时把摘要写进台账并交回数据，失败时记录并让本阶段提前结束。
async fn step<T>(
    report: &mut Report,
    name: &str,
    operation: impl Future<Output = Result<T, ContainerError>>,
    summarize: impl FnOnce(&T) -> String,
) -> Option<T> {
    match timed(name, operation).await {
        Ok(value) => {
            let detail = summarize(&value);
            report.pass(name, detail);
            Some(value)
        }
        Err(error) => {
            report.fail(name, error.to_string());
            None
        }
    }
}

/// 命令行覆盖项：容器名、镜像引用与 Compose 项目名。
struct Options {
    container: String,
    image: String,
    project: String,
    workdir: PathBuf,
}

fn parse_options() -> Options {
    let mut options = Options {
        container: DEFAULT_CONTAINER.to_string(),
        image: DEFAULT_IMAGE.to_string(),
        project: DEFAULT_COMPOSE_PROJECT.to_string(),
        workdir: std::env::temp_dir().join("transom-smoke-local"),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index + 1 < args.len() {
        match args[index].as_str() {
            "--container" => options.container = args[index + 1].clone(),
            "--image" => options.image = args[index + 1].clone(),
            "--project" => options.project = args[index + 1].clone(),
            _ => {}
        }
        index += 2;
    }
    options
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    init_logging();
    let options = parse_options();
    let mut report = Report::default();
    let Some(engine) = discover_engine(&mut report).await else {
        return report.finish();
    };

    // 失败已在各阶段内部记入台账，返回 None 只表示「本阶段余下步骤跳过」。
    let _ = list_stage(&engine, &options.container, &mut report).await;
    let _ = stats_stage(&engine, &options.container, &mut report).await;
    let _ = logs_stage(&engine, &options.container, &mut report).await;
    let _ = exec_stage(&engine, &options.container, &mut report).await;
    let _ = files_stage(&engine, &options.container, &options.workdir, &mut report).await;
    let _ = image_stage(&engine, &options.image, &mut report).await;
    volume_stage(&engine, &mut report).await;
    network_stage(&engine, &mut report).await;
    let _ = compose_stage(&engine, &options.project, &mut report).await;
    path_guard_stage(&mut report);
    let _ = lifecycle_stage(&engine, &options.container, &mut report).await;
    run_stage(&engine, &options.image, &mut report).await;

    report.finish()
}

/// 先走自动发现，全部不可用时回落到显式 unix socket，顺带验证发现顺序本身。
async fn discover_engine(report: &mut Report) -> Option<Engine> {
    let (engines, failures) = discover_all().await;
    report.check(
        !engines.is_empty(),
        "引擎发现",
        describe_candidates(&engines, &failures),
    );
    if engines.is_empty() {
        let endpoint = ContainerEndpoint::Unix {
            path: PathBuf::from(DEFAULT_SOCKET),
        };
        return match probe_explicit(ContainerEngineKind::Docker, &endpoint).await {
            Ok(engine) => {
                report.pass("显式端点探测", engine.info.version.clone());
                Some(engine)
            }
            Err(error) => {
                report.fail("显式端点探测", error.to_string());
                None
            }
        };
    }
    let index = engines
        .iter()
        .position(|engine| engine.info.engine.kind == ContainerEngineKind::Docker)
        .unwrap_or(0);
    let mut engines = engines;
    let engine = engines.swap_remove(index);
    report.pass(
        "探测与版本协商",
        format!(
            "种类 {}，端点 {}，版本 {}，API {}，平台 {}，CLI 兜底 {}",
            engine.info.engine.kind,
            engine.info.engine.endpoint.describe(),
            engine.info.version,
            engine.info.api_version,
            engine.info.platform,
            engine.info.cli_backend
        ),
    );
    Some(engine)
}

fn describe_candidates(engines: &[Engine], failures: &[String]) -> String {
    format!(
        "可用 {} 个：{}；不可用 {} 项：{}",
        engines.len(),
        engines
            .iter()
            .map(|engine| format!(
                "{}({})",
                engine.info.engine.kind,
                engine.info.engine.endpoint.describe()
            ))
            .collect::<Vec<_>>()
            .join("、"),
        failures.len(),
        if failures.is_empty() {
            "无".to_string()
        } else {
            failures.join("、")
        }
    )
}

/// 列表与详情：验证全量拉取、名称解析、挂载与原始 JSON 落地。
async fn list_stage(engine: &Engine, container: &str, report: &mut Report) -> Option<()> {
    let items = step(report, "容器列表", engine.list_containers(), |items| {
        summarize_containers(items)
    })
    .await?;
    let target = require_container(report, &items, container)?;
    let details = step(
        report,
        "容器详情",
        engine.inspect_container(&target.name),
        summarize_details,
    )
    .await?;
    report.check(
        details.state == ContainerState::Running,
        "详情状态与列表一致",
        format!("{} 状态 {}", details.name, details.state.key()),
    );
    Some(())
}

fn summarize_containers(items: &[ContainerSummary]) -> String {
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(format!(
            "{}[{}]({})",
            item.name,
            item.state.key(),
            item.compose_project
                .clone()
                .unwrap_or_else(|| "非编排".to_string())
        ));
    }
    format!("共 {} 个：{}", items.len(), parts.join(" "))
}

fn summarize_details(details: &ContainerDetails) -> String {
    format!(
        "{}：镜像 {}，命令 {}，挂载 {} 个，环境变量 {} 个，原始 JSON {} 字节",
        details.name,
        details.image,
        details.command,
        details.mounts.len(),
        details.env.len(),
        details.inspect.len()
    )
}

/// 取目标容器；缺失时记一条失败，避免整阶段静默跳过。
fn require_container(
    report: &mut Report,
    items: &[ContainerSummary],
    name: &str,
) -> Option<ContainerSummary> {
    match items.iter().find(|item| item.name == name) {
        Some(item) => Some(item.clone()),
        None => {
            report.fail(
                "目标容器存在",
                format!("未发现名为 {name} 的容器，夹具未准备"),
            );
            None
        }
    }
}

fn stats_line(stats: &ContainerStats) -> String {
    format!(
        "CPU {:.2}%，内存 {}/{}（{:.1}%），网络 rx={} tx={}，块读/写 {}/{}，进程 {}",
        stats.cpu_percent,
        stats.memory_used,
        stats.memory_limit,
        stats.memory_percent,
        stats.network_rx,
        stats.network_tx,
        stats.block_read,
        stats.block_write,
        stats.pids
    )
}

/// 统计：两次采样验证差分可用，且百分比落在合法区间。
async fn stats_stage(engine: &Engine, container: &str, report: &mut Report) -> Option<()> {
    let cancel = Arc::new(AtomicBool::default());
    let first = step(
        report,
        "统计采样",
        engine.container_stats(container, &cancel),
        stats_line,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let second = step(
        report,
        "统计二次采样",
        engine.container_stats(container, &cancel),
        stats_line,
    )
    .await?;
    report.check(
        second.cpu_percent.is_finite()
            && (0.0..=100.0).contains(&second.cpu_percent)
            && (0.0..=100.0).contains(&second.memory_percent),
        "统计取值合理",
        format!(
            "CPU {} -> {}，内存 {} -> {}",
            first.cpu_percent, second.cpu_percent, first.memory_used, second.memory_used
        ),
    );
    Some(())
}

/// 日志：先取全量尾部（含 stderr 分流标记），再验证跟随与取消。
async fn logs_stage(engine: &Engine, container: &str, report: &mut Report) -> Option<()> {
    let cancel = Arc::new(AtomicBool::default());
    let mut tail_text = String::new();
    let outcome = {
        let mut sink = |bytes: Vec<u8>| {
            tail_text.push_str(&String::from_utf8_lossy(&bytes));
        };
        timed(
            "读取容器日志",
            stream_logs(engine, container, 0, false, false, &cancel, &mut sink),
        )
        .await
    };
    match outcome {
        Ok(()) => report.check(
            tail_text.contains("hello-stdout") && tail_text.contains("[stderr] hello-stderr"),
            "日志全量与 stderr 分流",
            format!(
                "读到 {} 字节，首行 {}，末行 {}",
                tail_text.len(),
                first_line(tail_text.as_bytes()),
                tail_text.lines().next_back().unwrap_or("(空)")
            ),
        ),
        Err(error) => report.fail("读取容器日志", error.to_string()),
    }
    timestamped_logs(engine, container, report).await;
    follow_logs(engine, container, report).await;
    Some(())
}

/// 时间戳由引擎附带：真机上响应里每行都以 RFC3339 时间开头。
async fn timestamped_logs(engine: &Engine, container: &str, report: &mut Report) {
    let cancel = Arc::new(AtomicBool::default());
    let mut text = String::new();
    let outcome = {
        let mut sink = |bytes: Vec<u8>| {
            text.push_str(&String::from_utf8_lossy(&bytes));
        };
        timed(
            "日志时间戳",
            stream_logs(engine, container, 3, false, true, &cancel, &mut sink),
        )
        .await
    };
    match outcome {
        Ok(()) => {
            let first = first_line(text.as_bytes());
            let stamped = first.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                && first.contains('T')
                && !text.contains("[stderr] 2");
            report.check(stamped, "日志时间戳", format!("首行 {first:?}"));
        }
        Err(error) => report.fail("日志时间戳", error.to_string()),
    }
}

/// 跟随模式：容器主进程每秒打一行 `tick-N`，验证流打开后新日志能持续送达。
///
/// 为什么不用 exec 造日志：`docker logs` 只收容器主进程（PID 1）的输出，exec 的
/// 标准输出不进日志流；用 tick 循环才能真正验证跟随，`tick-N` 递增还能区分
/// 「尾部回放」与「实时追加」。
async fn follow_logs(engine: &Engine, container: &str, report: &mut Report) {
    let cancel = Arc::new(AtomicBool::default());
    let ticks = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink_ticks = ticks.clone();
    let mut sink = move |bytes: Vec<u8>| {
        let mut guard = lock_unpoisoned(&sink_ticks);
        for line in String::from_utf8_lossy(&bytes).lines() {
            if line.starts_with("tick-") && !guard.contains(&line.to_string()) {
                guard.push(line.to_string());
            }
        }
    };
    // 看门狗：4 秒后取消，既给出实时追加的观察窗口，也验证取消能及时收尾。
    let watchdog_cancel = cancel.clone();
    let watchdog = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        watchdog_cancel.store(true, Ordering::Release);
    });
    let outcome = timed(
        "日志跟随",
        stream_logs(engine, container, 1, true, false, &cancel, &mut sink),
    )
    .await;
    watchdog.abort();
    drop(sink);
    match outcome {
        Ok(()) => {
            let seen = lock_unpoisoned(&ticks).clone();
            report.check(
                seen.len() >= 2,
                "日志跟随与取消",
                format!("捕获 {} 行递增日志：{}", seen.len(), seen.join(" ")),
            );
        }
        Err(error) => report.fail("日志跟随", error.to_string()),
    }
}

/// 取锁并忽略中毒标记：冒烟程序里没有需要因他人 panic 而中止的共享不变量。
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// exec 终端：验证建立、尺寸调整、输出回传与结束通知。
async fn exec_stage(engine: &Engine, container: &str, report: &mut Report) -> Option<()> {
    let connection = ContainerConnection {
        container: container.to_string(),
        command: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo smoke-exec-ok".to_string(),
        ]),
        env: vec!["SMOKE_ENV=1".to_string()],
        tty: true,
        ..Default::default()
    };
    let (io, exit) = step(
        report,
        "exec 终端建立",
        open_exec(engine, &connection, TerminalSize::new(120, 32)),
        |_| "读写通道与控制句柄已建立".to_string(),
    )
    .await?;
    report.check(
        io.control.resize(TerminalSize::new(100, 24)),
        "exec 尺寸调整",
        "120x32 -> 100x24 控制消息已投递",
    );
    let output = drain_reader(io).await;
    report.check(
        String::from_utf8_lossy(&output).contains("smoke-exec-ok"),
        "exec 输出回传",
        format!("读到 {} 字节：{}", output.len(), first_line(&output)),
    );
    let ended = tokio::time::timeout(Duration::from_secs(3), exit).await;
    report.check(
        matches!(ended, Ok(Ok(()))),
        "exec 结束通知",
        format!("结束信号 {:?}", ended.map(|inner| inner.is_ok())),
    );
    stdin_echo(engine, container, report).await;
    Some(())
}

/// 把阻塞读搬进 `spawn_blocking`，读到 EOF 后返回全部输出。
async fn drain_reader(mut io: TerminalIo) -> Vec<u8> {
    // 读半区被取走后 TerminalIo 仍要存活：用空读器占位，控制与清理句柄不提前析构。
    let reader = std::mem::replace(&mut io.reader, Box::new(std::io::empty()));
    let taken = tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buffer = Vec::new();
        let _ = reader.read_to_end(&mut buffer);
        buffer
    })
    .await;
    taken.unwrap_or_default()
}

/// 非 TTY 的 `cat`：写入的字节必须原样回显，Close 后命令正常退出。
async fn stdin_echo(engine: &Engine, container: &str, report: &mut Report) {
    let connection = ContainerConnection {
        container: container.to_string(),
        command: Some(vec!["cat".to_string()]),
        tty: false,
        ..Default::default()
    };
    let opened = timed(
        "exec 写入通路",
        open_exec(engine, &connection, TerminalSize::new(80, 24)),
    )
    .await;
    let (mut io, _exit) = match opened {
        Ok(pair) => pair,
        Err(error) => {
            report.fail("exec 写入通路", error.to_string());
            return;
        }
    };
    if io.writer.write_all(b"smoke-stdin\n").is_err() {
        report.fail("exec 写入通路", "写入通道已关闭");
        return;
    }
    // 留一拍让写任务把已入队的字节送进容器，再关闭标准输入，避免与回显竞态。
    tokio::time::sleep(Duration::from_millis(200)).await;
    io.control.close();
    let output = drain_reader(io).await;
    report.check(
        String::from_utf8_lossy(&output).contains("smoke-stdin"),
        "exec 写入回显",
        format!("回显 {} 字节", output.len()),
    );
}

/// 归档 API：列目录、预览、上传、下载校验、重命名、删除。
async fn files_stage(
    engine: &Engine,
    container: &str,
    workdir: &Path,
    report: &mut Report,
) -> Option<()> {
    if let Err(error) = std::fs::create_dir_all(workdir) {
        report.fail("本地临时目录", error.to_string());
        return None;
    }
    let body = format!("transom-smoke-payload-{}", std::process::id());
    let source = workdir.join("upload-src.txt");
    if let Err(error) = std::fs::write(&source, body.as_bytes()) {
        report.fail("本地待上传文件", error.to_string());
        return None;
    }

    step(
        report,
        "容器列目录",
        list_dir(engine, container, REMOTE_DIR),
        |entries| summarize_entries(entries),
    )
    .await?;
    let preview = step(
        report,
        "容器文件预览",
        read_file(engine, container, "/etc/hosts", 4096),
        |(bytes, truncated, size)| {
            format!(
                "读到 {} 字节（声明大小 {size}，截断 {truncated}），首行 {}",
                bytes.len(),
                first_line(bytes)
            )
        },
    )
    .await?;
    report.check(!preview.0.is_empty(), "预览内容非空", "已取回正文");

    let dir = format!("{REMOTE_DIR}/transom-smoke-dir");
    step(
        report,
        "新建容器目录",
        create_dir(engine, container, &dir, &Arc::new(AtomicBool::default())),
        |_| format!("{dir} 已创建"),
    )
    .await?;
    // 新目录可能残留上一次运行的子目录，这里只要求能列出来（列目录走同一个归档接口）。
    step(
        report,
        "新目录可见",
        list_dir(engine, container, &dir),
        |entries| format!("新目录内 {} 项", entries.len()),
    )
    .await?;
    let nested = format!("{dir}/nested");
    step(
        report,
        "递归建子目录",
        create_dir(engine, container, &nested, &Arc::new(AtomicBool::default())),
        |_| format!("{nested} 已创建"),
    )
    .await?;

    let remote = format!("{REMOTE_DIR}/transom-smoke.txt");
    let cancel = Arc::new(AtomicBool::default());
    let (progress_tx, progress_rx) = flume::bounded::<(u64, Option<u64>)>(32);
    let uploaded = step(
        report,
        "上传到容器",
        upload(
            engine,
            container,
            &remote,
            &source,
            cancel.clone(),
            progress_tx,
        ),
        |size| format!("写入 {size} 字节"),
    )
    .await?;
    report.check(
        uploaded as usize == body.len(),
        "上传字节一致",
        format!("本地 {} 字节，容器 {uploaded} 字节", body.len()),
    );
    drain_progress(&progress_rx, report);

    let after_upload = step(
        report,
        "上传后列目录",
        list_dir(engine, container, REMOTE_DIR),
        |entries| summarize_entries(entries),
    )
    .await?;
    report.check(
        entry_present(&after_upload, &remote),
        "上传条目可见",
        format!("{remote} 已出现在 {REMOTE_DIR}"),
    );

    let downloaded = workdir.join("downloaded.txt");
    let (progress_tx, progress_rx) = flume::bounded::<(u64, Option<u64>)>(32);
    let read_back = step(
        report,
        "下载到本地",
        download(
            engine,
            container,
            &remote,
            &downloaded,
            cancel.clone(),
            progress_tx,
        ),
        |size| format!("落地 {size} 字节"),
    )
    .await?;
    let local_body = std::fs::read_to_string(&downloaded).unwrap_or_default();
    report.check(
        local_body == body && read_back as usize == body.len(),
        "下载内容一致",
        format!("本地读到 {local_body:?}"),
    );
    drain_progress(&progress_rx, report);

    let renamed = format!("{REMOTE_DIR}/transom-smoke-renamed.txt");
    step(
        report,
        "容器重命名",
        rename_path(engine, container, &remote, &renamed, &cancel),
        |_| format!("{remote} -> {renamed}"),
    )
    .await?;
    let after_rename = step(
        report,
        "重命名后列目录",
        list_dir(engine, container, REMOTE_DIR),
        |entries| summarize_entries(entries),
    )
    .await?;
    report.check(
        entry_present(&after_rename, &renamed) && !entry_present(&after_rename, &remote),
        "重命名条目一致",
        format!("{renamed} 存在且原名已消失"),
    );

    step(
        report,
        "删除容器文件",
        remove_path(engine, container, &renamed, false, &cancel),
        |_| format!("{renamed} 已删除"),
    )
    .await?;
    let after_remove = step(
        report,
        "删除后列目录",
        list_dir(engine, container, REMOTE_DIR),
        |entries| summarize_entries(entries),
    )
    .await?;
    report.check(
        !entry_present(&after_remove, &renamed),
        "删除条目一致",
        format!("{renamed} 已消失"),
    );
    step(
        report,
        "删除整个目录",
        remove_path(engine, container, &dir, true, &cancel),
        |_| format!("{dir} 已递归删除"),
    )
    .await?;
    let after_rmdir = step(
        report,
        "目录删除后列表",
        list_dir(engine, container, REMOTE_DIR),
        |entries| format!("剩余 {} 项", entries.len()),
    )
    .await?;
    report.check(
        !entry_present(&after_rmdir, &dir),
        "目录删除一致",
        format!("{dir} 已消失"),
    );
    Some(())
}

fn summarize_entries(entries: &[ContainerFileEntry]) -> String {
    let mut parts = Vec::with_capacity(entries.len().min(8));
    for entry in entries.iter().take(8) {
        parts.push(format!(
            "{}{}({})",
            entry.name,
            kind_mark(entry.kind),
            entry.size
        ));
    }
    format!("共 {} 项：{}", entries.len(), parts.join(" "))
}

fn entry_present(entries: &[ContainerFileEntry], path: &str) -> bool {
    entries.iter().any(|entry| entry.path == path)
}

fn kind_mark(kind: ContainerFileKind) -> &'static str {
    match kind {
        ContainerFileKind::Directory => "/",
        ContainerFileKind::File => "",
        ContainerFileKind::Symlink => "@",
        ContainerFileKind::Other => "?",
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("(空)")
        .to_string()
}

/// 进度只用于确认「有样本产生」：未被消费的样本按设计丢弃，不能拖慢传输。
fn drain_progress(rx: &flume::Receiver<(u64, Option<u64>)>, report: &mut Report) {
    let mut samples = 0usize;
    while let Ok((done, total)) = rx.try_recv() {
        samples += 1;
        let _ = (done, total);
    }
    report.pass("传输进度样本", format!("收到 {samples} 个样本"));
}

/// 镜像：列表、详情、重复拉取（验证进度事件与「已是最新」路径）。
async fn image_stage(engine: &Engine, image: &str, report: &mut Report) -> Option<()> {
    let items = step(report, "镜像列表", engine.list_images(), |images| {
        format!(
            "共 {} 个：{}",
            images.len(),
            images
                .iter()
                .map(|image| image.display_name())
                .collect::<Vec<_>>()
                .join(" ")
        )
    })
    .await?;
    report.check(
        !items.is_empty(),
        "镜像列表非空",
        format!("{} 条", items.len()),
    );
    let details = step(report, "镜像详情", engine.inspect_image(image), |info| {
        format!(
            "{}：大小 {}，层 {}，{}/{}",
            short_id(&info.id),
            info.size,
            info.layers,
            info.os.clone().unwrap_or_default(),
            info.architecture.clone().unwrap_or_default()
        )
    })
    .await?;
    report.check(
        !details.tags.is_empty(),
        "镜像标签落地",
        details.tags.join("、"),
    );
    let mut last = 0u64;
    let pulled = timed(
        "拉取镜像",
        engine.pull_image(image, &mut |done, _total| {
            last = done;
        }),
    )
    .await;
    match pulled {
        Ok(()) => report.pass("拉取镜像", format!("重复拉取成功，进度末值 {last}")),
        Err(error) => report.fail("拉取镜像", error.to_string()),
    }
    Some(())
}

/// 数据卷：创建、详情、删除。
async fn volume_stage(engine: &Engine, report: &mut Report) {
    let name = "transom-smoke-vol2";
    if timed("创建数据卷", engine.create_volume(name, None))
        .await
        .is_err()
    {
        report.fail("创建数据卷", "引擎拒绝了创建请求");
        return;
    }
    report.pass("创建数据卷", name.to_string());
    let details = step(
        report,
        "数据卷详情",
        engine.inspect_volume(name),
        |info| {
            format!(
                "{}：驱动 {}，挂载点 {}，使用容器 {} 个",
                info.summary.name,
                info.summary.driver,
                info.summary.mountpoint,
                info.used_by.len()
            )
        },
    )
    .await;
    if details.is_none() {
        return;
    }
    match timed("删除数据卷", engine.remove_volume(name, false)).await {
        Ok(()) => report.pass("删除数据卷", name.to_string()),
        Err(error) => report.fail("删除数据卷", error.to_string()),
    }
}

/// 网络：创建、详情、删除。
async fn network_stage(engine: &Engine, report: &mut Report) {
    let name = "transom-smoke-net2";
    if timed("创建网络", engine.create_network(name, None))
        .await
        .is_err()
    {
        report.fail("创建网络", "引擎拒绝了创建请求");
        return;
    }
    report.pass("创建网络", name.to_string());
    let details = step(
        report,
        "网络详情",
        engine.inspect_network(name),
        |info| {
            format!(
                "{}：驱动 {}，作用域 {}，子网 {:?}，网关 {:?}，连接容器 {} 个",
                info.summary.name,
                info.summary.driver,
                info.summary.scope,
                info.summary.subnet,
                info.gateway,
                info.connected.len()
            )
        },
    )
    .await;
    if details.is_none() {
        return;
    }
    match timed("删除网络", engine.remove_network(name)).await {
        Ok(()) => report.pass("删除网络", name.to_string()),
        Err(error) => report.fail("删除网络", error.to_string()),
    }
}

/// Compose：按容器标签聚合，验证项目名、计数与工作目录。
async fn compose_stage(engine: &Engine, project: &str, report: &mut Report) -> Option<()> {
    let projects = step(
        report,
        "编排项目列表",
        engine.compose_projects(),
        |projects| {
            format!(
                "共 {} 个：{}",
                projects.len(),
                projects
                    .iter()
                    .map(|item| format!("{}({})", item.name, item.state_label()))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        },
    )
    .await?;
    let matched = projects.iter().find(|item| item.name == project);
    let Some(matched) = matched else {
        report.fail("编排聚合命中", format!("未发现项目 {project}，夹具未准备"));
        return None;
    };
    report.check(
        matched.total >= 2 && matched.working_dir.is_some() && !matched.config_files.is_empty(),
        "编排聚合正确",
        format!(
            "{project}：{} 运行/{} 总数，工作目录 {:?}，配置 {:?} 个",
            matched.running,
            matched.total,
            matched.working_dir,
            matched.config_files.len()
        ),
    );
    Some(())
}

/// 路径防线：相对路径、逃逸与控制字符必须被拒绝，合法路径要归一化。
fn path_guard_stage(report: &mut Report) {
    for path in ["../etc/passwd", "/tmp/../etc/shadow", "/tmp/a\u{0}b"] {
        match validate_container_path(path) {
            Ok(normalized) => report.fail(
                "非法路径拦截",
                format!("{path:?} 被放行并规范化为 {normalized:?}"),
            ),
            Err(error) => report.pass("非法路径拦截", format!("{path:?} 被拒绝：{error}")),
        }
    }
    // 口径：含 `..` 的路径整条拒绝（本项目已定的输入洗化策略），这里只验证
    // 冗余分隔能被压平，不承诺对 `..` 做词法规范化。
    let legal = validate_container_path("/tmp//sub/file.txt")
        .unwrap_or_else(|error| format!("规范化失败：{error}"));
    report.check(
        legal == "/tmp/sub/file.txt",
        "合法路径规范化",
        format!("得到 {legal:?}"),
    );
}

/// 生命周期：停止与启动都要落到引擎状态上，最后让容器回到运行态。
async fn lifecycle_stage(engine: &Engine, container: &str, report: &mut Report) -> Option<()> {
    let items = step(
        report,
        "停止前列表",
        engine.list_containers(),
        |items| summarize_containers(items),
    )
    .await?;
    let target = require_container(report, &items, container)?;
    step(
        report,
        "停止容器",
        engine.lifecycle(&target.name, ContainerLifecycleAction::Stop),
        |_| format!("{} 已停止", target.name),
    )
    .await?;
    let stopped = step(
        report,
        "停止后状态",
        engine.inspect_container(&target.name),
        |details| format!("状态 {}", details.state.key()),
    )
    .await?;
    report.check(
        stopped.state == ContainerState::Exited,
        "停止状态一致",
        format!("{} 现为 {}", stopped.name, stopped.state.key()),
    );
    step(
        report,
        "启动容器",
        engine.lifecycle(&target.name, ContainerLifecycleAction::Start),
        |_| format!("{} 已启动", target.name),
    )
    .await?;
    let started = step(
        report,
        "启动后状态",
        engine.inspect_container(&target.name),
        |details| format!("状态 {}", details.state.key()),
    )
    .await?;
    report.check(
        started.state == ContainerState::Running,
        "启动状态一致",
        format!("{} 现为 {}", started.name, started.state.key()),
    );
    Some(())
}

/// 最小日志落盘器：冒烟程序不引入应用层的日志配置。
fn init_logging() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        struct StderrLogger;
        impl log::Log for StderrLogger {
            fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
                true
            }

            fn log(&self, record: &log::Record<'_>) {
                eprintln!("[{:?}] {}", record.level(), record.args());
            }

            fn flush(&self) {}
        }
        let _ = log::set_logger(&StderrLogger);
        let verbose = matches!(
            std::env::var("RUST_LOG").as_deref(),
            Ok("debug") | Ok("trace")
        );
        log::set_max_level(if verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        });
    });
}

/// 运行容器：创建 + 启动 + 名称冲突 + 清理，覆盖「把镜像跑起来」这条主路径。
async fn run_stage(engine: &Engine, image: &str, report: &mut Report) {
    let name = "transom-smoke-run";
    let spec = ContainerCreateSpec {
        name: name.to_string(),
        image: image.to_string(),
        command: vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo created-by-transom; sleep 120".to_string(),
        ],
        env: vec!["TRANSOM_SMOKE=1".to_string()],
        ports: vec![ContainerPortPublish {
            host: 18080,
            container: 80,
            protocol: "tcp".to_string(),
        }],
        binds: vec!["transom-smoke-vol:/mnt/data".to_string()],
        restart: Some("no".to_string()),
    };
    let mut check = spec.clone();
    report.check(
        check.validate().is_ok(),
        "运行参数校验",
        format!("合法 spec 通过校验，镜像 {}", check.image),
    );
    let mut blank = ContainerCreateSpec::default();
    report.check(
        blank.validate().is_err(),
        "空镜像被拒绝",
        blank
            .validate()
            .err()
            .unwrap_or_else(|| "未拒绝空镜像".to_string()),
    );

    let created = step(
        report,
        "创建容器",
        engine.create_container(&spec),
        |id| format!("新容器 {}", short_id(id)),
    )
    .await;
    let Some(id) = created else {
        return;
    };
    let conflict = engine.create_container(&spec).await;
    report.check(
        matches!(&conflict, Err(ContainerError::Api { status: 409, .. })),
        "同名容器返回冲突",
        format!("{:?}", conflict.err().map(|error| error.to_string())),
    );

    step(
        report,
        "启动新容器",
        engine.lifecycle(&id, ContainerLifecycleAction::Start),
        |_| format!("{} 已启动", name),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let details = step(
        report,
        "新容器详情",
        engine.inspect_container(&id),
        summarize_details,
    )
    .await;
    if let Some(details) = details {
        report.check(
            details.state == ContainerState::Running
                && details.env.contains(&"TRANSOM_SMOKE=1".to_string()),
            "新容器状态与环境",
            format!(
                "状态 {}，环境变量 {:?}，端口映射 {} 个，挂载 {} 个",
                details.state.key(),
                details.env,
                details.ports.len(),
                details.mounts.len()
            ),
        );
    }
    step(
        report,
        "删除新容器",
        engine.lifecycle(&id, ContainerLifecycleAction::Remove { force: true }),
        |_| format!("{} 已删除", name),
    )
    .await;
}
