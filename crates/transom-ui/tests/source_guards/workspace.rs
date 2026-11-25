//! 全工作区源码结构与异步任务生命周期守卫。

use crate::common::{impl_blocks, normalized_path, walkdir_like};
use crate::i18n::blank_test_modules;
use std::path::Path;

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
