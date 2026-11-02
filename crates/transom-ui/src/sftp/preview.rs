//! SFTP 悬停轻量预览的公共类型与内容处理。
//!
//! 设计原理（Why）：
//! - 实现初衷：文件管理器里最频繁的判断是「这个文件是什么内容」，
//!   双击打开编辑器成本过高；悬停即预览可以把这一步压缩到一次停留；
//! - 核心优势：文本只读文件头、图片限制体积，预览请求不进入传输队列，
//!   不占用队列进度与通知，也不会因一次误悬停产生大流量；
//! - 代价与局限：预览结果不做内存缓存，同一文件重复悬停会重新读取；
//!   这是为了保持状态机简单，把磁盘/网络开销限制在用户主动悬停的时机。

use gpui_kit::*;
use std::path::Path;
use std::sync::Arc;
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
/// 单张图片允许解码的最大像素数。
///
/// 文件体积小不等于解码内存小：一张 8 MB 的高压缩 JPEG 可能达到数千万像素，
/// 解码后占用数百 MB；在解码前读取图片头校验像素数，才能把峰值内存钉在
/// 可预期范围内。32 MP 已覆盖常见相机照片。
pub(crate) const PREVIEW_IMAGE_MAX_PIXELS: u64 = 32_000_000;
/// 缩略图目标尺寸：卡片图片区域（420x260）的 2 倍，高分屏下仍然清晰。
pub(crate) const PREVIEW_THUMBNAIL_MAX_WIDTH: u32 = 840;
pub(crate) const PREVIEW_THUMBNAIL_MAX_HEIGHT: u32 = 520;
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

/// 预览目标的唯一标识：远端路径与本地路径可能同名，必须带上来源；
/// 同一路径也可能出现在主远端与并列远端，远端键必须再带连接。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewKey {
    pub(crate) is_remote: bool,
    pub(crate) connection_id: Option<ConnectionId>,
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
    /// 已在后台解码并缩放好的缩略图，渲染阶段不再接触原图。
    Image {
        thumbnail: Arc<RenderImage>,
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
    /// 当前预览请求的标识：远端事件与本地缩略图任务都用它校验结果归属。
    pub(crate) op_id: Option<TransferId>,
    /// 仅用于持有延迟任务：替换/清理状态时 drop 即取消。
    pub(super) _delay_task: Option<Task<()>>,
    /// 图片缩略图解码任务；切换目标时 drop 取消。
    pub(super) decode_task: Option<Task<()>>,
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

/// 可本地解码为缩略图的位图格式；SVG 是文本 XML，归入文本预览。
///
/// AVIF 等需要额外编解码器的格式不在其中：宁可当作未知文件跳过，
/// 也不要在悬停时弹出无法解析的图片卡片。
const IMAGE_EXTENSIONS: &[&str] = &[
    "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
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
    "svg",
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

mod text;
mod thumbnail;

pub(crate) use text::{read_local_text_head, text_preview_lines};
pub(crate) use thumbnail::decode_preview_thumbnail;

#[cfg(test)]
mod tests {
    use super::thumbnail::decode_thumbnail;
    use super::{PreviewKind, preview_kind, text_preview_lines};

    fn encode_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("测试图片编码必须成功");
        bytes
    }

    #[test]
    fn test_preview_kind_image_by_extension() {
        assert_eq!(preview_kind("photo.PNG"), Some(PreviewKind::Image));
        assert_eq!(preview_kind("photo.jpeg"), Some(PreviewKind::Image));
        assert_eq!(preview_kind("icon.svg"), Some(PreviewKind::Text));
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
        assert_eq!(preview_kind("photo.avif"), None);
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
    fn test_decode_thumbnail_downscales_and_converts_to_bgra() {
        let bytes = encode_png(100, 40, [255, 0, 0, 255]);
        let thumbnail = decode_thumbnail(&bytes, 10_000_000, 20, 20).expect("解码缩略图必须成功");
        assert_eq!(thumbnail.size(0).width.0, 20);
        assert_eq!(thumbnail.size(0).height.0, 8);
        let pixels = thumbnail.as_bytes(0).expect("缩略图必须包含像素");
        assert_eq!(&pixels[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn test_decode_thumbnail_keeps_small_image() {
        let bytes = encode_png(2, 1, [0, 255, 0, 255]);
        let thumbnail = decode_thumbnail(&bytes, 100, 10, 10).expect("小图必须保留原尺寸");
        assert_eq!(thumbnail.size(0).width.0, 2);
        assert_eq!(thumbnail.size(0).height.0, 1);
    }

    #[test]
    fn test_decode_thumbnail_rejects_too_many_pixels() {
        let bytes = encode_png(3, 3, [0, 0, 0, 255]);
        let error = decode_thumbnail(&bytes, 8, 10, 10).expect_err("9 像素超过上限 8");
        let expected = rust_i18n::t!(
            "sftp.preview.too_many_pixels",
            width = 3,
            height = 3,
            pixels = 9,
            max = 8
        )
        .to_string();
        assert_eq!(error, expected);
    }

    #[test]
    fn test_decode_thumbnail_rejects_invalid_bytes() {
        assert!(decode_thumbnail(b"not an image", 100, 10, 10).is_err());
    }

    /// 本地文本预览读取文件头部并生成行；正常小文件不应标记截断。
    #[test]
    fn local_text_preview_reads_small_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("preview.txt");
        std::fs::write(&path, "第一行\n第二行\n").expect("写入样本");
        let content =
            super::super::preview_actions::local_text_preview(path.to_str().expect("路径"));
        match content {
            crate::sftp::preview::PreviewContent::Text { lines, truncated } => {
                assert!(!truncated);
                assert!(lines.join("\n").contains("第一行"));
            }
            _ => panic!("小文本应生成文本预览"),
        }
    }

    /// 超过上限的文件必须标记截断，避免预览静默吞掉后半段。
    #[test]
    fn local_text_preview_marks_truncated_for_large_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("large.txt");
        let data = "a\n".repeat(super::PREVIEW_TEXT_MAX_BYTES as usize / 2 + 16);
        std::fs::write(&path, data).expect("写入大样本");
        let content =
            super::super::preview_actions::local_text_preview(path.to_str().expect("路径"));
        match content {
            crate::sftp::preview::PreviewContent::Text { truncated, .. } => assert!(truncated),
            _ => panic!("大文本应生成带截断标记的预览"),
        }
    }

    /// CRLF 去掉回车；超长行截断并追加省略号，控制字符转空格。
    #[test]
    fn text_preview_lines_normalizes_and_truncates() {
        let long = "x".repeat(super::PREVIEW_LINE_MAX_CHARS + 8);
        let bytes = format!("a\r\n\u{7}b\r\n{long}").into_bytes();
        let (lines, more) = super::text::text_preview_lines(&bytes, false);
        assert_eq!(lines[0], "a");
        assert_eq!(lines[1], " b");
        assert!(lines[2].ends_with('…'));
        assert!(!more);
    }

    /// 读取失败必须转成错误气泡，而不是静默返回空文本。
    #[test]
    fn local_text_preview_reports_missing_file() {
        let content = super::super::preview_actions::local_text_preview(
            "/definitely/not/a/real/transom-preview-file",
        );
        match content {
            crate::sftp::preview::PreviewContent::Error(_) => {}
            _ => panic!("不存在的文件应生成错误预览"),
        }
    }

    /// 扩展名判定不区分大小写；未知扩展名不产生预览入口。
    #[test]
    fn preview_kind_matches_extensions_case_insensitively() {
        assert_eq!(
            super::preview_kind("shot.PNG"),
            Some(super::PreviewKind::Image)
        );
        assert_eq!(
            super::preview_kind("notes.md"),
            Some(super::PreviewKind::Text)
        );
        assert_eq!(super::preview_kind("archive.bin"), None);
        assert_eq!(super::preview_kind("no-extension"), None);
    }
}
