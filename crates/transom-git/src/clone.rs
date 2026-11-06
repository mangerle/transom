//! 从远端克隆仓库，并把进度逐行交给调用方。
//!
//! 设计原理（Why）：
//! - 克隆是唯一可能跑几分钟的 git 动作，`process::run` 那种「收完再返回」的形态
//!   在这里等于界面上一动不动，所以本模块单独走管道逐行读；
//! - `--progress` 必须显式给：git 只在 stderr 是终端时才输出进度，非终端时整个
//!   克隆过程一声不响，正好是最需要反馈的时候；
//! - 只读 stderr：`git clone` 的进度与提示全部走 stderr，stdout 没有语义，直接
//!   关掉它省一路管道，也省掉并发读两路时的取整与死锁风险；
//! - 空闲超时而不是总时长超时：几个 G 的仓库合法地要跑很久，但两分钟没有任何
//!   新输出基本就是网络断了或凭据挂住，必须收；
//! - 取消要真的杀进程：用户关掉克隆标签页后还让 git 继续写盘，等于留下半个仓库
//!   加一个孤儿进程；`kill_on_drop` 只是兜底，这里显式 `start_kill`；
//! - 参数表用 `--` 截断：URL 由用户给，若以 `-` 开头会被 git 当成选项
//!   （`--upload-pack=` 那一类），那是命令注入而不是笔误。
//!
//! 代价与局限：
//! - 进度按行交出，git 用 `\\r` 刷同一行的百分比，这里按换行切分，所以百分比
//!   刷新可能成段到达；调用方（控制器）负责按时间窗合并，避免把事件通道打满。

use crate::error::GitError;
use crate::process::{RunOptions, build_command, display_command, exit_code};
use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, BufReader};

/// 空闲超时：连续这么久没有任何新输出就判定挂起并强杀。
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// 取消轮询间隔：关闭标签页最迟 300 毫秒后子进程被杀，不等下一行输出。
const CANCEL_POLL: Duration = Duration::from_millis(300);
/// 失败时随错误一起上报的尾部行数：够看清 git 的最后几句，又不至于刷屏。
const TAIL_LINES: usize = 12;

/// 克隆一个远端仓库到 `dest`，每读到一行文本就调用一次 `on_line`。
///
/// `url` 与 `dest` 都来自用户输入：`dest` 的父目录必须已存在（git 只创建最后一级，
/// 父级不存在时它报的是一句定位不到的错），`url` 走 `--` 截断防止被当成选项。
/// `depth` 为 `Some(n)` 时只拉最近 n 个提交（浅克隆）。
///
/// # Errors
/// - [`GitError::InvalidPath`]：`dest` 没有父目录或父目录不存在；
/// - [`GitError::Precondition`]：`depth` 为 0；
/// - [`GitError::Spawn`]：git 无法启动（未安装、权限不足）；
/// - [`GitError::Timeout`]：`IDLE_TIMEOUT` 内没有任何新输出；
/// - [`GitError::Cancelled`]：`cancel` 置位，子进程已被强杀；
/// - [`GitError::Failed`]：git 以非零码退出，尾部输出附在错误里。
pub async fn clone_with_progress(
    url: &str,
    dest: &Path,
    depth: Option<u32>,
    cancel: &AtomicBool,
    mut on_line: impl FnMut(&str),
) -> Result<(), GitError> {
    if depth == Some(0) {
        return Err(GitError::Precondition {
            reason: "浅克隆深度必须大于 0".into(),
        });
    }
    let parent = dest.parent().ok_or_else(|| GitError::InvalidPath {
        path: dest.display().to_string(),
        reason: "目标目录没有上级目录，无法作为克隆位置".into(),
    })?;
    if !parent.is_dir() {
        return Err(GitError::InvalidPath {
            path: parent.display().to_string(),
            reason: "克隆位置的上级目录不存在".into(),
        });
    }
    let dest_text = dest.display().to_string();
    let depth_text = depth.map(|value| value.to_string());
    let mut args: Vec<&str> = Vec::with_capacity(8);
    args.extend(["clone", "--progress"]);
    if let Some(value) = depth_text.as_deref() {
        args.extend(["--depth", value]);
    }
    args.extend(["--", url, dest_text.as_str()]);
    let display = display_command(&args);
    let mut command = build_command(parent, &args, &RunOptions::write())?;
    // 克隆的 stdout 没有语义，关掉它只留 stderr 一路管道。
    command.stdout(Stdio::null());
    let mut child = command.spawn().map_err(|source| GitError::Spawn {
        command: display.clone(),
        source,
    })?;
    let stderr = child.stderr.take().ok_or_else(|| GitError::Spawn {
        command: display.clone(),
        source: std::io::Error::other("无法取得 git clone 的标准错误输出"),
    })?;

    let mut stream = BufReader::new(stderr);
    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL_LINES);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    // 首轮 tick 立即返回，避免刚起来就被判成"已取消"。
    poll.tick().await;
    // 用 `read` 而不是 `next_line`/`read_line`：后者被 select 打断会丢掉已读进
    // 缓冲的半行（tokio 明确标注取消不安全），而 `read` 要么读到要么没读到，
    // 未成行的尾巴由这里的 `carry` 自己跨轮次保管。
    let mut chunk = [0u8; 4096];
    let mut carry: Vec<u8> = Vec::with_capacity(4096);
    loop {
        let read = tokio::select! {
            result = stream.read(&mut chunk) => match result {
                Ok(read) => read,
                Err(source) => {
                    let _ = child.start_kill();
                    return Err(GitError::Spawn { command: display, source });
                }
            },
            _ = tokio::time::sleep(IDLE_TIMEOUT) => {
                let _ = child.start_kill();
                log::warn!(
                    "git clone 长时间无输出被强制终止，命令: git {display}，空闲秒数: {}",
                    IDLE_TIMEOUT.as_secs()
                );
                return Err(GitError::Timeout {
                    command: display,
                    secs: IDLE_TIMEOUT.as_secs(),
                });
            }
            _ = poll.tick() => {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.start_kill();
                    log::info!("克隆已被用户取消，命令: git {display}");
                    return Err(GitError::Cancelled);
                }
                // 没取消就继续等下一批字节：这一轮什么都没读到，也不该丢缓冲。
                continue;
            }
        };
        if read == 0 {
            break;
        }
        carry.extend_from_slice(&chunk[..read]);
        // 按 `\n` 与 `\r` 双重切分：git 的进度用 `\r` 回写同一行，
        // 逐段交出去正好是"每次刷新算一条"，界面按时间窗合并即可。
        while let Some(index) = carry
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
        {
            let line = take_line(&mut carry, index);
            if line.is_empty() {
                continue;
            }
            remember(&mut tail, &line);
            on_line(&line);
        }
    }
    // 进程退出前可能留着一行没有结束符的输出（被中断的 fatal 常见），照实交出。
    if !carry.is_empty() {
        let line = String::from_utf8_lossy(&carry).trim().to_string();
        if !line.is_empty() {
            remember(&mut tail, &line);
            on_line(&line);
        }
    }

    let status = child.wait().await.map_err(|source| GitError::Spawn {
        command: display.clone(),
        source,
    })?;
    if status.success() {
        return Ok(());
    }
    Err(GitError::Failed {
        command: display,
        code: exit_code(&status),
        stderr: tail
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
    })
}

/// 取走 `carry` 里到 `index`（含分隔符）为止的一行文本。
fn take_line(carry: &mut Vec<u8>, index: usize) -> String {
    let bytes: Vec<u8> = carry.drain(..=index).collect();
    String::from_utf8_lossy(&bytes[..bytes.len() - 1])
        .trim_end()
        .to_string()
}

/// 记住尾部若干行：失败时把这几行原样带给界面，不做任何"猜原因"的加工。
fn remember(tail: &mut VecDeque<String>, line: &str) {
    if tail.len() == TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line.to_string());
}

#[cfg(test)]
mod tests {
    use super::clone_with_progress;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    /// 造一个带一次提交的本仓库，作为"远端"用 `file://` 克隆。
    fn seeded_source(dir: &std::path::Path) -> PathBuf {
        let source = dir.join("source");
        std::fs::create_dir_all(&source).expect("建源仓库目录");
        let run = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(&source)
                .output()
                .expect("执行 git");
            assert!(
                output.status.success(),
                "git {args:?} 失败: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-b", "main", "."]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "测试"]);
        std::fs::write(source.join("README.md"), b"hello\n").expect("写文件");
        run(&["add", "README.md"]);
        run(&["commit", "-m", "首个提交"]);
        source
    }

    #[tokio::test]
    async fn clone_writes_working_repo_and_reports_progress() {
        let dir = tempfile::tempdir().expect("临时目录");
        let source = seeded_source(dir.path());
        let dest = dir.path().join("work").join("cloned");
        std::fs::create_dir_all(dest.parent().expect("克隆位置的上级目录")).expect("建上级目录");
        let url = format!("file://{}", source.display());

        let mut lines: Vec<String> = Vec::new();
        let outcome = clone_with_progress(&url, &dest, None, &AtomicBool::new(false), |line| {
            lines.push(line.to_string());
        })
        .await;

        assert!(outcome.is_ok(), "本地克隆应当成功: {outcome:?}");
        assert!(
            dest.join(".git").is_dir(),
            "克隆完必须是一个工作仓库，目标: {}",
            dest.display()
        );
        assert!(
            dest.join("README.md").is_file(),
            "内容要跟着落盘，不能只有元数据"
        );
        // `Cloning into ...` 一定出现；断言"有输出"而不是"有百分比"，
        // 小仓库的进度块会被 git 省略。
        assert!(!lines.is_empty(), "克隆过程必须逐行交出输出");
        assert!(
            lines.iter().any(|line| line.contains("Cloning into")),
            "首行应是 git 自己的 Cloning into 提示，实际: {lines:?}"
        );
    }

    #[tokio::test]
    async fn missing_parent_directory_is_rejected_before_spawn() {
        let dir = tempfile::tempdir().expect("临时目录");
        let dest = dir.path().join("没有的父目录").join("repo");
        let error = clone_with_progress(
            "https://example.invalid/x.git",
            &dest,
            None,
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect_err("父目录不存在必须直接拒绝");
        assert!(
            matches!(error, super::GitError::InvalidPath { .. }),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn leading_dash_url_is_not_read_as_an_option() {
        let dir = tempfile::tempdir().expect("临时目录");
        let dest = dir.path().join("repo");
        // 参数表里有 `--`：这条 URL 会被当成位置参数（一个不存在的仓库地址）而失败，
        // 绝不能被 git 解析成 `--upload-pack` 之类的可执行选项。
        let cancel = AtomicBool::new(false);
        let error = clone_with_progress(
            "--upload-pack=touch /tmp/pwned",
            &dest,
            None,
            &cancel,
            |_| {},
        )
        .await
        .expect_err("非法地址必须失败");
        assert!(
            matches!(
                error,
                super::GitError::Failed { .. } | super::GitError::Spawn { .. }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn zero_depth_is_rejected_before_spawn() {
        let dir = tempfile::tempdir().expect("临时目录");
        let dest = dir.path().join("repo");
        let error = clone_with_progress(
            "https://example.invalid/x.git",
            &dest,
            Some(0),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect_err("深度 0 必须拒绝");
        assert!(
            matches!(error, super::GitError::Precondition { .. }),
            "{error:?}"
        );
    }
}
