//! 全工作区源码结构与异步任务生命周期守卫。

use crate::common::{impl_blocks, normalized_path, walkdir_like};
use crate::i18n::blank_test_modules;
use std::path::Path;

/// 粗略提取具名函数体并统计物理行数。
fn function_line_count(source: &str, name: &str) -> Option<usize> {
    let marker = format!("fn {name}(");
    let start = source.find(&marker)?;
    let open = source[start..].find('{')? + start;
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(source[start..open + offset + 1].lines().count());
                }
            }
            _ => {}
        }
    }
    None
}

/// 工作区生产源码：只扫描 `apps/*/src` 与 `crates/*/src`。
fn workspace_source_files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    for base in ["apps", "crates"] {
        for entry in walkdir_like(&root.join(base)) {
            let path = normalized_path(&entry);
            if !path.contains("/src/") || !path.ends_with(".rs") {
                continue;
            }
            if path.ends_with("/tests.rs")
                || path.ends_with("_tests.rs")
                || path.contains("/tests/")
            {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&entry) {
                out.push((path, content));
            }
        }
    }
    out
}

/// 生产区域不得超过 500 个非空行，包含测试的物理文件不得超过 800 行。
#[test]
fn workspace_sources_respect_line_limits() {
    let mut offenders = Vec::new();
    for (path, content) in workspace_source_files() {
        let production_lines = blank_test_modules(&content)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        let total_lines = content.lines().count();
        if production_lines > 500 || total_lines > 800 {
            offenders.push(format!(
                "{path}: 生产非空 {production_lines} 行，总计 {total_lines} 行"
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "生产区域超过 500 个非空行或总文件超过 800 行，必须先按关注点拆分：\n{}",
        offenders.join("\n")
    );
}

/// `Drop` 实现不得派生异步任务；同步析构必须登记资源并交给显式收尾路径。
#[test]
fn drop_impls_do_not_spawn_tasks() {
    let mut offenders = Vec::new();
    for (path, content) in workspace_source_files() {
        let production = blank_test_modules(&content);
        for body in impl_blocks(&production, "impl Drop for") {
            if body.contains("tokio::spawn") || body.contains("tokio::task::spawn") {
                offenders.push(path.clone());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "以下 Drop 实现派生了异步任务，必须改为可追踪的显式清理队列：{offenders:?}"
    );
}

/// 生产代码不得用 `let _ = tokio::spawn(...)` 丢弃任务句柄。
#[test]
fn production_code_does_not_discard_spawn_handles() {
    let mut offenders = Vec::new();
    for (path, content) in workspace_source_files() {
        let production = blank_test_modules(&content);
        for (index, line) in production.lines().enumerate() {
            let line = line.trim();
            if line.contains("let _ = tokio::spawn(")
                || line.contains("let _ = tokio::task::spawn(")
            {
                offenders.push(format!("{path}:{}: {line}", index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "生产代码不得丢弃异步任务句柄：\n{}",
        offenders.join("\n")
    );
}

/// 控制器主循环必须轮询所有常驻任务集合，避免已完成任务长期滞留。
#[test]
fn controller_polls_all_background_task_sets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = std::fs::read_to_string(root.join("apps/transom/src/controller/mod.rs"))
        .expect("读取控制器主循环失败");
    for set in ["cleanup_set", "git_set"] {
        assert!(
            content.contains(&format!("state.{set}.join_next()")),
            "控制器主循环必须轮询 {set}，及时回收已完成任务并观察 panic"
        );
    }
    assert!(
        content.contains("state.sync_task.as_mut()"),
        "控制器主循环必须轮询云同步任务，完成或 panic 后清理任务句柄"
    );
}

/// 终端阻塞读写线程必须有拥有型句柄，并通过有界回收器统一 join。
#[test]
fn terminal_io_threads_are_reaped() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tasks = std::fs::read_to_string(root.join("crates/transom-terminal/src/view/tasks.rs"))
        .expect("读取终端 I/O 任务失败");
    let view = std::fs::read_to_string(root.join("crates/transom-terminal/src/view/mod.rs"))
        .expect("读取终端视图失败");
    assert!(
        tasks.contains("struct TerminalIoThreads")
            && tasks.contains("TERMINAL_THREAD_QUEUE_CAPACITY")
            && tasks.contains("reap_io_threads"),
        "终端读写线程必须由 TerminalIoThreads 持有并交给有界回收器"
    );
    assert!(
        view.contains("_reader_thread: Option<JoinHandle<()>>")
            && view.contains("_writer_thread: Option<JoinHandle<()>>"),
        "TerminalView 必须持有读写线程句柄"
    );
    let drop_impl = view
        .find("impl Drop for TerminalView")
        .map(|start| &view[start..])
        .expect("缺少 TerminalView Drop");
    assert!(
        drop_impl.contains("reap_io_threads"),
        "TerminalView 销毁时必须把读写线程交给回收器"
    );
}

/// UI 阻塞 IO 必须使用进程级固定线程池与有界队列，禁止每次调用裸建线程。
#[test]
fn blocking_io_pool_is_bounded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let runtime = std::fs::read_to_string(root.join("crates/transom-ui/src/app/runtime.rs"))
        .expect("读取 UI 阻塞 IO 调度失败");
    for required in [
        "OnceLock",
        "BLOCKING_QUEUE_CAPACITY",
        "MAX_BLOCKING_WORKERS",
        "send_async",
    ] {
        assert!(
            runtime.contains(required),
            "UI 阻塞 IO 调度缺少固定线程池约束: {required}"
        );
    }
}

/// WebSocket 日志必须使用有界虚拟列表，禁止回退到全量字符串重建。
#[test]
fn websocket_log_uses_bounded_virtual_list() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let log = std::fs::read_to_string(
        root.join("crates/transom-ui/src/api_client/workspace/ws_log_view.rs"),
    )
    .expect("读取 WebSocket 日志视图失败");
    for required in [
        "MAX_WS_LOG_LINES",
        "MAX_WS_LOG_BYTES",
        "v_virtual_list",
        "scroll_to_item",
    ] {
        assert!(
            log.contains(required),
            "WebSocket 日志视图缺少有界虚拟列表约束: {required}"
        );
    }
}

/// SFTP 本地预览与属性读取必须走阻塞 IO 池，避免卡住 UI 线程。
#[test]
fn sftp_local_metadata_uses_blocking_pool() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "crates/transom-ui/src/sftp/preview_actions.rs",
        "crates/transom-ui/src/sftp/properties.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("读取 {relative} 失败: {error}"));
        assert!(
            content.contains("spawn_blocking_local"),
            "SFTP 本地 IO 必须在阻塞池执行: {relative}"
        );
    }
}

/// 窗口请求队列必须声明容量上限并支持可覆盖请求合并。
#[test]
fn window_requests_are_bounded_and_coalesced() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content =
        std::fs::read_to_string(root.join("crates/transom-ui/src/app/window_requests.rs"))
            .expect("读取窗口请求队列失败");
    for required in ["WINDOW_REQUEST_LIMIT", "is_coalescible", "retain("] {
        assert!(
            content.contains(required),
            "窗口请求队列缺少有界或合并约束: {required}"
        );
    }
}

/// 审计点名的长函数必须保持拆分后的规模，禁止重新长回单体函数。
#[test]
fn audit_hotspot_functions_stay_split() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (relative, function, limit) in [
        (
            "crates/transom-ui/src/forward/form.rs",
            "render_forward_add_form",
            120,
        ),
        (
            "crates/transom-ui/src/forward/row.rs",
            "render_forward_rule_row",
            80,
        ),
        (
            "crates/transom-ui/src/session_form/defaults.rs",
            "seed_from_init",
            110,
        ),
        (
            "crates/transom-ui/src/dialog_host/mod.rs",
            "render_body",
            60,
        ),
        (
            "crates/transom-ui/src/api_client/workspace/tree_view.rs",
            "render_tree",
            45,
        ),
        (
            "crates/transom-ui/src/app/container_requests.rs",
            "handle_container_request",
            50,
        ),
    ] {
        let source = std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("读取 {relative} 失败: {error}"));
        let lines = function_line_count(&source, function)
            .unwrap_or_else(|| panic!("{relative} 缺少函数 {function}"));
        assert!(
            lines <= limit,
            "{relative}::{function} 当前 {lines} 行，必须保持在 {limit} 行以内"
        );
    }
}

/// UI 搜索必须经过共享匹配器，禁止重新手写逐项小写副本。
#[test]
fn ui_search_uses_shared_matcher() {
    let mut offenders = Vec::new();
    for (path, content) in workspace_source_files() {
        if !path.contains("/crates/transom-ui/src/") || path.ends_with("/search.rs") {
            continue;
        }
        let production = blank_test_modules(&content);
        for (index, line) in production.lines().enumerate() {
            if line.contains(".to_lowercase().contains(")
                || line.contains(".to_ascii_lowercase().contains(")
            {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "UI 搜索必须复用 crate::search，不得逐项创建小写副本：\n{}",
        offenders.join("\n")
    );
}
