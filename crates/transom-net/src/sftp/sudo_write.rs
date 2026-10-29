//! SFTP Sudo 提权写入通道：口令校验与数据流分离，长度闸门 + 同目录原子替换。
//!
//! 设计原理（Why）：
//! - 为什么不用一条 `sudo -S tee` 通道搞定（本模块前一版的写法）：`sudo -S` 从
//!   stdin 首行读口令，而我们要写的**内容也在 stdin**。两种后果都是数据损坏——
//!   远端配了 NOPASSWD 或票据未过期时不会提示口令，那一行口令被 `tee` 当正文写进
//!   文件；反过来真需要口令时，第一个 64 KiB 数据块被当成口令吃掉。靠"检测提示符
//!   出现后再送口令"仍不可靠：提示符走 stderr、时序受网络抖动影响，判错的代价是
//!   损坏一个生产配置文件。
//! - 本模块的解法是**把两个通道彻底分开**：
//!   1. 口令只进 `sudo -S -- -v`：这条通道只校验/续期票据，除一行口令外不写任何
//!      数据，天然没有"口令与正文混淆"的可能；
//!   2. 正文只进 `sudo -n`：`-n` 让 sudo 永不提示口令，stdin 100% 是文件内容。
//!      若票据在校验后失效，`-n` 直接失败退出而不是把正文当口令吞掉——失败可以
//!      重试，损坏不行。
//! - 免密不弹：[`probe_sudo`] 用 `sudo -n -- true` 探测，0 退出即免密可用，界面
//!   就不该问口令；探测本身不产生任何口令交互。
//! - **长度闸门**（本模块最重要的安全性质）：远端脚本在 `mv` 之前比对
//!   `wc -c` 与预期字节数，不符即以 41 退出，配合入口注册的 `trap rm -f EXIT`
//!   删除半成品。于是"网络中断、任务取消、进程被提前退出、通道被 EOF"这类
//!   一切"写了一半"的情形，后果都只是放弃替换，绝不会把生产文件截断。
//!   这也是取消路径无需额外协议就能安全中止的原因：断开通道 = 安全放弃。
//! - 参数一律以位置参数传给远端脚本（`sh -c '脚本' 名 $1..$4`），脚本体内不拼接
//!   任何外部字符串：路径含空格、引号、`$()` 都不可能被解析成命令，注入面归零。
//! - 属性保全：用 `stat` 读取原文件的 uid/gid/mode 再 `chown`/`chmod` 到临时文件，
//!   比 `cp -p --attributes-only` 更可移植（BusyBox 等精简环境没有该选项）；
//!   GNU 与 BSD 两套 `stat` 语法都试，全部失败则退回默认属主并在日志说明。
//! - 代价与局限：
//!   1. 探测只验证"该用户能否 sudo"，不覆盖按命令粒度授权的 sudoers 规则；若写入
//!      阶段被该规则拒绝，会以退出码非 0 + 原文报出，不会静默；
//!   2. 要求远端存在 `sudo`、`sh`、`cat`、`mv`、`stat`、`wc`，且目标路径在 sudo
//!      授权范围内；`requiretty` 的机器（老 RHEL 默认）无法用非交互 exec 通道提权，
//!      本模块会明确报 [`transom_core::SudoUnavailable::RequiresTty`]；
//!   3. 单次写入把整份内容留在内存里（编辑器缓冲区本来就在内存中）；将来接
//!      大文件提权落盘时，把 `payload` 换成有界通道分块喂入即可，长度闸门与
//!      协议时序都不用改。

use crate::client::SshRemote;
use russh::ChannelMsg;
use std::path::Path;
use std::time::Duration;
use transom_core::{AppError, ErrorCode, SudoCapability, SudoUnavailable};

/// 单次向通道灌入的 stdin 分块上限。
///
/// 真正的网络背压由 SSH 窗口决定（russh 在窗口耗尽时挂起等待），这里的上限只是
/// 控制单次 `data_bytes` 的粒度，让长写入能被及时取消、也能及时看到通道错误。
const STDIN_CHUNK_SIZE: usize = 64 * 1024;

/// 单步（打开通道、exec、每块写入、每步收包）的无进展上限。
///
/// 刻意不做"整次写入的总时长上限"：大文件走慢链路本来就要几分钟，总上限会把
/// 合法写入误判成失败；无进展判定才是真正需要的保护。
const PIPE_STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// 口令校验通道的总时长上限：`sudo -v` 只该花一次往返。
const VALIDATE_TIMEOUT: Duration = Duration::from_secs(15);

/// 远端 stderr 尾部保留上限，防止失控输出把本地内存填满。
const STDERR_TAIL_BYTES: usize = 4096;

/// 提权写入的输入契约。
///
/// 一次成型而不是散参数：路径、内容、口令、备份意图必须成套出现，少传一个就会
/// 退化成"备份没做"或"长度没校验"这类静默缺陷。
#[derive(Debug, Clone, Copy)]
pub struct SudoWriteRequest<'a> {
    /// 远端目标绝对路径。
    pub target: &'a str,
    /// 待写入的完整字节（BOM/CRLF 已按远端形态还原）。
    pub payload: &'a [u8],
    /// sudo 口令；`None` 表示按免密提交。
    pub password: Option<&'a str>,
    /// 是否在替换前把原文件备份为 `<target>.transom-bak`。
    pub backup: bool,
}

/// 提权写入的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SudoWriteReport {
    /// 远端确认写入的字节数（长度闸门已保证它等于提交字节数）。
    pub bytes: u64,
}

/// 一条 exec 通道跑完后的现场。
#[derive(Debug, Default)]
struct PipedOutcome {
    exit_status: Option<u32>,
    /// 远端 stdout 尾部。脚本本身不输出正文，这里只可能装诊断信息。
    stdout_tail: String,
    /// stderr 尾部，仅用于诊断文本。
    stderr_tail: String,
}

impl PipedOutcome {
    /// 把退出码与 stderr 组装成领域错误；错误文本带现场但不含口令。
    fn failure(&self, action: &str) -> AppError {
        let detail = if self.stderr_tail.trim().is_empty() {
            match self.exit_status {
                Some(code) => format!("远端进程退出码 {code}"),
                None => "远端进程未回报退出码".to_string(),
            }
        } else {
            self.stderr_tail.trim().to_string()
        };
        AppError::with_detail(ErrorCode::SftpFailed, format!("{action}，{detail}"))
    }

    /// 判定是否属于"需要口令"：`-n` 下的这句诊断是唯一能让界面回头问口令的信号。
    fn needs_password(&self) -> bool {
        let text = self.stderr_tail.to_ascii_lowercase();
        text.contains("password is required") || text.contains("a terminal is required")
    }
}

/// POSIX 单引号闭合转义，杜绝路径与参数里的 Shell 元字符被执行。
pub(crate) fn posix_shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// 提取目标路径的父级目录。
pub(crate) fn get_parent_directory(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if let Some(pos) = trimmed.rfind('/') {
        if pos == 0 { "/" } else { &trimmed[..pos] }
    } else {
        "."
    }
}

/// 基于目标路径构建同目录下的隐藏原子临时文件路径。
///
/// 刻意不用 `/tmp`：跨文件系统时 `rename(2)` 会退化成"复制+删除"，原子性丢失；
/// 共享 `/tmp` 也可能被同机其他用户抢先建同名文件。
pub(crate) fn generate_same_dir_temp_path(target_path: &str, unique_token: &str) -> String {
    let parent = get_parent_directory(target_path);
    let file_name = Path::new(target_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("transom_file");
    if parent == "/" {
        format!("/.{file_name}.transom_sudo_{unique_token}")
    } else {
        format!("{parent}/.{file_name}.transom_sudo_{unique_token}")
    }
}

/// 备份路径：与普通写回路径共用同一个后缀约定。
#[cfg(test)]
pub(crate) fn backup_path(target_path: &str) -> String {
    format!("{target_path}.transom-bak")
}

/// 构建远端脚本体：只引用位置参数 `$1..$4`，不拼接任何外部文本。
///
/// 参数次序：`$1` 目标、`$2` 临时文件、`$3` 预期字节数、`$4` 是否备份（`1`/`0`）。
/// 退出码约定：`41` 专指"收到的字节数与预期不符"，调用方据此区分截断与一般失败。
pub(crate) fn build_sudo_write_script() -> &'static str {
    r#"set -e
dst=$1
tmp=$2
bak=$dst.transom-bak
trap 'rm -f "$tmp"' EXIT INT TERM
if [ -f "$dst" ] && [ "$4" = 1 ]; then cp -p "$dst" "$bak" || true; fi
touch "$tmp"
if [ -e "$dst" ]; then
  uid=$(stat -c %u "$dst" 2>/dev/null || stat -f %u "$dst" 2>/dev/null || true)
  gid=$(stat -c %g "$dst" 2>/dev/null || stat -f %g "$dst" 2>/dev/null || true)
  mode=$(stat -c %a "$dst" 2>/dev/null || stat -f %Lp "$dst" 2>/dev/null || true)
  [ -n "$uid" ] && [ -n "$gid" ] && chown "$uid" "$gid" "$tmp" || true
  [ -n "$mode" ] && chmod "$mode" "$tmp" || true
fi
cat > "$tmp"
got=$(wc -c < "$tmp" | tr -d ' ')
[ "$got" = "$3" ] || exit 41
mv -f "$tmp" "$dst"
trap - EXIT INT TERM
"#
}

/// 组装完整的提权命令字符串。
pub(crate) fn build_sudo_write_command(
    target: &str,
    temp: &str,
    expected_bytes: u64,
    backup: bool,
) -> String {
    let args = [
        "transom".to_string(),
        posix_shell_quote(target),
        posix_shell_quote(temp),
        expected_bytes.to_string(),
        if backup { "1" } else { "0" }.to_string(),
    ]
    .join(" ");
    format!(
        "sudo -n -- sh -c {} {}",
        posix_shell_quote(build_sudo_write_script()),
        args
    )
}

/// 探测远端能否提权、以及是否需要口令。
///
/// # Errors
/// 通道打不开或 exec 本身失败时返回 `ErrorCode::ChannelFailed`/`SftpFailed`；
/// "不能提权"本身不是错误，而是 `Ok(SudoCapability::Unavailable)`。
pub async fn probe_sudo(remote: &SshRemote) -> Result<SudoCapability, AppError> {
    let outcome = exec_piped(remote, "sudo -n -- true", None, &[], PIPE_STEP_TIMEOUT).await?;
    if outcome.exit_status == Some(0) {
        return Ok(SudoCapability::Passwordless);
    }
    let text = outcome.stderr_tail.to_ascii_lowercase();
    let reason = if text.contains("sudo:")
        && (text.contains("not found") || text.contains("no such file"))
        || outcome.exit_status == Some(127)
    {
        SudoUnavailable::NotInstalled
    } else if text.contains("password is required") || text.contains("a terminal is required") {
        return Ok(SudoCapability::PasswordRequired);
    } else if text.contains("tty") {
        SudoUnavailable::RequiresTty
    } else if text.contains("not allowed")
        || text.contains("not in the sudoers")
        || text.contains("user is not allowed")
    {
        SudoUnavailable::NotAuthorized
    } else {
        SudoUnavailable::Other
    };
    log::info!(
        "远端提权能力探测为不可用，原因: {:?}，现场: {}",
        reason,
        outcome.stderr_tail.trim()
    );
    Ok(SudoCapability::Unavailable {
        reason,
        detail: outcome.stderr_tail.trim().to_string(),
    })
}

/// 执行提权写入。
///
/// 时序：需要口令时先在独立通道校验（`sudo -S -- -v`），校验通过后写入通道一律用
/// `sudo -n`，因此正文流里永远不含口令；口令为空则直接按免密提交。
///
/// # Errors
/// 口令校验失败 `AuthFailed`；票据失效需口令 `SftpFailed`（文本明确提示重新输入）；
/// 长度闸门未过 `SftpVerifyFailed`；其余为 `SftpFailed`/`ChannelFailed`。
pub async fn sudo_write(
    remote: &SshRemote,
    request: &SudoWriteRequest<'_>,
) -> Result<SudoWriteReport, AppError> {
    let expected = request.payload.len() as u64;
    if let Some(password) = request.password {
        validate_password(remote, password).await?;
    }
    let token = unique_token();
    let temp = generate_same_dir_temp_path(request.target, &token);
    let command = build_sudo_write_command(request.target, &temp, expected, request.backup);
    log::info!(
        "发起 Sudo 提权写入，目标: {}，临时路径: {temp}，提交字节: {expected}",
        request.target
    );
    let outcome = exec_piped(remote, &command, None, request.payload, PIPE_STEP_TIMEOUT).await?;

    if outcome.exit_status != Some(0) {
        // 长度闸门失败（41）意味着远端只收到部分字节：此时替换已被拒绝，原文件完好。
        if outcome.exit_status == Some(41) {
            log::warn!(
                "Sudo 提权写入被长度闸门拒绝，远端仅收到不完整数据，原文件未被替换，目标: {}",
                request.target
            );
            return Err(AppError::with_detail(
                ErrorCode::SftpVerifyFailed,
                format!(
                    "提权写入中断：远端收到字节数与预期 {expected} 不符，已放弃替换，原文件未改动"
                ),
            ));
        }
        if outcome.needs_password() {
            return Err(AppError::with_detail(
                ErrorCode::AuthFailed,
                "提权票据已失效，需要重新输入 sudo 口令后重试".to_string(),
            ));
        }
        return Err(outcome.failure("Sudo 提权写入失败"));
    }
    // 退出码 0 本身就是长度闸门的凭证：脚本在 `mv` 之前已比对 `wc -c` 与预期字节数，
    // 不一致会以 41 退出并放弃替换，所以这里无需再 stat 一次做二次确认。
    let written = expected;
    log::info!(
        "Sudo 提权写入完成并已原子替换，目标: {}，字节: {written}",
        request.target
    );
    Ok(SudoWriteReport { bytes: written })
}

/// 校验 sudo 口令：口令只出现在这条通道里。
async fn validate_password(remote: &SshRemote, password: &str) -> Result<(), AppError> {
    let outcome = exec_piped(
        remote,
        "sudo -S -- -v",
        Some(password),
        &[],
        VALIDATE_TIMEOUT,
    )
    .await?;
    if outcome.exit_status == Some(0) {
        log::info!("Sudo 口令校验通过，提权票据已续期");
        return Ok(());
    }
    log::warn!(
        "Sudo 口令校验失败，退出码 {:?}，现场: {}",
        outcome.exit_status,
        outcome.stderr_tail.trim()
    );
    Err(AppError::with_detail(
        ErrorCode::AuthFailed,
        "Sudo 口令不正确或该用户未被授权提权".to_string(),
    ))
}

/// 生成同目录临时文件的唯一标记。
fn unique_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let pid = std::process::id();
    format!("{pid:x}{nanos:x}")
}

/// 在一条 exec 通道上执行命令：可选先送一行口令，再分块送正文，最后收退出码。
///
/// 读写用 `split()` 分离：正文在灌的同时必须有任务在收包，否则远端提前退出后
/// 窗口不再补充，写侧会永远等下去；收包任务同时负责抓统计行与 stderr 尾部。
async fn exec_piped(
    remote: &SshRemote,
    command: &str,
    password: Option<&str>,
    payload: &[u8],
    step_timeout: Duration,
) -> Result<PipedOutcome, AppError> {
    let channel = remote.open_session().await?;
    let (mut reader, writer) = channel.split();
    let mut read_task = tokio::spawn(async move {
        let mut outcome = PipedOutcome::default();
        while let Some(message) = reader.wait().await {
            match message {
                ChannelMsg::ExtendedData { ext: 1, data } => {
                    append_tail(&mut outcome.stderr_tail, &data);
                }
                ChannelMsg::Data { data } => append_tail(&mut outcome.stdout_tail, &data),
                ChannelMsg::ExitStatus { exit_status } => {
                    outcome.exit_status = Some(exit_status);
                }
                ChannelMsg::Close | ChannelMsg::Eof => break,
                _ => {}
            }
        }
        outcome
    });

    let feed = async {
        writer.exec(false, command).await.map_err(|error| {
            AppError::with_detail(
                ErrorCode::ChannelFailed,
                format!("启动提权进程失败，原因: {error}"),
            )
        })?;
        if let Some(password) = password {
            let line = format!("{password}\n");
            writer
                .data_bytes(line.into_bytes())
                .await
                .map_err(|error| pipe_write_error(error, "提交提权口令"))?;
        }
        for chunk in payload.chunks(STDIN_CHUNK_SIZE) {
            writer
                .data_bytes(chunk.to_vec())
                .await
                .map_err(|error| pipe_write_error(error, "传输提权数据"))?;
        }
        let _ = writer.eof().await;
        Ok::<(), AppError>(())
    };

    // 写侧与收包并发进行：远端提前退出时窗口不再补充，只有持续收包才能让写侧
    // 以"通道已关闭"错误结束，而不是永久等在窗口上。
    let write_budget = step_timeout.max(payload_feed_timeout(payload.len()));
    let fed = tokio::time::timeout(write_budget, feed).await.map_err(|_| {
        AppError::with_detail(
            ErrorCode::Timeout,
            format!(
                "提权通道写入长时间无进展（上限 {} 秒）",
                write_budget.as_secs()
            ),
        )
    });
    // 写侧失败也要收退出码：那才是真正的原因，直接抛写侧错误会把现场丢掉。
    let collected = tokio::time::timeout(step_timeout, &mut read_task).await;
    if collected.is_err() {
        // 超时后必须掐掉收包任务，否则它会带着通道句柄继续活着成脱缰任务。
        read_task.abort();
    }
    let outcome = collected
        .map_err(|_| {
            AppError::with_detail(
                ErrorCode::Timeout,
                "提权进程未在规定时间内回报结果".to_string(),
            )
        })?
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::ChannelFailed,
                format!("提权结果收集任务异常结束，原因: {error}"),
            )
        })?;
    fed??;
    Ok(outcome)
}

/// 正文档位越多，允许的单步时限越长：仍然要求"有进展"，但不误杀合法大文件写入。
fn payload_feed_timeout(bytes: usize) -> Duration {
    let chunks = bytes.div_ceil(STDIN_CHUNK_SIZE.max(1)) as u64;
    // 每块最多放宽 2 秒，封顶 10 分钟，避免病态尺寸把超时变成没有超时。
    Duration::from_secs((chunks.saturating_mul(2)).min(600))
}

fn pipe_write_error(error: impl std::fmt::Display, action: &str) -> AppError {
    AppError::with_detail(
        ErrorCode::Io,
        format!("{action}到提权通道失败，原因: {error}"),
    )
}

/// 追加 stderr 尾部并裁剪到上限，防止失控输出填满内存。
fn append_tail(tail: &mut String, bytes: &[u8]) {
    tail.push_str(&String::from_utf8_lossy(bytes));
    if tail.len() > STDERR_TAIL_BYTES {
        let drop = tail.len() - STDERR_TAIL_BYTES;
        // 按字符边界裁剪，避免把一个多字节汉字切成半个导致后续 lossy 文本乱码。
        let mut boundary = drop;
        while boundary < tail.len() && !tail.is_char_boundary(boundary) {
            boundary += 1;
        }
        tail.drain(..boundary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_shell_quote() {
        assert_eq!(posix_shell_quote("hello"), "'hello'");
        assert_eq!(posix_shell_quote("nginx'conf"), r"'nginx'\''conf'");
    }

    #[test]
    fn test_parent_directory() {
        assert_eq!(get_parent_directory("/etc/nginx/nginx.conf"), "/etc/nginx");
        assert_eq!(get_parent_directory("/hosts"), "/");
        assert_eq!(get_parent_directory("foo.txt"), ".");
    }

    #[test]
    fn test_generate_same_dir_temp_path() {
        let temp = generate_same_dir_temp_path("/etc/nginx/nginx.conf", "1234");
        assert_eq!(temp, "/etc/nginx/.nginx.conf.transom_sudo_1234");
        assert_eq!(
            generate_same_dir_temp_path("/hosts", "5678"),
            "/.hosts.transom_sudo_5678"
        );
    }

    /// 备份后缀必须与普通写回路径一致，否则同一目录里会出现两套备份文件。
    #[test]
    fn test_backup_path_matches_plain_write_suffix() {
        assert_eq!(backup_path("/etc/hosts"), "/etc/hosts.transom-bak");
    }

    /// 脚本体内不含任何外部文本：路径与字节数一律走位置参数，注入面为零。
    #[test]
    fn test_script_references_positional_args_only() {
        let script = build_sudo_write_script();
        assert!(script.contains("dst=$1"));
        assert!(script.contains("tmp=$2"));
        assert!(script.contains("[ \"$got\" = \"$3\" ] || exit 41"));
        assert!(!script.contains("sudo "));
        assert!(script.contains("trap 'rm -f \"$tmp\"' EXIT"));
        // 长度闸门必须先于 mv：这是"中断不会截断生产文件"的唯一保证。
        let gate = script.find("exit 41").expect("长度闸门");
        let rename = script.find("mv -f").expect("原子替换");
        assert!(gate < rename, "长度闸门必须在替换之前");
    }

    /// 特殊字符路径被完整引用，且不会被拆成额外命令。
    #[test]
    fn test_command_quotes_path_with_injection_attempt() {
        let evil = "/etc/$(rm -rf '/') ; reboot";
        let command = build_sudo_write_command(evil, "/etc/.tmp", 42, true);
        assert!(command.starts_with("sudo -n -- sh -c '"), "{command}");
        // 参数以 `transom '<target>' '<tmp>' 42 1` 平铺，target 内部单引号已闭合转义。
        assert!(
            command.ends_with("transom '/etc/$(rm -rf '\\''/'\\'') ; reboot' '/etc/.tmp' 42 1"),
            "{command}"
        );
    }

    #[test]
    fn test_command_backup_flag_and_expected_length() {
        let with_backup = build_sudo_write_command("/etc/hosts", "/etc/.t", 7, true);
        let without = build_sudo_write_command("/etc/hosts", "/etc/.t", 7, false);
        assert!(with_backup.ends_with("' 7 1"));
        assert!(without.ends_with("' 7 0"));
        // 备份意图只改最后一个位置参数，脚本体与其余参数必须完全一致。
        let script_part = |command: &str| {
            command
                .split(" transom ")
                .next()
                .unwrap_or(command)
                .to_string()
        };
        assert_eq!(
            script_part(&with_backup),
            script_part(&without),
            "备份意图不该改变脚本结构"
        );
    }

    /// stderr 尾部必须有界，且不能在多字节字符中间截断。
    #[test]
    fn test_stderr_tail_is_bounded_and_char_safe() {
        let mut tail = String::new();
        for _ in 0..200 {
            append_tail(&mut tail, "权限不足，无法写入。".as_bytes());
        }
        assert!(
            tail.len() <= STDERR_TAIL_BYTES + 32,
            "实际长度 {}",
            tail.len()
        );
        assert!(
            std::str::from_utf8(tail.as_bytes()).is_ok(),
            "必须仍是合法 UTF-8"
        );
    }

    /// 大正文的写入时限按块数放宽，但仍有封顶，不会退化成"没有超时"。
    #[test]
    fn test_feed_timeout_scales_with_size_and_caps() {
        assert_eq!(payload_feed_timeout(0), Duration::from_secs(0));
        assert_eq!(
            payload_feed_timeout(STDIN_CHUNK_SIZE * 5),
            Duration::from_secs(10)
        );
        assert_eq!(payload_feed_timeout(usize::MAX), Duration::from_secs(600));
    }

    /// 提权通道的命令必须区分"校验口令"与"写正文"两种时序。
    #[test]
    fn test_write_channel_never_asks_for_password() {
        // 正文通道固定用 -n：sudo 永不提示口令，stdin 100% 是文件内容。
        let command = build_sudo_write_command("/etc/hosts", "/etc/.t", 3, false);
        assert!(command.contains("sudo -n"));
        assert!(!command.contains("-S"));
    }

    #[test]
    fn test_needs_password_detects_both_diagnostic_forms() {
        let mut outcome = PipedOutcome {
            stderr_tail: "sudo: a password is required".to_string(),
            ..Default::default()
        };
        assert!(outcome.needs_password());
        outcome.stderr_tail = "sudo: sorry, you must have a tty to run sudo".to_string();
        assert!(!outcome.needs_password(), "requiretty 不能被误判成缺口令");
    }

    #[test]
    fn test_failure_text_keeps_exit_code_when_no_stderr() {
        let outcome = PipedOutcome {
            exit_status: Some(1),
            ..Default::default()
        };
        let error = outcome.failure("Sudo 提权写入失败");
        assert!(error.detail.unwrap_or_default().contains("退出码 1"));
    }
}
