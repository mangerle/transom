//! 目录列举的数据层：条目形状、单层读取与新建目录名校验。
//!
//! 设计原理（Why）：这一层只碰文件系统与纯函数，不碰 gpui 状态，
//! 所以「隐藏项过滤、目录优先、大小写不敏感排序、防穿越校验」这些口径能在
//! 这里一次定死并被单测覆盖；面板与动作层读的是同一份结果，不会各列各的。

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// 一个目录一次最多列出的条目数：再多只取前若干并如实说明。
const MAX_ENTRIES: usize = 400;

pub(crate) struct GitDirEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
    /// 文件字节数；目录不显示大小，故为 0。
    pub(crate) size: u64,
}

/// 某个目录那一层的列举结果。
pub(crate) struct DirListing {
    pub(crate) entries: Vec<GitDirEntry>,
    pub(crate) truncated: bool,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
}

impl DirListing {
    /// 已排队、结果还没回来的那一格。
    pub(crate) fn pending() -> Self {
        Self {
            entries: Vec::new(),
            truncated: false,
            error: None,
            loading: true,
        }
    }
}

/// 树面板的运行期状态（不落盘：只是这一次选路过程中的视图）。
/// 列举一层目录：目录在前、文件在后，各自按名称升序（大小写不敏感）。
///
/// 为什么大小写不敏感：Linux 上大小写混排的目录名会让「看起来该在前面的」排到
/// 后面，与文件管理器的通行观感不一致。
pub(crate) fn list_entries(
    path: &Path,
    show_hidden: bool,
) -> (Vec<GitDirEntry>, bool, Option<String>) {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        log::warn!("列举目录失败，路径: {}", path.display());
        return (
            Vec::new(),
            false,
            Some(rust_i18n::t!("git.browser.read_failed").to_string()),
        );
    };
    let mut entries: Vec<GitDirEntry> = Vec::with_capacity(64);
    let mut truncated = false;
    for item in read_dir.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        let is_dir = file_type.is_dir();
        // 大小只在需要显示文件时取；目录不显示大小，省一次 metadata 系统调用。
        let size = if is_dir {
            0
        } else {
            item.metadata().map(|meta| meta.len()).unwrap_or_default()
        };
        entries.push(GitDirEntry {
            name,
            path: item.path(),
            is_dir,
            size,
        });
    }
    entries.sort_by(|left, right| {
        match right.is_dir.cmp(&left.is_dir) {
            Ordering::Equal => {}
            ordered => return ordered,
        }
        left.name.to_lowercase().cmp(&right.name.to_lowercase())
    });
    (entries, truncated, None)
}

/// 校验「在此新建目录」的名字。
///
/// 为什么必须校验：这个字符串会和目标目录拼成落盘路径，含 `/` 或 `..` 就等于
/// 让用户（或一次误触）把目录建到受控目录之外，路径防穿越是红线。
pub(crate) fn check_dir_name(name: &str) -> Option<&'static str> {
    if name.is_empty()
        || name.len() > 64
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(|ch| ch.is_control())
    {
        return Some("git.browser.bad_name");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::check_dir_name;

    #[test]
    fn plain_names_are_accepted() {
        assert!(check_dir_name("repo-a").is_none());
        assert!(check_dir_name("新仓库 2").is_none());
    }

    /// 防穿越：分隔符与 `..` 都会把目录建到目标之外，必须拒绝。
    #[test]
    fn traversal_and_separators_are_rejected() {
        assert!(check_dir_name("..").is_some());
        assert!(check_dir_name("a/b").is_some());
        assert!(check_dir_name("a\\b").is_some());
        assert!(check_dir_name("").is_some());
        assert!(check_dir_name("a\u{7}b").is_some());
    }

    #[test]
    fn overlong_names_are_rejected() {
        let long = "a".repeat(65);
        assert!(check_dir_name(&long).is_some());
    }
}
