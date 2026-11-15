//! 源码守卫公共工具函数。

use std::path::{Path, PathBuf};

/// 简易目录递归（避免为此引入 walkdir 依赖）。
pub(crate) fn walkdir_like(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// 统一使用 `/` 作为源码路径分隔符，避免 Windows 的反斜杠使后缀白名单失效。
pub(crate) fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn source_guard_paths_use_forward_slashes() {
    assert_eq!(
        normalized_path(Path::new(r"C:\repo\transom-ui\src\settings\mod.rs")),
        "C:/repo/transom-ui/src/settings/mod.rs"
    );
}

pub(crate) fn ui_source_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for base in [
        "crates/transom-ui/src",
        "crates/transom-ui-kit/src",
        "crates/transom-terminal/src",
    ] {
        for entry in walkdir_like(&root.join(base)) {
            let path = normalized_path(&entry);
            if path.ends_with("tests.rs") {
                continue;
            }
            if entry.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(content) = std::fs::read_to_string(&entry)
            {
                out.push((path, content));
            }
        }
    }
    out
}

pub(crate) fn collect_rust_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("读取源码目录失败") {
        let path = entry.expect("读取源码目录项失败").path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

pub(crate) fn impl_blocks<'a>(content: &'a str, header: &str) -> Vec<&'a str> {
    let mut blocks = Vec::new();
    let mut search = 0;
    while let Some(relative) = content[search..].find(header) {
        let start = search + relative;
        let Some(body_start) = content[start..].find('{') else {
            break;
        };
        let body = &content[start + body_start..];
        let mut depth = 0usize;
        let mut end = 0usize;
        for (offset, ch) in body.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        blocks.push(&body[..end]);
        search = start + body_start + end + 1;
    }
    blocks
}
