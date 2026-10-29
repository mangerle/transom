//! OSC 8 超链接目标识别：区分网页链接与编辑器代码位置。
//!
//! 设计原理（Why）：
//! - 终端只负责把链接"值"翻译成不含策略的目标类型，是否打开浏览器、是否在
//!   内置编辑器里跳行属于应用层决策；终端 crate 不依赖 UI，也就不该知道 SFTP。
//! - 网页协议继续走白名单：只有 http/https/mailto/ftp 能交给系统默认程序，
//!   防止远端输出借 `xdg-open` 触发任意协议处理器。
//! - `file:line:col` 是编译器、grep、ripgrep 的通用定位格式；同时接受
//!   `file:///path:line:col` 与编辑器常见的 `file:///path#Lline[:col]` 两种写法。
//!
//! 安全约束：
//! - 路径中出现 `..` 组件整条拒绝（与项目输入洗化口径一致），避免链接把
//!   编辑器引向任意目录；
//! - 非本机的 `file://host/...` 无法可靠解析，直接判为不支持而不是猜成本地路径。

/// 一条 OSC 8 链接所属的目标类别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HyperlinkTarget {
    /// 可交给系统默认程序打开的网页类链接。
    Url(String),
    /// 可交给内置编辑器定位的代码位置；行列均为 1 基。
    CodeLocation {
        path: String,
        line: u32,
        column: Option<u32>,
    },
    /// 协议未知或路径不安全。
    Unsupported,
}

const URL_PREFIXES: [&str; 4] = ["http://", "https://", "mailto:", "ftp://"];

/// 识别链接目标；纯函数，便于覆盖所有边界分支。
pub(crate) fn classify(uri: &str) -> HyperlinkTarget {
    let uri = uri.trim();
    if uri.is_empty() {
        return HyperlinkTarget::Unsupported;
    }
    let lowered = uri.to_ascii_lowercase();
    if URL_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
    {
        return HyperlinkTarget::Url(uri.to_string());
    }
    if let Some(rest) = lowered.strip_prefix("file:").map(|_| &uri["file:".len()..]) {
        return classify_file_uri(rest);
    }
    // 不带 scheme 的 `path:line[:col]`：编译器与检索工具最常见的形式。
    match split_code_location(uri) {
        Some((path, line, column)) if !path.is_empty() => {
            HyperlinkTarget::CodeLocation { path, line, column }
        }
        _ => HyperlinkTarget::Unsupported,
    }
}

/// 处理 `file:` 之后的部分（可能带 `//` 与主机段）。
fn classify_file_uri(rest: &str) -> HyperlinkTarget {
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    // `file:///path` 剥掉 `//` 后以 `/` 开头；`file://localhost/path` 允许本机名。
    let path = if rest.starts_with('/') {
        rest
    } else if let Some(local) = rest.strip_prefix("localhost/") {
        return finish_code_path(&format!("/{local}"));
    } else if rest.contains('/') {
        // `file://host/path`：主机不是本机，无法映射到本地文件系统。
        return HyperlinkTarget::Unsupported;
    } else {
        rest
    };
    finish_code_path(path)
}

/// 对路径做百分号解码、代码位置拆分与安全校验。
fn finish_code_path(raw: &str) -> HyperlinkTarget {
    let decoded = percent_decode(raw);
    let Some((path, line, column)) = split_code_location(&decoded) else {
        return HyperlinkTarget::Unsupported;
    };
    if path.is_empty() || !path_is_safe(&path) {
        return HyperlinkTarget::Unsupported;
    }
    HyperlinkTarget::CodeLocation { path, line, column }
}

/// 从路径尾部拆出 `:line[:col]` 或 `#Lline[:col]`。
///
/// 只从尾部消费数字段：路径中间可以合法含 `:`（如容器内路径），不参与判断。
fn split_code_location(value: &str) -> Option<(String, u32, Option<u32>)> {
    if let Some((path, suffix)) = value.rsplit_once("#L") {
        let (line, column) = parse_line_column(suffix)?;
        return Some((path.to_string(), line, column));
    }
    let (before_last, last) = value.rsplit_once(':')?;
    let last_value = last.parse::<u32>().ok().filter(|value| *value >= 1)?;
    if let Some((path, previous)) = before_last.rsplit_once(':')
        && let Ok(line) = previous.parse::<u32>()
        && line >= 1
    {
        return Some((path.to_string(), line, Some(last_value)));
    }
    Some((before_last.to_string(), last_value, None))
}

/// `#L` 后缀：`12` 或 `12:3`。
fn parse_line_column(suffix: &str) -> Option<(u32, Option<u32>)> {
    let mut parts = suffix.split(':');
    let line = parts
        .next()?
        .parse::<u32>()
        .ok()
        .filter(|line| *line >= 1)?;
    let column = match parts.next() {
        Some(raw) => Some(raw.parse::<u32>().ok().filter(|column| *column >= 1)?),
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((line, column))
}

/// 只做 `%XX` 解码；非法转义原样保留，不因一个坏字节丢掉整条链接。
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 拒绝含 `..` 组件的路径；绝对/相对都允许，由应用层决定解析基准。
fn path_is_safe(path: &str) -> bool {
    !path.split(['/', '\\']).any(|part| part == "..")
}

#[cfg(test)]
mod tests {
    use super::{HyperlinkTarget, classify};

    fn location(uri: &str) -> (String, u32, Option<u32>) {
        match classify(uri) {
            HyperlinkTarget::CodeLocation { path, line, column } => (path, line, column),
            other => panic!("应识别为代码位置，实际: {other:?}"),
        }
    }

    #[test]
    fn web_schemes_go_to_system_opener() {
        for uri in [
            "https://example.com/a?b=1",
            "HTTP://EXAMPLE.COM",
            "mailto:dev@example.com",
            "ftp://host/file",
        ] {
            assert!(
                matches!(classify(uri), HyperlinkTarget::Url(_)),
                "{uri} 应交给系统默认程序"
            );
        }
    }

    #[test]
    fn file_uri_with_line_and_column() {
        assert_eq!(
            location("file:///home/dev/src/main.rs:42:7"),
            ("/home/dev/src/main.rs".to_string(), 42, Some(7))
        );
        assert_eq!(
            location("file:///home/dev/src/main.rs#L42"),
            ("/home/dev/src/main.rs".to_string(), 42, None)
        );
        assert_eq!(
            location("file://localhost/home/dev/a.py#L8:3"),
            ("/home/dev/a.py".to_string(), 8, Some(3))
        );
    }

    #[test]
    fn bare_path_with_line_suffix() {
        assert_eq!(
            location("src/editor/view.rs:120"),
            ("src/editor/view.rs".to_string(), 120, None)
        );
        assert_eq!(
            location("/var/log/app.log:9:2"),
            ("/var/log/app.log".to_string(), 9, Some(2))
        );
    }

    #[test]
    fn percent_encoded_path_is_decoded() {
        assert_eq!(
            location("file:///tmp/my%20dir/a.rs:3"),
            ("/tmp/my dir/a.rs".to_string(), 3, None)
        );
    }

    #[test]
    fn rejects_traversal_and_foreign_hosts() {
        assert_eq!(
            classify("file:///etc/../../secret:1"),
            HyperlinkTarget::Unsupported,
            "含 .. 的路径必须整条拒绝"
        );
        assert_eq!(
            classify("file://otherhost/etc/passwd:1"),
            HyperlinkTarget::Unsupported,
            "非本机主机无法可靠映射"
        );
        assert_eq!(
            classify("javascript:alert(1)"),
            HyperlinkTarget::Unsupported
        );
    }
}
