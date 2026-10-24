//! SFTP 悬停轻量预览的公共类型与内容处理。
//!
//! 设计原理（Why）：
//! - 实现初衷：文件管理器里最频繁的判断是「这个文件是什么内容」，
//!   双击打开编辑器成本过高；悬停即预览可以把这一步压缩到一次停留；
//! - 核心优势：文本只读文件头、图片限制体积，预览请求不进入传输队列，
//!   不占用队列进度与通知，也不会因一次误悬停产生大流量；
//! - 代价与局限：预览结果不做内存缓存，同一文件重复悬停会重新读取；
//!   这是为了保持状态机简单，把磁盘/网络开销限制在用户主动悬停的时机。
//!   远端图片仅把字节写入临时文件供渲染器异步解码，不保留预览内容本身。

use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::time::Duration;
use transom_core::{AppError, ConnectionId, TabId, TransferId};

/// 悬停多久后才真正读取预览：避免鼠标扫过列表时制造无意义的网络/磁盘读取。
pub(crate) const HOVER_PREVIEW_DELAY: Duration = Duration::from_millis(320);
/// 文本预览最多展示的行数，控制卡片高度。
pub(crate) const PREVIEW_TEXT_LINES: usize = 20;
/// 文本预览最多读取的字节数；覆盖常见配置与代码文件的开头部分。
pub(crate) const PREVIEW_TEXT_MAX_BYTES: u64 = 64 * 1024;
/// 图片缩略图允许下载的最大字节数；超过则直接跳过预览。
pub(crate) const PREVIEW_IMAGE_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// 单行文本的字符上限，防止压缩后的单行日志把卡片撑出屏幕。
const PREVIEW_LINE_MAX_CHARS: usize = 120;
/// 预览卡片宽度；文本与图片共用，保持浮层视觉一致。
pub(crate) const PREVIEW_CARD_WIDTH: f32 = 444.0;
/// 图片缩略图区域高度。
pub(crate) const PREVIEW_IMAGE_HEIGHT: f32 = 260.0;

/// 可以预览的内容类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewKind {
    Image,
    Text,
}

/// 预览目标的唯一标识：远端路径与本地路径可能同名，必须带上来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewKey {
    pub(crate) is_remote: bool,
    pub(crate) path: String,
}

/// 文件行悬停时传给预览状态机的目标快照。
#[derive(Clone, Debug)]
pub(crate) struct SftpHoverTarget {
    pub(crate) tab_id: TabId,
    pub(crate) key: PreviewKey,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
}

/// 预览卡片当前内容。
pub(crate) enum PreviewContent {
    /// 悬停延迟尚未结束：不渲染卡片，避免闪烁。
    Pending,
    /// 已发起读取，等待远端或写缓存完成。
    Loading,
    Text {
        lines: Vec<String>,
        truncated: bool,
    },
    Image {
        path: PathBuf,
    },
    Error(String),
}

/// 远端预览结果快照。
///
/// 设计原理（Why）：预览结果需要同时携带标签、连接、请求标识与字节内容，
/// 平铺为函数参数会超过参数数量上限，也不利于调用点按语义阅读；
/// 统一为参数对象后，事件分发处只需构造一次。
pub(crate) struct SftpPreviewResponse {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) op_id: TransferId,
    pub(crate) result: Result<(Vec<u8>, bool), AppError>,
}

/// 单个面板当前的悬停预览状态。
///
/// 设计原理（Why）：状态挂在 SFTP 面板而非全局，关闭面板或标签时随面板
/// 一起释放；任务句柄一并持有，目标切换时 `drop` 即取消旧延迟/写盘任务，
/// 不需要额外维护取消令牌。
pub(crate) struct SftpPreviewState {
    pub(crate) key: PreviewKey,
    pub(crate) name: String,
    pub(crate) kind: PreviewKind,
    pub(crate) anchor: Point<Pixels>,
    pub(crate) content: PreviewContent,
    /// 远端图片预览的请求标识；文本与本地预览为 `None`。
    pub(crate) op_id: Option<TransferId>,
    /// 远端图片写入的本地缓存文件；清理时删除，绝不指向用户本地文件。
    pub(crate) cache_file: Option<PathBuf>,
    /// 仅用于持有延迟任务：替换/清理状态时 drop 即取消。
    pub(super) _delay_task: Option<Task<()>>,
    pub(super) write_task: Option<Task<()>>,
}

impl SftpPreviewState {
    /// 删除预览产生的本地缓存文件（仅远端图片存在缓存）。
    pub(crate) fn discard_cache_file_if_any(&self) {
        if let Some(path) = &self.cache_file {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 按文件名判断是否支持预览；扩展名优先，无扩展名时按常见文件名匹配。
///
/// 设计原理（Why）：不按文件内容嗅探，避免为了判断类型而读取每一个文件；
/// 白名单只覆盖常见图片与文本类型，未知扩展名一律不预览，宁可少预览
/// 也不把二进制内容当文本展示。
pub(crate) fn preview_kind(name: &str) -> Option<PreviewKind> {
    let lower = name.to_ascii_lowercase();
    if let Some((_, extension)) = lower.rsplit_once('.')
        && !extension.is_empty()
    {
        if IMAGE_EXTENSIONS.contains(&extension) {
            return Some(PreviewKind::Image);
        }
        if TEXT_EXTENSIONS.contains(&extension) {
            return Some(PreviewKind::Text);
        }
        return None;
    }
    TEXT_FILE_NAMES
        .contains(&lower.as_str())
        .then_some(PreviewKind::Text)
}

const IMAGE_EXTENSIONS: &[&str] = &[
    "avif", "bmp", "gif", "ico", "jpeg", "jpg", "png", "svg", "tif", "tiff", "webp",
];

const TEXT_EXTENSIONS: &[&str] = &[
    "bash",
    "bashrc",
    "bib",
    "cfg",
    "c",
    "cc",
    "cjs",
    "cmake",
    "cmd",
    "conf",
    "cpp",
    "cs",
    "css",
    "csv",
    "cts",
    "cxx",
    "desktop",
    "dockerignore",
    "editorconfig",
    "env",
    "fish",
    "gitattributes",
    "gitignore",
    "go",
    "gradle",
    "h",
    "hh",
    "hpp",
    "htm",
    "html",
    "hxx",
    "ini",
    "java",
    "js",
    "json",
    "jsonl",
    "jsx",
    "kt",
    "kts",
    "less",
    "log",
    "lua",
    "m",
    "markdown",
    "md",
    "mjs",
    "mm",
    "mts",
    "php",
    "pl",
    "pm",
    "properties",
    "ps1",
    "py",
    "pyi",
    "r",
    "rb",
    "rc",
    "rs",
    "sass",
    "scss",
    "service",
    "sh",
    "sql",
    "svelte",
    "swift",
    "tex",
    "text",
    "toml",
    "ts",
    "tsv",
    "tsx",
    "txt",
    "vimrc",
    "vue",
    "xml",
    "yaml",
    "yml",
    "zsh",
    "zshrc",
];

const TEXT_FILE_NAMES: &[&str] = &[
    "authors",
    "authorized_keys",
    "changelog",
    "changes",
    "config",
    "contributors",
    "crontab",
    "dockerfile",
    "fstab",
    "gemfile",
    "gnumakefile",
    "hosts",
    "known_hosts",
    "licence",
    "license",
    "makefile",
    "notice",
    "passwd",
    "procfile",
    "rakefile",
    "readme",
    "ssh_config",
    "sshd_config",
    "vagrantfile",
];

/// 从文件头部字节生成预览行；返回实际是否还有未展示内容。
pub(crate) fn text_preview_lines(bytes: &[u8], truncated: bool) -> (Vec<String>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = Vec::with_capacity(PREVIEW_TEXT_LINES);
    let mut more = truncated;
    for line in text.lines() {
        if lines.len() >= PREVIEW_TEXT_LINES {
            more = true;
            break;
        }
        lines.push(truncate_preview_line(line));
    }
    (lines, more)
}

fn truncate_preview_line(line: &str) -> String {
    let line = line.trim_end_matches('\r');
    let mut characters = line.chars();
    let mut out = String::with_capacity(line.len().min(PREVIEW_LINE_MAX_CHARS * 4));
    for _ in 0..PREVIEW_LINE_MAX_CHARS {
        match characters.next() {
            Some('\t') => out.push_str("    "),
            Some(character) if character.is_control() => out.push(' '),
            Some(character) => out.push(character),
            None => return out,
        }
    }
    if characters.next().is_some() {
        out.push('…');
    }
    out
}

/// 读取本地文本文件的头部；多读 1 字节判断是否发生截断。
pub(super) fn read_local_text_head(
    path: &Path,
    max_bytes: u64,
) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;

    let file = std::fs::File::open(path)?;
    let capacity = usize::try_from(max_bytes).unwrap_or(0);
    let mut buffer = Vec::with_capacity(capacity);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut buffer)?;
    let truncated = u64::try_from(buffer.len()).unwrap_or(u64::MAX) > max_bytes;
    if truncated {
        buffer.truncate(capacity);
    }
    Ok((buffer, truncated))
}

pub(super) fn preview_cache_path(tab_id: TabId, op_id: TransferId, extension: String) -> PathBuf {
    std::env::temp_dir()
        .join("transom-sftp-preview")
        .join(tab_id.to_string())
        .join(format!("{op_id}.{extension}"))
}

pub(super) fn image_extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "png".to_string())
}

#[cfg(test)]
mod tests {
    use super::{PreviewKind, image_extension, preview_kind, text_preview_lines};

    #[test]
    fn test_preview_kind_image_by_extension() {
        assert_eq!(preview_kind("photo.PNG"), Some(PreviewKind::Image));
        assert_eq!(preview_kind("photo.jpeg"), Some(PreviewKind::Image));
        assert_eq!(preview_kind("icon.svg"), Some(PreviewKind::Image));
    }

    #[test]
    fn test_preview_kind_text_by_extension_and_name() {
        assert_eq!(preview_kind("日志.log"), Some(PreviewKind::Text));
        assert_eq!(preview_kind("config.toml"), Some(PreviewKind::Text));
        assert_eq!(preview_kind(".env"), Some(PreviewKind::Text));
        assert_eq!(preview_kind(".gitignore"), Some(PreviewKind::Text));
        assert_eq!(preview_kind("README"), Some(PreviewKind::Text));
        assert_eq!(preview_kind("Makefile"), Some(PreviewKind::Text));
    }

    #[test]
    fn test_preview_kind_unknown_is_skipped() {
        assert_eq!(preview_kind("archive.tar.gz"), None);
        assert_eq!(preview_kind("firmware.bin"), None);
        assert_eq!(preview_kind("binary"), None);
        assert_eq!(preview_kind(""), None);
    }

    #[test]
    fn test_text_preview_lines_limits_and_cleanup() {
        let content = "第一行\r\n\t缩进行\n\n最后一行";
        let (lines, truncated) = text_preview_lines(content.as_bytes(), false);
        assert_eq!(lines, vec!["第一行", "    缩进行", "", "最后一行"]);
        assert!(!truncated);
    }

    #[test]
    fn test_text_preview_lines_stops_at_limit() {
        let content = (0..30)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (lines, truncated) = text_preview_lines(content.as_bytes(), false);
        assert_eq!(lines.len(), super::PREVIEW_TEXT_LINES);
        assert_eq!(lines[0], "line 0");
        assert!(truncated);
    }

    #[test]
    fn test_text_preview_lines_marks_byte_truncation() {
        let (lines, truncated) = text_preview_lines(b"only one line", true);
        assert_eq!(lines, vec!["only one line"]);
        assert!(truncated);
    }

    #[test]
    fn test_text_preview_lines_handles_empty_and_long_lines() {
        let (lines, truncated) = text_preview_lines(b"", false);
        assert!(lines.is_empty());
        assert!(!truncated);

        let long = "a".repeat(super::PREVIEW_LINE_MAX_CHARS + 10);
        let (lines, _) = text_preview_lines(long.as_bytes(), false);
        assert_eq!(lines[0].chars().count(), super::PREVIEW_LINE_MAX_CHARS + 1);
        assert!(lines[0].ends_with('…'));
    }

    #[test]
    fn test_image_extension_normalizes() {
        assert_eq!(image_extension("photo.PNG"), "png");
        assert_eq!(image_extension("photo.jpeg"), "jpeg");
        assert_eq!(image_extension("no-extension"), "png");
    }
}
