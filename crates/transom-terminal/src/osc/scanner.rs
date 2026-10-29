//! PTY 字节流旁路 OSC 增量扫描器。

use super::types::{MAX_PAYLOAD_BYTES, OSC_INTRO, OscEvent};
use crate::sequence_scanner::SequenceScanner;
use crate::shell_integration::HANDSHAKE_TOKEN;

/// OSC 扫描器：序列边界复用通用骨架，载荷语义在本模块解析。
pub(crate) struct OscScanner {
    inner: SequenceScanner,
}

impl Default for OscScanner {
    fn default() -> Self {
        Self {
            inner: SequenceScanner::new(OSC_INTRO, MAX_PAYLOAD_BYTES),
        }
    }
}

impl OscScanner {
    /// 喂入一段字节流，返回其中识别到的事件。不消费输入，原样透传给内核解析器。
    #[cfg(test)]
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        self.feed_with_offsets(bytes)
            .into_iter()
            .map(|(_, event)| event)
            .collect()
    }

    /// 与 [`Self::feed`] 相同，但附带序列末字节在本段中的下标。
    pub(crate) fn feed_with_offsets(&mut self, bytes: &[u8]) -> Vec<(usize, OscEvent)> {
        self.inner.feed_with_offsets(bytes, emit)
    }
}

/// 解析一条完整 OSC 载荷。
fn emit(bytes: &[u8], offset: usize, events: &mut Vec<(usize, OscEvent)>) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Some((code, params)) = text.split_once(';') else {
        return;
    };
    let parse = match code {
        "133" => parse_command_marker(params),
        // 7 是 xterm 系当前工作目录的事实标准：`file://<host>/<路径>`。
        "7" => parse_cwd(params),
        // 9 同时被通知、进度与 ConEmu cwd 复用，必须按子码分流。
        "9" => parse_osc9(params),
        // 777 是"自定义通知"约定段：先试本客户端的集成握手，再退回桌面通知。
        "777" => parse_integration_handshake(params).or_else(|| parse_notify(params)),
        _ => None,
    };
    if let Some(event) = parse {
        events.push((offset, event));
    }
}

/// OSC 777 的 `transom;<键>=<值>` 握手：注入脚本与探针的回报通道。
///
/// 与桌面通知共用 777 段，靠首段关键字区分，两条路径互不干扰。回报走 OSC 而非
/// 普通 stdout，是为了让探测在屏幕上完全不落地——只有 Shell 回显的那行命令本身可见。
///
/// 局限：`$SHELL` 路径里含分号时会在分号处截断（Unix 合法但极罕见），截断后仍
/// 能在候选方言里匹配，不至于误判成别的方言。
fn parse_integration_handshake(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    if parts.next()? != HANDSHAKE_TOKEN {
        return None;
    }
    let value = parts.next()?;
    match value.strip_prefix("shell=") {
        Some(shell) => Some(OscEvent::ShellReport {
            shell: shell.to_string(),
        }),
        None => value
            .strip_prefix("ready=")
            .map(|shell| OscEvent::IntegrationReady {
                shell: shell.to_string(),
            }),
    }
}

/// OSC 133 的参数：iTerm2/WezTerm 系 shell 集成用它标记提示符与命令边界。
fn parse_command_marker(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    let mark = parts.next()?;
    match mark {
        "A" => Some(OscEvent::PromptStart),
        "B" => Some(OscEvent::CommandStart),
        "D" => Some(OscEvent::CommandFinish {
            exit_code: parts.next().and_then(|value| value.parse::<i32>().ok()),
            duration_ms: parts.next().and_then(|value| value.parse::<u64>().ok()),
        }),
        "C" | "E" => Some(OscEvent::PreExec {
            command: parts
                .next()
                .map(str::to_string)
                .filter(|text| !text.is_empty()),
        }),
        _ => None,
    }
}

/// OSC 7：`file://<host>/<路径>`，host 可为空，路径允许百分号编码。
///
/// 设计原理（Why）：
/// - 只接受本地文件 URI，host 段仅作来源标记，不参与路径判断：SSH 会话里
///   主机名可能与本地不同，拿它做校验只会误伤；
/// - 非法百分号编码不整条丢弃，而是按字面保留：很多提示符脚本直接拼 `$PWD`，
///   目录名里的 `%` 未必编码；拒绝会让整条 cwd 上报失效；
/// - 解码后含控制字符的路径一律拒绝：换行与 ESC 会污染路径输入框与界面文本，
///   这是不可信输入，不能只依赖上游 Shell。
///
/// 代价与局限：不做 URI 规范级校验（端口、用户信息、相对路径等），只提取
/// 第一个 `/` 之后的路径段；`file://` 前缀大小写不敏感。
fn parse_cwd(params: &str) -> Option<OscEvent> {
    let scheme = params.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = params.get(7..)?;
    let path_start = rest.find('/')?;
    let raw_path = &rest[path_start..];
    let path = percent_decode(raw_path);
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    Some(OscEvent::Cwd { path })
}

/// 解码 URI 路径里的 `%XX`；非法转义或解码后不是合法 UTF-8 时按原样保留。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| input.to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// OSC 9：iTerm2 约定的桌面通知 `9;<正文>`。
///
/// 该码被多个协议复用：`9;4;...` 是进度（ConEmu/Windows Terminal），
/// `9;9;<path>` 是 ConEmu 的 cwd 上报。两者都不属于通知：进度暂未实现，
/// cwd 由 OSC 7 统一承载，这里显式忽略，避免被当成正文弹出。
fn parse_osc9(params: &str) -> Option<OscEvent> {
    if params.starts_with("4;") || params.starts_with("9;") {
        return None;
    }
    if params.trim().is_empty() {
        return None;
    }
    Some(OscEvent::Notify {
        title: String::new(),
        body: params.to_string(),
    })
}

/// OSC 777：`notify;标题;正文` 或 `notify;title=..;body=..;icon=..`。
fn parse_notify(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    if parts.next()? != "notify" {
        return None;
    }
    let mut title = String::new();
    let mut body = String::new();
    let mut positional: Vec<&str> = Vec::with_capacity(2);
    for part in parts.take(8) {
        match part.split_once('=') {
            Some(("title", value)) => title = value.to_string(),
            Some(("body", value)) | Some(("message", value)) => body = value.to_string(),
            Some((_, _)) => {}
            None if positional.len() < 2 => positional.push(part),
            None => {}
        }
    }
    if title.is_empty() {
        title = positional.first().copied().unwrap_or_default().to_string();
    }
    if body.is_empty() {
        body = positional.get(1).copied().unwrap_or_default().to_string();
    }
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(OscEvent::Notify { title, body })
}
