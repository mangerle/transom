//! API 领域与存储层的结构守卫。

use crate::common::{normalized_path, walkdir_like};
use std::path::Path;

fn api_source_files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for base in ["crates/transom-api/src", "crates/transom-store/src"] {
        for entry in walkdir_like(&root.join(base)) {
            let normalized = normalized_path(&entry);
            if normalized.ends_with("tests.rs") || normalized.ends_with("/tests.rs") {
                continue;
            }
            if entry.extension().and_then(|extension| extension.to_str()) == Some("rs")
                && let Ok(content) = std::fs::read_to_string(&entry)
            {
                files.push((normalized, content));
            }
        }
    }
    files
}

#[test]
fn api_production_files_stay_below_500_lines() {
    let mut offenders = Vec::new();
    for (path, content) in api_source_files() {
        let production_lines = content
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .count();
        if production_lines > 500 {
            offenders.push(format!("{path}: {production_lines} 行"));
        }
    }
    assert!(
        offenders.is_empty(),
        "API 与存储生产文件超过 500 行，必须按职责拆分:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn api_and_store_use_only_bounded_channels() {
    let mut offenders = Vec::new();
    for (path, content) in api_source_files() {
        for (index, line) in content.lines().enumerate() {
            if line.contains("unbounded_channel")
                || line.contains("crossbeam_channel::unbounded")
                || line.contains("flume::unbounded")
            {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "API 与存储层禁止无界通道:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn ui_depends_on_ai_contracts_not_full_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = std::fs::read_to_string(root.join("crates/transom-ui/Cargo.toml"))
        .expect("读取 transom-ui 清单失败");
    assert!(
        !manifest
            .lines()
            .any(|line| line.trim() == "transom-ai.workspace = true"),
        "transom-ui 不得依赖完整 transom-ai 运行时"
    );

    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("crates/transom-ui/src")) {
        let path = normalized_path(&entry);
        if entry.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains("transom_ai::") {
            offenders.push(path);
        }
    }
    assert!(
        offenders.is_empty(),
        "UI 只能依赖 transom-ai-contracts，不得使用运行时命名空间:\n{}",
        offenders.join("\n")
    );
}
