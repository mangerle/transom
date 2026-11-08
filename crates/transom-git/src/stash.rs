//! 搁置（stash）：列表、新建、应用、弹出与删除。
//!
//! 设计原理（Why）：
//! - 一律用 `stash@{n}` 选择器而不是 SHA：选择器是用户在界面上看到的编号，
//!   也是 git 自己文档里的口径，换成 SHA 反而让日志与命令行对不上；
//! - 列表用 `--format` 自定义分隔符取回「选择器 + 标题 + 时间」三列，
//!   时间用 `%ci`（相对时间由界面算，这里给绝对时间以便排序）；
//! - `apply` 与 `pop` 分开暴露：前者保留栈项，后者弹出。IDEA 的「应用」和
//!   「多次应用」就是这两条，合成一个按钮会让人担心栈项被吃掉。

use crate::config::GitConfig;
use crate::error::GitError;
use crate::fields::{lossy, split_limited};
use crate::process::{RunOptions, run, run_allow_fail};
use std::collections::HashSet;
use std::path::Path;
use transom_core::{StashEntry, StashFile};

/// 字段分隔符（与日志侧同一套控制字符约定）。
const FIELD: u8 = 0x1f;
/// `git stash list` 的自定义格式：`%gd` 是 `stash@{n}` 选择器，`%gs` 是标题。
///
/// 注意 `git stash list` 只认 pretty 格式（`%gd`/`%gs`），不认 `for-each-ref`
/// 那套 `%(refname)` 占位符——实测会把占位符原样打印出来。
const FORMAT: &str = "--pretty=format:%x1e%gd%x1f%gs";

/// 列出全部搁置项（栈顶在前）。
pub(crate) async fn list(root: &Path, config: &GitConfig) -> Result<Vec<StashEntry>, GitError> {
    let output = run(
        root,
        config,
        &["stash", "list", FORMAT],
        RunOptions::default(),
    )
    .await?;
    let mut entries = Vec::new();
    for record in output.stdout.split(|b| *b == 0x1e) {
        // `--pretty=format:` 在记录之间插入换行，先剥掉这一个。
        let record = match record.split_first() {
            Some((b'\n', rest)) => rest,
            _ => record,
        };
        if record.is_empty() {
            continue;
        }
        let fields = split_limited(record, FIELD, 2);
        if fields.len() < 2 {
            return Err(GitError::Decode {
                command: "stash list".into(),
                reason: format!("搁置记录字段不足: {}", lossy(record)),
            });
        }
        let subject = lossy(fields[1]);
        entries.push(StashEntry {
            selector: lossy(fields[0]),
            message: subject
                .split_once(": ")
                .map_or_else(String::new, |(_, rest)| rest.to_string()),
            subject,
        });
    }
    Ok(entries)
}

/// 新建搁置项；`include_untracked` 为真时连未跟踪文件一起收。
pub(crate) async fn save(
    root: &Path,
    config: &GitConfig,
    message: &str,
    include_untracked: bool,
) -> Result<(), GitError> {
    let mut args: Vec<&str> = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if !message.trim().is_empty() {
        args.push("-m");
        args.push(message);
    }
    run(root, config, &args, RunOptions::write()).await?;
    Ok(())
}

/// 应用一个搁置项但保留栈项。
pub(crate) async fn apply(root: &Path, config: &GitConfig, selector: &str) -> Result<(), GitError> {
    run(
        root,
        config,
        &["stash", "apply", selector],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 应用并弹出栈项。
pub(crate) async fn pop(root: &Path, config: &GitConfig, selector: &str) -> Result<(), GitError> {
    run(
        root,
        config,
        &["stash", "pop", selector],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 丢弃一个搁置项（不可恢复，界面必须已二次确认）。
pub(crate) async fn drop(root: &Path, config: &GitConfig, selector: &str) -> Result<(), GitError> {
    run(
        root,
        config,
        &["stash", "drop", selector],
        RunOptions::write(),
    )
    .await?;
    Ok(())
}

/// 一个搁置项里的文件清单（含收进搁置的未跟踪文件）。
///
/// 设计原理（Why）：
/// - 用 `stash show --include-untracked --name-status -z`：不带 `--include-untracked`
///   时未跟踪部分整段缺席，界面会显示一份缺文件的搁置，比不显示更糟；`-z` 让
///   含空格与换行的路径可按 NUL 切分；
/// - 额外一次 `ls-tree stash@{n}^3` 判定哪些条目住在未跟踪侧：`A` 状态本身分不出
///   「先 add 再搁置」与「未跟踪被收进去」，而两者的内容位于不同的树，
///   猜错就是一张空差异。没有第三父提交时该命令非零退出，按空集处理。
pub(crate) async fn files(
    root: &Path,
    config: &GitConfig,
    selector: &str,
) -> Result<Vec<StashFile>, GitError> {
    let output = run(
        root,
        config,
        &[
            "stash",
            "show",
            "--include-untracked",
            "--name-status",
            "-z",
            selector,
        ],
        RunOptions::default(),
    )
    .await?;
    let entries =
        crate::log::parse_name_status(&output.stdout).map_err(|reason| GitError::Decode {
            command: format!("stash show {selector}"),
            reason,
        })?;
    let untracked = untracked_paths(root, config, selector).await?;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let untracked_flag = untracked.contains(&entry.path);
            StashFile {
                path: entry.path,
                old_path: entry.old_path,
                change: entry.change,
                untracked: untracked_flag,
            }
        })
        .collect())
}

/// 未跟踪侧（`<selector>^3`）的文件集合；该搁置没带未跟踪文件时为空。
async fn untracked_paths(
    root: &Path,
    config: &GitConfig,
    selector: &str,
) -> Result<HashSet<String>, GitError> {
    let spec = format!("{selector}^3");
    let output = run_allow_fail(
        root,
        config,
        &["ls-tree", "-r", "--name-only", "-z", &spec],
        RunOptions::default(),
    )
    .await?;
    if !output.ok() {
        return Ok(HashSet::new());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(lossy)
        .collect())
}
