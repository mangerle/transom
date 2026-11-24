//! multipart 文件值的路径规范化与本地状态检查。
//!
//! 文件字段与文本字段共享项目 JSON 中的 `value` 字符串，但 UI 必须把文件路径
//! 当作独立草稿保存，避免切换值类型时把 `file://` 路径当成普通文本发送。

use std::path::Path;
use transom_api::MAX_UPLOAD_BYTES;

/// 文件字段在界面上的检查状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum FormFileStatus {
    /// 尚未检查，例如刚导入项目或刚切换到文件类型。
    #[default]
    Unknown,
    /// 正在后台读取元数据。
    Checking,
    /// 文件可用，保存实际字节数。
    Ready { size: u64 },
    /// 文件不可用；保留具体原因供行内提示。
    Invalid(FormFileIssue),
}

/// 文件不可用的原因；文案由视图层负责国际化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FormFileIssue {
    Missing,
    NotFile,
    TooLarge { size: u64, limit: u64 },
    Unreadable { reason: String },
}

/// 从持久化的字段值中取出文件系统路径。
pub(super) fn path_from_field_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.strip_prefix("file://").unwrap_or(value).to_string())
}

/// 把文件系统路径编码为项目文件中的稳定值。
pub(super) fn field_value_from_path(path: &str) -> String {
    format!("file://{}", path.trim())
}

/// 取文件展示名；路径没有文件名时回退到完整路径，避免出现空按钮。
pub(super) fn file_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// 检查文件是否存在、是否为普通文件以及是否超过上传上限。
///
/// 该函数只读取元数据，不读取文件内容，调用方必须放到阻塞线程池执行。
pub(super) fn inspect_file(path: &str) -> FormFileStatus {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return FormFileStatus::Invalid(FormFileIssue::Missing);
        }
        Err(error) => {
            return FormFileStatus::Invalid(FormFileIssue::Unreadable {
                reason: error.to_string(),
            });
        }
    };
    if !metadata.is_file() {
        return FormFileStatus::Invalid(FormFileIssue::NotFile);
    }
    let size = metadata.len();
    if size > MAX_UPLOAD_BYTES {
        return FormFileStatus::Invalid(FormFileIssue::TooLarge {
            size,
            limit: MAX_UPLOAD_BYTES,
        });
    }
    FormFileStatus::Ready { size }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_file_urls_and_names() {
        assert_eq!(
            path_from_field_value(" file:///tmp/demo.txt ").as_deref(),
            Some("/tmp/demo.txt")
        );
        assert_eq!(path_from_field_value("   "), None);
        assert_eq!(
            field_value_from_path("/tmp/demo.txt"),
            "file:///tmp/demo.txt"
        );
        assert_eq!(file_display_name("/tmp/demo.txt"), "demo.txt");
    }

    #[test]
    fn inspects_existing_file_and_reports_missing_file() {
        let directory = tempfile::tempdir().expect("创建临时目录");
        let path = directory.path().join("demo.txt");
        std::fs::write(&path, b"hello").expect("写入测试文件");
        let oversized = directory.path().join("oversized.bin");
        let file = std::fs::File::create(&oversized).expect("创建超限测试文件");
        file.set_len(MAX_UPLOAD_BYTES.saturating_add(1))
            .expect("设置超限文件长度");

        assert_eq!(
            inspect_file(path.to_string_lossy().as_ref()),
            FormFileStatus::Ready { size: 5 }
        );
        assert_eq!(
            inspect_file(oversized.to_string_lossy().as_ref()),
            FormFileStatus::Invalid(FormFileIssue::TooLarge {
                size: MAX_UPLOAD_BYTES.saturating_add(1),
                limit: MAX_UPLOAD_BYTES,
            })
        );
        assert_eq!(
            inspect_file(directory.path().to_string_lossy().as_ref()),
            FormFileStatus::Invalid(FormFileIssue::NotFile)
        );
        assert_eq!(
            inspect_file(
                directory
                    .path()
                    .join("missing.txt")
                    .to_string_lossy()
                    .as_ref()
            ),
            FormFileStatus::Invalid(FormFileIssue::Missing)
        );
    }
}
