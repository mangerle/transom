//! 外部终端会话导入解析器（Xshell `.xsh`、MobaXterm `.mxtsessions`）。
//!
//! 设计原理（Why）：
//! - 与 `ssh_config` 相同，将第三方会话文件的线格式解析收敛在 `transom-core`，
//!   与 UI 解耦以便独立进行单元测试。
//! - 统一产出 [`ImportedSessionEntry`]，同时携带构建好的 [`Session`] 与相对
//!   文件夹路径，由 UI/Store 层决定根分组名称与重名合并策略。

mod mobaxterm;
mod xshell;

pub use mobaxterm::parse_mobaxterm_sessions;
pub use xshell::{parse_xshell_file, parse_xshell_session, scan_xshell_directory};

use crate::model::session::Session;

/// 单文件读取上限（8 MiB），防止误选超大二进制文件导致内存膨胀。
pub(crate) const MAX_IMPORT_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// 外部导入会话条目：包含解析出的会话与相对文件夹路径。
#[derive(Debug, Clone)]
pub struct ImportedSessionEntry {
    /// 已解析并构建的会话模型。
    pub session: Session,
    /// 相对导入根分组的子文件夹路径（以 `/` 分隔）；`None` 表示直接放在根分组下。
    pub folder_path: Option<String>,
}

/// 将原始字节解码为文本，自动识别 UTF-8、UTF-16LE（Xshell 常见编码）与 UTF-16BE BOM。
///
/// 设计原理（Why）：Windows 版 Xshell 导出的 `.xsh` 默认常为 UTF-16LE 带 BOM，
/// 直接用 `std::fs::read_to_string` 会因非 UTF-8 报错失败；按 BOM 识别后无需
/// 引入额外的编码转换依赖即可无损解码。
pub(crate) fn decode_text_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_be_bytes(pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(stripped).into_owned()
}

/// 规范化导入的相对文件夹路径：统一 `/` 分隔符并过滤空段与 `.` / `..`。
pub(crate) fn normalize_folder_path(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}
