//! 树的行构建：把「展开中的目录」这一棵树压平成一行行。
//!
//! 设计原理（Why）：
//! - 压平与画行分开，是因为压平要处理深度上限、还没读过的格子、附加说明这些纯数据
//!   问题，而画行只关心样式与点击；混在一起时，「子级紧跟父级」这个顺序约束很容易被
//!   样式改动碰坏；
//! - 压平必须深度优先交错：展开某个目录后，它的子级要紧跟在它下面。先铺完一层再处理
//!   子目录的写法会把子级甩到整张表末尾，看上去就是「点了展开却没东西，滚到最底下才
//!   冒出来」；
//! - 行只有三种：条目、说明（读取中 / 读取失败 / 已截断）、行内新建目录的草稿行。
//!   草稿行进压平逻辑而不是画图层，是因为它的语义是「在这个目录里新建」，位置必须
//!   跟着那个父级走——画完再插到列表末尾会出现"在 A 里新建、输入框画在 Z 下面"。

use super::GitDirBrowser;
use super::listing::{DirListing, GitDirEntry};
use std::path::{Path, PathBuf};

/// 树的最大渲染深度：深度由用户一路点出来，设上限既防止一路点到极深层级，
/// 也防止符号链接成环时列表停不下来。
const MAX_DEPTH: usize = 16;

/// 一行的内容种类。
pub(crate) enum RowBody {
    /// 目录或文件条目；`expanded` 只对该条目是目录时有意义。
    Entry { entry: GitDirEntry, expanded: bool },
    /// 一句说明，缩进跟它所属的父级一致。
    Note(String),
    /// 行内新建目录：输入行已经就位，等用户敲名字。
    Draft,
}

/// 一行树：缩进深度 + 内容。
pub(crate) struct GitRow {
    pub(crate) depth: usize,
    pub(crate) body: RowBody,
}

/// 栈里的挂起项：要么直接成行，要么先展开一个目录。
enum Pending {
    Emit(GitRow),
    Expand { path: PathBuf, depth: usize },
}

/// 展开中的树扁平化成渲染行：深度优先，子级紧跟在父级后面。
///
/// 为什么用显式栈而不是递归：深度由用户一路点出来，递归等于把调用栈交给用户操作，
/// 符号链接成环时就是栈溢出；显式栈配 `MAX_DEPTH` 上限，成环最多渲染到上限层。
pub(crate) fn build_rows(browser: &GitDirBrowser) -> Vec<GitRow> {
    let mut rows = Vec::with_capacity(32);
    let mut stack = vec![Pending::Expand {
        path: browser.root.clone(),
        depth: 0,
    }];
    while let Some(item) = stack.pop() {
        match item {
            Pending::Emit(row) => rows.push(row),
            Pending::Expand { path, depth } => push_level(browser, &path, depth, &mut stack),
        }
    }
    rows
}

/// 把某个目录那一层压成挂起项后倒序入栈。
///
/// 栈是后压先弹，所以倒序压才得到正序输出；展开中的子目录紧跟在自己的父行之后入栈，
/// 于是它的整棵子树都插在同层下一行前面。
fn push_level(browser: &GitDirBrowser, parent: &Path, depth: usize, stack: &mut Vec<Pending>) {
    let Some(listing) = browser.listed.get(parent) else {
        return;
    };
    let mut emits: Vec<Pending> = Vec::with_capacity(listing.entries.len() + 4);
    for entry in &listing.entries {
        let expanded = entry.is_dir && browser.is_expanded(&entry.path);
        emits.push(Pending::Emit(GitRow {
            depth,
            body: RowBody::Entry {
                entry: GitDirEntry {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    is_dir: entry.is_dir,
                    size: entry.size,
                },
                expanded,
            },
        }));
        if expanded {
            // 这一项比同层下一行先出栈：子级紧跟父行。
            emits.push(Pending::Expand {
                path: entry.path.clone(),
                depth: depth + 1,
            });
        }
    }
    // 草稿行排在它所属父级的条目之后：新建的是这一层的又一个目录。
    if browser
        .draft
        .as_ref()
        .is_some_and(|draft| draft.parent == parent)
    {
        emits.push(Pending::Emit(GitRow {
            depth,
            body: RowBody::Draft,
        }));
    }
    for note in listing_notes(listing) {
        emits.push(Pending::Emit(GitRow {
            depth,
            body: RowBody::Note(note),
        }));
    }
    if depth >= MAX_DEPTH {
        // 到上限就不再往下展开：只保留已经排好的行，成环时列表也不会停不下来。
        emits.retain(|item| !matches!(item, Pending::Expand { .. }));
    }
    for item in emits.into_iter().rev() {
        stack.push(item);
    }
}

/// 某一格的附加说明（读取中 / 读取失败 / 已截断）。
fn listing_notes(listing: &DirListing) -> Vec<String> {
    let mut notes = Vec::with_capacity(3);
    if listing.loading {
        notes.push(rust_i18n::t!("git.browser.loading").to_string());
    }
    if let Some(error) = &listing.error {
        notes.push(error.clone());
    }
    if listing.truncated {
        notes.push(rust_i18n::t!("git.browser.truncated").to_string());
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::{RowBody, build_rows};
    use crate::git::dir_browser::GitDirBrowser;
    use crate::git::dir_browser::listing::{DirListing, GitDirEntry};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn dir(path: PathBuf) -> GitDirEntry {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        GitDirEntry {
            name,
            is_dir: true,
            size: 0,
            path,
        }
    }

    fn listing(entries: Vec<GitDirEntry>) -> DirListing {
        DirListing {
            entries,
            truncated: false,
            error: None,
            loading: false,
        }
    }

    /// 根下 `first`（已展开，里面有一个子目录 `inner`）与 `second`。
    fn two_level() -> GitDirBrowser {
        let root = PathBuf::from("/root");
        let first = root.join("first");
        let inner = first.join("inner");
        let mut browser = GitDirBrowser::new();
        browser.root = root.clone();
        browser.expanded = HashSet::from([first.clone()]);
        browser.listed = HashMap::from([
            (
                root.clone(),
                listing(vec![dir(first.clone()), dir(root.join("second"))]),
            ),
            (first, listing(vec![dir(inner)])),
        ]);
        browser
    }

    fn labels(browser: &GitDirBrowser) -> Vec<(String, usize)> {
        build_rows(browser)
            .iter()
            .map(|row| match &row.body {
                RowBody::Entry { entry, .. } => (entry.name.clone(), row.depth),
                RowBody::Note(text) => (format!("注:{text}"), row.depth),
                RowBody::Draft => ("草稿".into(), row.depth),
            })
            .collect()
    }

    #[test]
    fn expanded_children_follow_their_parent() {
        assert_eq!(
            labels(&two_level()),
            vec![
                ("first".into(), 0),
                ("inner".into(), 1),
                ("second".into(), 0),
            ],
            "子级必须紧跟父行，而不是掉到整张表末尾"
        );
    }

    #[test]
    fn collapsed_children_stay_hidden() {
        let mut browser = two_level();
        browser.expanded.clear();
        assert_eq!(
            labels(&browser),
            vec![("first".into(), 0), ("second".into(), 0)],
            "收起后子级一行都不该出现"
        );
    }

    #[test]
    fn notes_follow_the_level_they_belong_to() {
        let mut browser = two_level();
        let first = browser.root.join("first");
        browser
            .listed
            .get_mut(&first)
            .expect("first 已列举")
            .loading = true;
        let rows = labels(&browser);
        // 说明行属于 `first` 那一层：排在它的子级之后、同层下一行之前。
        // 文案按当前 locale 取，不断言具体字串，只断言它是第 1 层的一条说明。
        assert_eq!(rows[0], ("first".into(), 0), "实际行表: {rows:?}");
        assert_eq!(rows[1], ("inner".into(), 1), "实际行表: {rows:?}");
        assert!(rows[2].0.starts_with("注:"), "实际行表: {rows:?}");
        assert_eq!(
            rows[2].1, 1,
            "说明行要跟着它所属的层级缩进，实际行表: {rows:?}"
        );
        assert_eq!(rows[3], ("second".into(), 0), "实际行表: {rows:?}");
    }
}
