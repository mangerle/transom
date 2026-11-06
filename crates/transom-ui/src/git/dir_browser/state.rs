//! 目录面板的状态：宿主、树、草稿行与路径框。
//!
//! 设计原理（Why）：状态单独成文件，是因为「谁持有这棵树」（弹窗还是克隆标签页）
//! 与「这棵树怎么动」是两件事——动作层要按宿主取到那棵树，而宿主形状不该被
//! 列举、缓存、展开集合这些实现细节淹没。

use super::listing::DirListing;
use crate::app::AppShell;
use crate::sftp::quick_paths::LocalQuickPaths;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 一棵目录树 + 它写回的路径框 + 一句错误。
pub(crate) struct DirPicker {
    pub browser: GitDirBrowser,
    /// 面板第一行的路径框：浏览结果写回这里，它也是宿主确认时的唯一真相。
    pub path: Entity<InputState>,
    /// 面板内的一句错误（新建目录名非法、建目录失败等）。
    pub error: Option<String>,
}

impl DirPicker {
    /// 用给定起点建面板。
    pub(crate) fn new(window: &mut Window, cx: &mut Context<AppShell>, placeholder: &str) -> Self {
        Self {
            browser: GitDirBrowser::new(),
            path: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            error: None,
        }
    }
}

pub(crate) struct GitDirBrowser {
    /// 树的根：换根由表头触发（上箭头、快捷目录、→ 进入路径）。
    pub root: PathBuf,
    /// 已列举过的目录 → 那一层的内容。
    pub(super) listed: HashMap<PathBuf, DirListing>,
    /// 展开中的目录集合。
    pub(super) expanded: HashSet<PathBuf>,
    /// 当前选中的目录：写回路径框的就是它。
    pub selected: Option<PathBuf>,
    /// 是否列出隐藏条目（以 `.` 开头）。
    pub show_hidden: bool,
    /// 快捷目录（主目录/桌面/下载），与 SFTP 本地列同一份解析。
    pub quick_paths: LocalQuickPaths,
    /// 行内新建目录的草稿：`parent` 决定它画在哪一层的末尾。
    pub draft: Option<DirDraft>,
}

/// 行内新建目录的草稿行。
///
/// 设计原理（Why）：不用独立输入框 + 按钮那条横杠，也不用系统式"命名对话框"——
/// 文件管理器的通行做法就是在目标层直接长出一行处于重命名态的条目，
/// 名字敲在哪、目录就建在哪，省掉"先选父级再填名字"的来回。
pub(crate) struct DirDraft {
    pub(crate) parent: PathBuf,
    pub(crate) input: Entity<InputState>,
}

impl GitDirBrowser {
    /// 以主目录为根建树；主目录取不到时退回根目录，面板不能没有起点。
    pub(crate) fn new() -> Self {
        let quick_paths = LocalQuickPaths::resolve();
        let root = quick_paths
            .home
            .clone()
            .unwrap_or_else(|| PathBuf::from("/"));
        let mut listed = HashMap::new();
        listed.insert(root.clone(), DirListing::pending());
        Self {
            root,
            listed,
            expanded: HashSet::new(),
            selected: None,
            show_hidden: false,
            quick_paths,
            draft: None,
        }
    }

    /// 换根：旧的展开与缓存都不再适用，从新根重新列一层。
    pub(super) fn re_root(&mut self, path: PathBuf) {
        self.root = path.clone();
        self.listed.clear();
        self.listed.insert(path, DirListing::pending());
        self.expanded.clear();
        // 换根后原来的草稿行不再属于可见层级，直接作废，避免留一行没归属的输入框。
        self.draft = None;
    }

    /// 展开或收起某个目录；返回「需要新列举一次」的路径（收起时为 `None`）。
    pub(super) fn toggle(&mut self, path: &Path) -> Option<PathBuf> {
        if self.expanded.remove(path) {
            return None;
        }
        self.expanded.insert(path.to_path_buf());
        // 首次展开才读盘；收起再展开用的是缓存。
        if !self.listed.contains_key(path) {
            self.listed
                .insert(path.to_path_buf(), DirListing::pending());
            return Some(path.to_path_buf());
        }
        None
    }

    /// 作废根与所有展开中目录的缓存，返回需要重新派发列举的路径。
    pub(super) fn relist_all(&mut self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::with_capacity(self.expanded.len() + 1);
        paths.push(self.root.clone());
        paths.extend(self.expanded.iter().cloned());
        for path in &paths {
            self.listed.insert(path.clone(), DirListing::pending());
        }
        paths
    }

    /// 展开某个目录（新建子目录后要让父级可见），不触发列举。
    pub(super) fn expand(&mut self, path: &Path) {
        self.expanded.insert(path.to_path_buf());
    }

    pub(super) fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// 某个目录是否还没列过（决定要不要补一次列举）。
    pub(super) fn is_listed(&self, path: &Path) -> bool {
        self.listed.contains_key(path)
    }

    /// 记下某个目录那一层，结果只写它自己那一格。
    pub(super) fn record(&mut self, path: &Path, listing: DirListing) -> bool {
        let Some(slot) = self.listed.get_mut(path) else {
            return false;
        };
        *slot = listing;
        true
    }

    /// 排队某个目录那一层（还没读过的才排）。
    pub(super) fn queue(&mut self, path: &Path) {
        if !self.is_listed(path) {
            self.listed
                .insert(path.to_path_buf(), DirListing::pending());
        }
    }

    /// 强制作废某一层并返回路径：新建/删除条目后，父级即便之前列过、缓存里
    /// 也不会有这条新记录，必须重读；`queue` 的"没列过才排"在这里是错的。
    pub(super) fn refresh_one(&mut self, path: &Path) -> PathBuf {
        self.listed
            .insert(path.to_path_buf(), DirListing::pending());
        path.to_path_buf()
    }
}

impl Default for GitDirBrowser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::GitDirBrowser;
    use crate::git::dir_browser::listing::DirListing;

    #[test]
    fn refresh_one_invalidates_an_already_listed_level() {
        let mut browser = GitDirBrowser::new();
        let parent = browser.root.join("Downloads");
        browser.listed.insert(parent.clone(), DirListing::pending());
        // 先落一格已完成结果，模拟“展开时已经读过空目录”。
        if let Some(slot) = browser.listed.get_mut(&parent) {
            slot.loading = false;
        }
        assert!(browser.is_listed(&parent));
        browser.refresh_one(&parent);
        assert!(
            browser.listed[&parent].loading,
            "新建条目后父级必须回到待读取状态，否则新目录进不了缓存"
        );
    }
}
