//! 私有 Git 仓库端到端加密同步。
//!
//! 设计原理（Why）：
//! - 直接调用系统 `git`，而不是在进程内重写 Git 协议：SSH 密钥、agent、
//!   known_hosts、HTTPS 凭据助手都能按用户既有配置工作，握手与对象格式的
//!   兼容性由 git 自己保证；
//! - 仓库里只有一个加密信封文件（`transom-sync.envelope`），分支固定为
//!   `transom-sync`，与用户自己的提交历史完全隔离；
//! - 每次同步都是"拉取密文 -> 解密 -> 三路合并 -> 重新加密 -> 提交推送"，
//!   共同祖先（上一次成功同步的目录）也以密文保存在本地缓存目录，重启后
//!   仍能做三路合并；
//! - 推送被并发设备抢先时（non-fast-forward），自动重新拉取、重新合并，
//!   最多重试三次。
//!
//! 代价与局限：
//! - 只同步会话/文件夹/凭据模板，不同步系统钥匙串里的口令（与备份默认口径
//!   一致；Git 仓库通常比对象存储更容易被多人看到）；
//! - 依赖宿主机安装 `git`，缺失时返回明确配置错误。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use transom_core::{
    AppError, ErrorCode, GitSyncSettings, SessionCatalog, SyncMergeReport, merge_catalogs,
    private_fs,
};
use transom_crypto::envelope::{is_envelope, open, seal};
use transom_store::write_atomic;

/// 仓库内保存加密信封的文件名。
pub const GIT_SYNC_FILE: &str = "transom-sync.envelope";
/// 本地缓存中保存共同祖先的文件名（同样是密文）。
const GIT_BASE_FILE: &str = "base.envelope";
/// Git 子命令超时：大仓库首次拉取可能较慢，但不能无限挂起。
const GIT_TIMEOUT: Duration = Duration::from_secs(180);
/// 推送冲突后的最大重试次数（含首次）。
const GIT_MAX_ATTEMPTS: usize = 3;

/// Git 同步的认证信息。
#[derive(Default)]
pub struct GitAuth {
    /// HTTPS Personal Access Token；SSH 远端忽略。
    pub token: Option<zeroize::Zeroizing<String>>,
    /// HTTPS 用户名；缺省用 `x-access-token`（GitHub/GitLab 均接受）。
    pub username: Option<String>,
}

/// 一次 Git 同步的结果。
#[derive(Debug)]
pub struct GitSyncOutcome {
    /// 合并后的完整目录，调用方应整体替换本地目录。
    pub catalog: SessionCatalog,
    pub report: SyncMergeReport,
    /// 是否真的产生了新提交并推送；内容无变化时为 false。
    pub pushed: bool,
    pub bytes: u64,
    pub at: i64,
}

/// 执行一次完整的 Git 同步（阻塞）。
///
/// 必须在 `spawn_blocking` 中调用：Git 子进程与 Argon2 派生都是阻塞操作。
///
/// # Errors
/// 未安装 git、远端不可达、认证失败、密文损坏/口令错误、推送连续冲突时
/// 返回 `AppError`，错误文本不包含任何凭据。
pub fn sync_git_blocking(
    settings: &GitSyncSettings,
    local: &SessionCatalog,
    passphrase: &str,
    cache_dir: &Path,
    auth: &GitAuth,
) -> Result<GitSyncOutcome, AppError> {
    let mut last_error = None;
    for _ in 0..GIT_MAX_ATTEMPTS {
        match attempt_sync(settings, local, passphrase, cache_dir, auth) {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                log::warn!(
                    "Git 同步本轮失败，准备重试，分支: {}，原因: {}",
                    settings.branch,
                    error
                );
                last_error = Some(error);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| AppError::with_detail(ErrorCode::CloudSyncFailed, "Git 同步失败")))
}

fn attempt_sync(
    settings: &GitSyncSettings,
    local: &SessionCatalog,
    passphrase: &str,
    cache_dir: &Path,
    auth: &GitAuth,
) -> Result<GitSyncOutcome, AppError> {
    ensure_repo(cache_dir, &settings.remote, auth)?;
    let remote_bytes = fetch_remote(cache_dir, &settings.branch, auth)?;
    let remote_catalog = match &remote_bytes {
        Some(bytes) => decode_catalog(bytes, passphrase)?,
        None => SessionCatalog::default(),
    };
    let base = load_base(cache_dir, passphrase);
    let (merged, report) = merge_catalogs(base.as_ref(), local, &remote_catalog);
    let plaintext = serde_json::to_vec(&merged).map_err(|error| {
        AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("序列化同步目录失败: {error}"),
        )
    })?;
    // 信封每次加密都带随机盐与 nonce，不能直接比较密文字节；比较的是
    // 解密后的目录内容。数组顺序按设备本地顺序保存，两台设备可能不同，
    // 因此按 ID 集合比较，内容一致就不产生空提交。
    if remote_bytes.is_some() && catalogs_equivalent(&remote_catalog, &merged) {
        let envelope = seal(&plaintext, passphrase)?;
        write_atomic(&cache_dir.join(GIT_BASE_FILE), &envelope)?;
        return Ok(GitSyncOutcome {
            catalog: merged,
            report,
            pushed: false,
            bytes: envelope.len() as u64,
            at: chrono::Utc::now().timestamp(),
        });
    }
    let envelope = seal(&plaintext, passphrase)?;

    checkout_branch(cache_dir, &settings.branch, remote_bytes.is_some(), auth)?;
    write_atomic(&cache_dir.join(GIT_SYNC_FILE), &envelope)?;
    commit_and_push(cache_dir, &settings.branch, auth)?;
    write_atomic(&cache_dir.join(GIT_BASE_FILE), &envelope)?;
    let bytes = envelope.len() as u64;
    log::info!(
        "Git 同步完成，分支: {}，会话数: {}，新增: {}，更新: {}，冲突: {}，字节数: {}",
        settings.branch,
        merged.sessions.len(),
        report.sessions_added,
        report.sessions_updated,
        report.conflicts,
        bytes
    );
    Ok(GitSyncOutcome {
        catalog: merged,
        report,
        pushed: true,
        bytes,
        at: chrono::Utc::now().timestamp(),
    })
}

/// 按 ID 与内容指纹比较两个目录是否等价（忽略数组顺序）。
fn catalogs_equivalent(left: &SessionCatalog, right: &SessionCatalog) -> bool {
    fn fingerprint<T: serde::Serialize, K: std::hash::Hash + Eq>(
        items: &[T],
        key: impl Fn(&T) -> K,
    ) -> std::collections::HashMap<K, Vec<u8>> {
        items
            .iter()
            .map(|item| (key(item), serde_json::to_vec(item).unwrap_or_default()))
            .collect()
    }
    fingerprint(&left.sessions, |session| session.id)
        == fingerprint(&right.sessions, |session| session.id)
        && fingerprint(&left.folders, |folder| folder.id)
            == fingerprint(&right.folders, |folder| folder.id)
        && fingerprint(&left.identities, |identity| identity.id)
            == fingerprint(&right.identities, |identity| identity.id)
}

fn decode_catalog(bytes: &[u8], passphrase: &str) -> Result<SessionCatalog, AppError> {
    if !is_envelope(bytes) {
        return Err(AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            "远端同步文件不是加密信封；为避免明文泄露已拒绝读取",
        ));
    }
    let plaintext = open(bytes, passphrase)?;
    serde_json::from_slice(&plaintext).map_err(|error| {
        AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("同步信封内容无法解析: {error}"),
        )
    })
}

fn load_base(cache_dir: &Path, passphrase: &str) -> Option<SessionCatalog> {
    let bytes = fs::read(cache_dir.join(GIT_BASE_FILE)).ok()?;
    match decode_catalog(&bytes, passphrase) {
        Ok(catalog) => Some(catalog),
        Err(error) => {
            // 口令变更或缓存损坏：丢不掉用户数据，只是退回两路合并。
            log::warn!("本地同步基线无法解密，本次按两路合并处理: {error}");
            None
        }
    }
}

fn ensure_repo(cache_dir: &Path, remote: &str, auth: &GitAuth) -> Result<(), AppError> {
    // 缓存里是包含会话拓扑的密文与基线，目录必须只对属主可见；
    // 已有目录不改权限，文件侧仍由原子写收紧到 0600。
    private_fs::ensure_private_dir(cache_dir).map_err(|error| {
        AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("创建 Git 同步缓存目录失败: {error}"),
        )
    })?;
    if !cache_dir.join(".git").exists() {
        run_git(cache_dir, &["init", "-q"], auth)?;
    }
    let current = run_git(cache_dir, &["remote", "get-url", "origin"], auth);
    match current {
        Ok(output) if output.status.success() => {
            let existing = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if existing != remote {
                run_git(cache_dir, &["remote", "set-url", "origin", remote], auth)?;
            }
        }
        _ => {
            run_git(cache_dir, &["remote", "add", "origin", remote], auth)?;
        }
    }
    Ok(())
}

fn fetch_remote(
    cache_dir: &Path,
    branch: &str,
    auth: &GitAuth,
) -> Result<Option<Vec<u8>>, AppError> {
    let remote_ref = format!("refs/heads/{branch}");
    let heads = run_git(
        cache_dir,
        &["ls-remote", "--heads", "origin", &remote_ref],
        auth,
    )?;
    if !heads.status.success() || heads.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    run_git(
        cache_dir,
        &["fetch", "-q", "--depth=1", "origin", &remote_ref],
        auth,
    )?;
    let show = run_git(
        cache_dir,
        &["show", &format!("FETCH_HEAD:{GIT_SYNC_FILE}")],
        auth,
    )?;
    if !show.status.success() {
        return Ok(None);
    }
    Ok(Some(show.stdout))
}

fn checkout_branch(
    cache_dir: &Path,
    branch: &str,
    has_remote: bool,
    auth: &GitAuth,
) -> Result<(), AppError> {
    if has_remote {
        run_git(
            cache_dir,
            &["checkout", "-q", "-f", "-B", branch, "FETCH_HEAD"],
            auth,
        )?;
    } else {
        // 空仓库首次同步：直接建立孤立分支，不继承用户可能残留的提交。
        let _ = run_git(cache_dir, &["checkout", "-q", "--orphan", branch], auth);
    }
    Ok(())
}

fn commit_and_push(cache_dir: &Path, branch: &str, auth: &GitAuth) -> Result<(), AppError> {
    run_git(cache_dir, &["add", "--", GIT_SYNC_FILE], auth)?;
    let commit = run_git(
        cache_dir,
        &[
            "-c",
            "user.name=Transom Sync",
            "-c",
            "user.email=transom-sync@localhost",
            "commit",
            "-q",
            "-m",
            "chore(sync): 更新加密会话信封",
        ],
        auth,
    )?;
    if !commit.status.success() {
        let detail = String::from_utf8_lossy(&commit.stderr);
        // 内容无变化时 git 会以非零退出；此时直接跳过推送即可。
        if detail.contains("nothing to commit") {
            return Ok(());
        }
        return Err(git_error("提交同步信封", &detail));
    }
    let push_ref = format!("HEAD:refs/heads/{branch}");
    let push = run_git(cache_dir, &["push", "-q", "origin", &push_ref], auth)?;
    if !push.status.success() {
        return Err(git_error(
            "推送同步分支（可能已被其他设备抢先，将自动重试）",
            &String::from_utf8_lossy(&push.stderr),
        ));
    }
    Ok(())
}

fn run_git(cache_dir: &Path, args: &[&str], auth: &GitAuth) -> Result<Output, AppError> {
    // 用 coreutils 的 timeout 包住 git：它负责超时强杀与子进程收割，
    // `output()` 同时并发排空 stdout/stderr，避免大信封写满管道造成死锁。
    let mut command = Command::new("timeout");
    command
        .arg("-k")
        .arg("5s")
        .arg(format!("{}s", GIT_TIMEOUT.as_secs()))
        .arg("git")
        .arg("-C")
        .arg(cache_dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(token) = &auth.token {
        // 凭据只进环境变量，不进命令行：`ps` 看不到 token。
        let username = auth.username.as_deref().unwrap_or("x-access-token");
        let encoded = BASE64.encode(format!("{username}:{}", token.as_str()));
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Basic {encoded}"),
            );
    }
    command.env(
        "GIT_SSH_COMMAND",
        "ssh -o BatchMode=yes -o ConnectTimeout=15",
    );

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::with_detail(
                ErrorCode::Configuration,
                "未找到 git 或 timeout 命令，请先安装 Git 后再使用仓库同步",
            )
        } else {
            AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                format!("启动 git 失败: {error}"),
            )
        }
    })?;
    if output.status.code() == Some(124) {
        return Err(AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("git 操作超时（{} 秒），已终止子进程", GIT_TIMEOUT.as_secs()),
        ));
    }
    Ok(output)
}

fn git_error(action: &str, stderr: &str) -> AppError {
    let detail: String = stderr.trim().chars().take(600).collect();
    AppError::with_detail(
        ErrorCode::CloudSyncFailed,
        format!("{action}失败: {detail}"),
    )
}
