//! 系统 git 子进程的统一调用通道。
//!
//! 设计原理（Why）：
//! - 全部子命令走这一条路径，才能把「超时、杀进程、环境变量、凭据提示屏蔽、
//!   锁冲突识别、日志脱敏」这些约束一次性固化下来；散落在各处调用会漏掉
//!   最关键的一条——任何一次未设超时的 `git fetch` 都可能让任务永久挂起；
//! - 环境变量的选择决定了本层可自动化：`GIT_TERMINAL_PROMPT=0` 与空
//!   `GIT_ASKPASS` 保证 git 在需要凭据时直接失败而不是把 stdin 挂住，
//!   `LC_ALL=C` 保证 stderr 文案稳定（否则中文系统会回传中文 fatal），
//!   `GIT_OPTIONAL_LOCKS=0` 让只读命令不去创建 `index.lock`，避免刷新状态
//!   时与用户的命令行 git 抢锁；
//! - 输出按字节返回而不是先转字符串：porcelain v2 的路径字段是原始字节，
//!   可能含空格、非 UTF-8 文件名，转成 `String` 会丢信息。
//!
//! 代价与局限：
//! - 依赖运行环境装有 git；未安装时以 [`GitError::Spawn`] 上报，界面提示安装；
//! - 子进程启动开销约几毫秒，高频刷新靠调用方节流，本层不做缓存。

use crate::error::GitError;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::RwLock;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

static GIT_PROGRAM: RwLock<String> = RwLock::new(String::new());

/// 全局 Git 提交作者配置（姓名与邮箱）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitAuthorConfig {
    pub name: String,
    pub email: String,
}

static GIT_AUTHOR: RwLock<GitAuthorConfig> = RwLock::new(GitAuthorConfig {
    name: String::new(),
    email: String::new(),
});

/// 配置全局提交作者信息（当 name 或 email 非空时在底层 git 命令中注入对应环境变量）。
pub fn set_git_author(name: impl Into<String>, email: impl Into<String>) {
    if let Ok(mut w) = GIT_AUTHOR.write() {
        w.name = name.into();
        w.email = email.into();
    }
}

/// 获取当前配置的 Git 提交作者信息。
pub fn git_author() -> GitAuthorConfig {
    if let Ok(r) = GIT_AUTHOR.read() {
        return r.clone();
    }
    GitAuthorConfig::default()
}

/// 配置底层 Git 命令的可执行文件路径（为空或默认时回退到系统 `"git"`）。
pub fn set_git_program(path: impl Into<String>) {
    if let Ok(mut w) = GIT_PROGRAM.write() {
        *w = path.into();
    }
}

/// 获取当前生效的 Git 可执行文件路径。
pub fn git_program() -> String {
    if let Ok(r) = GIT_PROGRAM.read()
        && !r.trim().is_empty()
    {
        return r.clone();
    }
    "git".to_string()
}

/// 只读查询的默认超时：本地仓库的 status/log 远小于此，超过即视为仓库异常。
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// 子命令运行参数。
#[derive(Debug, Clone)]
pub(crate) struct RunOptions {
    /// 超时时长，到期即强杀子进程。
    pub timeout: Duration,
    /// 只读命令：追加 `GIT_OPTIONAL_LOCKS=0`，不去创建可选锁文件。
    pub read_only: bool,
    /// 交互式变基的「待办清单编辑器」替身：给出一个已经写好内容的清单文件，
    /// git 会用它覆盖自己的 todo。设置后追加 `GIT_SEQUENCE_EDITOR=cp <文件>`。
    pub sequence_todo: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            read_only: true,
            sequence_todo: None,
        }
    }
}

impl RunOptions {
    /// 会修改仓库状态的命令：保留锁语义，让并发写按 git 自己的规则互斥。
    pub(crate) fn write() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            read_only: false,
            sequence_todo: None,
        }
    }

    /// 带自定义待办清单的写命令（交互式变基的唯一入口）。
    ///
    /// 超时放宽到 300 秒：变基要逐个重放提交，几十个提交的仓库在慢盘上
    /// 远超一次普通查询的耗时，用默认的 30 秒会把正常变基判成故障并强杀，
    /// 留下一个停在中间状态的仓库。
    pub(crate) fn with_todo(todo: PathBuf) -> Self {
        Self {
            timeout: Duration::from_secs(300),
            sequence_todo: Some(todo),
            ..Self::write()
        }
    }
}

/// 一次子进程调用的完整结果。
pub(crate) struct GitOutput {
    /// stdout 原始字节（porcelain 输出以 NUL/LF 分隔，不做任何转码）。
    pub stdout: Vec<u8>,
    /// stderr 已按 UTF-8 宽松解码，仅用于诊断与错误分类。
    pub stderr: String,
    /// 退出码；被信号终止时为 `-1`，与 git 自身的失败区分开。
    pub code: i32,
}

impl GitOutput {
    /// stdout 是否成功且非空。
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// 按行切分 stdout（用于明确以 LF 分隔的 `--format` 输出）。
    pub fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// 构造一次 git 子进程调用：环境变量与 stdio 的唯一出处。
///
/// 设计原理（Why）：一次性调用的 `run` 与逐行流式的 `clone` 必须共用这一份口径。
/// 少复制一条 `GIT_TERMINAL_PROMPT=0`，克隆遇到私有仓库就会把 stdin 挂住而不是
/// 直接失败，而这种差异只在真实网络上偶发、单测覆盖不到；把环境集中到一个函数里，
/// 两条路径就不可能各说各话。
pub(crate) fn build_command(
    cwd: &Path,
    args: &[&str],
    opts: &RunOptions,
) -> Result<Command, GitError> {
    let mut command = Command::new(git_program());
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // 关掉终端与图形凭据提示：需要凭据时直接失败，绝不把 stdin 挂住。
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GIT_SSH_ASKPASS", "")
        // 稳定 stderr 语言：中文系统的 fatal 文案会随 locale 变化，无法据此分类。
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        // 杜绝任何分页与编辑器接管输出。
        .env("GIT_PAGER", "cat")
        .env("GIT_EDITOR", "true");
    let author = git_author();
    if !author.name.trim().is_empty() {
        command.env("GIT_AUTHOR_NAME", author.name.trim());
        command.env("GIT_COMMITTER_NAME", author.name.trim());
    }
    if !author.email.trim().is_empty() {
        command.env("GIT_AUTHOR_EMAIL", author.email.trim());
        command.env("GIT_COMMITTER_EMAIL", author.email.trim());
    }
    if let Some(todo) = &opts.sequence_todo {
        // git 通过 shell 调用序列编辑器并把自身的 todo 路径追加为最后一个参数，
        // 于是 `cp <我们的清单>` 正好覆盖它。路径含单引号会让这条 shell 语句变形，
        // 这里直接拒绝——清单文件由本模块自己创建，出现引号只可能是环境被篡改。
        let raw = todo.display().to_string();
        if raw.contains('\'') {
            return Err(GitError::InvalidPath {
                path: raw,
                reason: "待办清单路径含单引号，无法安全传给 git 的序列编辑器".into(),
            });
        }
        command.env("GIT_SEQUENCE_EDITOR", format!("cp '{raw}'"));
    }
    if opts.read_only {
        // 只读命令不创建可选锁，避免与用户命令行 git 抢 index.lock。
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    Ok(command)
}

/// 执行一次 git 调用，非零退出码返回 [`GitError::Failed`]。
pub(crate) async fn run(
    cwd: &Path,
    args: &[&str],
    opts: RunOptions,
) -> Result<GitOutput, GitError> {
    let output = run_allow_fail(cwd, args, opts).await?;
    if output.ok() {
        return Ok(output);
    }
    Err(failure(cwd, args, output))
}

/// 执行一次 git 调用，非零退出码也原样返回（用于「存在性探测」类命令）。
pub(crate) async fn run_allow_fail(
    cwd: &Path,
    args: &[&str],
    opts: RunOptions,
) -> Result<GitOutput, GitError> {
    let mut command = build_command(cwd, args, &opts)?;
    let display = display_command(args);
    let mut child = command.spawn().map_err(|source| GitError::Spawn {
        command: display.clone(),
        source,
    })?;

    // 超时必须覆盖「等待退出 + 读完管道」全过程：管道读满或子进程卡在
    // hook 脚本里都会让 wait_with_output 永久挂起。
    let pipes = match tokio::time::timeout(opts.timeout, read_pipes(&mut child)).await {
        Ok(Ok(pipes)) => pipes,
        Ok(Err(source)) => {
            let _ = child.start_kill();
            return Err(GitError::Spawn {
                command: display,
                source,
            });
        }
        Err(_) => {
            let _ = child.start_kill();
            log::warn!(
                "git 命令超时被强制终止，命令: git {display}，超时秒数: {}",
                opts.timeout.as_secs()
            );
            return Err(GitError::Timeout {
                command: display,
                secs: opts.timeout.as_secs(),
            });
        }
    };
    let status = child.wait().await.map_err(|source| GitError::Spawn {
        command: display.clone(),
        source,
    })?;

    Ok(GitOutput {
        stdout: pipes.0,
        stderr: String::from_utf8_lossy(&pipes.1).trim().to_string(),
        code: exit_code(&status),
    })
}

/// 并发读尽 stdout 与 stderr。
///
/// 分开读两个管道会死锁：子进程写满 stderr 缓冲区（约 64KB）后阻塞，
/// 而父进程还在等 stdout 读完。`wait_with_output` 内部就是并发读，
/// 但它在超时时会消费掉 `Child`，导致无法再 `start_kill`，因此这里自行并发。
async fn read_pipes(child: &mut tokio::process::Child) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out = async {
        match stdout {
            Some(mut pipe) => read_to_end_bounded(&mut pipe).await,
            None => Ok(Vec::new()),
        }
    };
    let err = async {
        match stderr {
            Some(mut pipe) => read_to_end_bounded(&mut pipe).await,
            None => Ok(Vec::new()),
        }
    };
    let (out, err) = tokio::join!(out, err);
    Ok((out?, err?))
}

/// 输出上限：远超正常查询体量，只为防住被攻陷的 hook 把内存打满。
const MAX_PIPE_BYTES: usize = 64 * 1024 * 1024;

async fn read_to_end_bounded<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        if buf.len() + read > MAX_PIPE_BYTES {
            return Err(std::io::Error::other("git 输出超过单次读取上限"));
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    Ok(buf)
}

/// 退出码；被信号杀死时 `ExitStatus::code()` 为 `None`，统一记为 `-1`。
pub(crate) fn exit_code(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

/// 把非零退出转成具体错误：锁冲突与非仓库目录单独成类，其余归为命令失败。
fn failure(cwd: &Path, args: &[&str], output: GitOutput) -> GitError {
    let stderr = output.stderr.clone();
    if stderr.contains(".lock") && stderr.contains("fatal") {
        return GitError::Locked {
            path: cwd.display().to_string(),
        };
    }
    if stderr.contains("not a git repository") {
        return GitError::NotRepo {
            path: cwd.display().to_string(),
        };
    }
    GitError::Failed {
        command: display_command(args),
        code: output.code,
        stderr: if stderr.is_empty() {
            "（无错误输出）".to_string()
        } else {
            stderr
        },
    }
}

/// 供日志与错误信息使用的命令摘要，已对 URL 内联凭据脱敏。
pub(crate) fn display_command(args: &[&str]) -> String {
    args.iter()
        .map(|arg| redact_url(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 遮蔽 `scheme://user:token@host` 形态的凭据，避免令牌进日志与界面。
fn redact_url(arg: &str) -> String {
    let Some(scheme_end) = arg.find("://") else {
        return arg.to_string();
    };
    let rest = &arg[scheme_end + 3..];
    match rest.find('@') {
        Some(at) if !rest[..at].contains('/') => {
            format!("{}***@{}", &arg[..scheme_end + 3], &rest[at + 1..])
        }
        _ => arg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn redacts_inline_credentials_only() {
        assert_eq!(
            redact_url("https://user:tok@github.com/a/b.git"),
            "https://***@github.com/a/b.git"
        );
        assert_eq!(
            redact_url("https://github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(redact_url("origin"), "origin");
        // 路径里出现 @ 不算凭据，不能误伤。
        assert_eq!(redact_url("file:///srv/@repo"), "file:///srv/@repo");
    }

    #[test]
    fn author_and_program_config_roundtrip() {
        super::set_git_program("/usr/bin/git-custom");
        assert_eq!(super::git_program(), "/usr/bin/git-custom");
        super::set_git_program("");
        assert_eq!(super::git_program(), "git");

        super::set_git_author("测试作者", "test@example.com");
        let a = super::git_author();
        assert_eq!(a.name, "测试作者");
        assert_eq!(a.email, "test@example.com");
        super::set_git_author("", "");
    }
}
