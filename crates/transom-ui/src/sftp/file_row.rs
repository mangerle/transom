//! 文件列表行的共用外壳、空态提示与 `..` 父目录注入。
//!
//! 设计原理（Why）：远端主栏、本地栏与并列远端三条列表此前各自复制同一套
//! 行外观与父目录注入逻辑，外观改动要同步三处、漏一处就出现视觉分叉。这里
//! 只收敛"三处必须一致"的部分；点击、拖拽与右键菜单仍留在各自模块，因为
//! 它们的业务语义确实不同。

use super::{FileRowContentOptions, render_file_row_content};
use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::*;
use std::path::Path;
use transom_core::RemoteFile;

use super::LocalFileEntry;

/// 一行文件条目的渲染输入。
pub(crate) struct FileRowProps<'a> {
    /// 行元素标识；各列表自行保证唯一（远端用完整路径，并列远端带列序号）。
    pub(crate) id: String,
    pub(crate) name: &'a str,
    pub(crate) is_dir: bool,
    pub(crate) is_parent: bool,
    pub(crate) size: u64,
    pub(crate) permissions: Option<u32>,
    pub(crate) selected: bool,
    pub(crate) palette: &'a Palette,
}

/// 构造统一外观的文件行：命中区、内边距、选中/hover 配色与行内容。
pub(crate) fn file_row(props: FileRowProps<'_>) -> Stateful<Div> {
    let p = props.palette;
    let row = h_flex()
        .id(ElementId::Name(props.id.into()))
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .rounded(p.radius.sm)
        .cursor_pointer();
    let row = if props.selected {
        row.bg(p.sidebar_accent)
            .text_color(p.sidebar_accent_foreground)
    } else {
        row.hover(|style| {
            style
                .bg(p.sidebar_accent)
                .text_color(p.sidebar_accent_foreground)
        })
    };
    row.child(render_file_row_content(FileRowContentOptions {
        name: props.name,
        is_dir: props.is_dir,
        is_parent: props.is_parent,
        size: props.size,
        permissions: props.permissions,
        palette: p,
    }))
}

/// 目录为空时的占位提示。
pub(crate) fn empty_dir_hint(p: &Palette) -> AnyElement {
    div()
        .p_3()
        .text_xs()
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("sftp.empty").to_string())
        .into_any_element()
}

/// 在当前远端路径非根时，向列表头部注入 `..` 父目录项。
///
/// `filter_query` 非空时只在它匹配 `..` 时注入；过滤状态下父目录与普通条目
/// 遵守同一套匹配规则，避免"过滤后仍能看到上一级"的错觉。
pub(crate) fn insert_parent_remote_entry(
    entries: &mut Vec<RemoteFile>,
    path: &str,
    filter_query: &str,
) {
    if path == "/" || path.is_empty() {
        return;
    }
    if !filter_query.is_empty() && !super::fuzzy_match_name("..", filter_query) {
        return;
    }
    entries.insert(
        0,
        RemoteFile {
            name: "..".to_string(),
            path: super::parent_remote(path),
            is_dir: true,
            is_symlink: false,
            size: 0,
            modified: None,
            permissions: None,
        },
    );
}

/// 在当前本地路径存在父目录时，向列表头部注入 `..` 父目录项。
pub(crate) fn insert_parent_local_entry(
    entries: &mut Vec<LocalFileEntry>,
    path: &Path,
    filter_query: &str,
) {
    let Some(parent) = path.parent() else {
        return;
    };
    if path == parent {
        return;
    }
    if !filter_query.is_empty() && !super::fuzzy_match_name("..", filter_query) {
        return;
    }
    entries.insert(
        0,
        LocalFileEntry {
            name: "..".to_string(),
            path: parent.to_path_buf(),
            is_dir: true,
            size: 0,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{LocalFileEntry, Path, insert_parent_local_entry, insert_parent_remote_entry};
    use transom_core::RemoteFile;

    fn remote_file(name: &str) -> RemoteFile {
        RemoteFile {
            name: name.to_string(),
            path: format!("/{name}"),
            is_dir: false,
            is_symlink: false,
            size: 0,
            modified: None,
            permissions: None,
        }
    }

    #[test]
    fn parent_remote_entry_skips_root_and_empty_path() {
        let mut entries = vec![remote_file("a")];
        insert_parent_remote_entry(&mut entries, "/", "");
        insert_parent_remote_entry(&mut entries, "", "");
        assert_eq!(entries.len(), 1, "根目录与空路径不得注入上级条目");

        insert_parent_remote_entry(&mut entries, "/home", "");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "..");
        assert_eq!(entries[0].path, "/");
    }

    #[test]
    fn parent_remote_entry_follows_filter() {
        let mut filtered = vec![remote_file("a")];
        insert_parent_remote_entry(&mut filtered, "/home", "zzz");
        assert_eq!(filtered.len(), 1, "过滤词不匹配时不应出现上级条目");

        let mut matched = vec![remote_file("a")];
        insert_parent_remote_entry(&mut matched, "/home", "..");
        assert_eq!(matched.len(), 2, "过滤词匹配时应保留上级条目");
    }

    #[test]
    fn parent_local_entry_skips_filesystem_root() {
        let mut entries = vec![LocalFileEntry {
            name: "a".to_string(),
            path: Path::new("/tmp/a").to_path_buf(),
            is_dir: false,
            size: 0,
        }];
        insert_parent_local_entry(&mut entries, Path::new("/"), "");
        assert_eq!(entries.len(), 1, "文件系统根目录不得注入上级条目");

        insert_parent_local_entry(&mut entries, Path::new("/tmp"), "");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "..");
        assert_eq!(entries[0].path, Path::new("/"));
    }
}
