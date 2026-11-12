//! 公共 Diff 视图的数据模型与配置选项。
//!
//! 设计原理（Why）：
//! - Git 差异视图与 SFTP 远程文件编辑的保存前预览都需要同一套展示配置
//!   （并排/统一、忽略空白、高亮粒度、行号、换行、折叠未变更上下文），
//!   收拢到此共享层后两边行为、菜单与快捷键口径完全一致，杜绝重复定义。

use std::rc::Rc;

use super::hunk::PreparedHunk;
use gpui_kit::component::diff::DiffFile;
use gpui_kit::{App, Window};

/// 公共 Diff 面板按钮交互的无状态 UI 回调签名。
pub(crate) type DiffUiAction = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

/// 空白处理档位（对应差异工具条的「忽略差异」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum IgnoreWhitespace {
    /// 不忽略。
    #[default]
    None,
    /// 修整空白：去掉行首行尾空白。
    Trim,
    /// 忽略所有空白。
    All,
    /// 忽略空白与空行。
    BlankLines,
}

/// 差异高亮粒度（对应工具条的「高亮显示差异」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DiffHighlight {
    /// 整行。
    Line,
    /// 行内单词级高亮（默认开启，便于识别局部编辑）。
    #[default]
    Word,
    /// 不做差异背景着色，仅看代码结构与语法高亮。
    None,
}

/// 差异视图通用显示选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiffViewOptions {
    /// 并排两栏还是统一单栏。
    pub side_by_side: bool,
    /// 是否显示两侧行号。
    pub show_line_numbers: bool,
    /// 长行是否自动换行。
    pub wrap_lines: bool,
    /// 空白处理档位。
    pub ignore_whitespace: IgnoreWhitespace,
    /// 高亮粒度。
    pub highlight: DiffHighlight,
    /// 是否折叠连续未变更上下文行（默认开启，可局部展开）。
    pub collapse_unchanged: bool,
}

impl Default for DiffViewOptions {
    fn default() -> Self {
        Self {
            side_by_side: true,
            show_line_numbers: true,
            // `gpui_kit::Diff` 仅在开启软换行时按视口 50%/50% 均分左右栏；
            // 关闭后单栏会按全文件最长行撑宽并把右栏挤出视口，故默认开启以保并排均分。
            wrap_lines: true,
            ignore_whitespace: IgnoreWhitespace::default(),
            highlight: DiffHighlight::default(),
            collapse_unchanged: true,
        }
    }
}

impl DiffViewOptions {
    /// 从持久化 `GitSettings` 构造差异显示选项。
    pub(crate) fn from_git_settings(git: &transom_core::GitSettings) -> Self {
        let side_by_side = match git.default_diff_mode.as_str() {
            "unified" => false,
            "split" => true,
            _ => true,
        };
        let ignore_whitespace = match git.diff_ignore_whitespace_level.as_deref() {
            Some("none") => IgnoreWhitespace::None,
            Some("trim") => IgnoreWhitespace::Trim,
            Some("all") => IgnoreWhitespace::All,
            Some("blank") => IgnoreWhitespace::BlankLines,
            _ if git.ignore_whitespace => IgnoreWhitespace::All,
            _ => IgnoreWhitespace::None,
        };
        let highlight = match git.diff_highlight.as_str() {
            "line" => DiffHighlight::Line,
            "none" => DiffHighlight::None,
            _ => DiffHighlight::Word,
        };
        Self {
            side_by_side,
            show_line_numbers: git.diff_show_line_numbers,
            wrap_lines: git.diff_wrap_lines,
            ignore_whitespace,
            highlight,
            collapse_unchanged: git.diff_collapse_unchanged,
        }
    }

    /// 将当前差异显示选项回写到持久化 `GitSettings`。
    pub(crate) fn write_to_git_settings(&self, git: &mut transom_core::GitSettings) {
        git.default_diff_mode = if self.side_by_side {
            "split".to_string()
        } else {
            "unified".to_string()
        };
        let (ignore_flag, level_str) = match self.ignore_whitespace {
            IgnoreWhitespace::None => (false, "none"),
            IgnoreWhitespace::Trim => (true, "trim"),
            IgnoreWhitespace::All => (true, "all"),
            IgnoreWhitespace::BlankLines => (true, "blank"),
        };
        git.ignore_whitespace = ignore_flag;
        git.diff_ignore_whitespace_level = Some(level_str.to_string());
        git.diff_show_line_numbers = self.show_line_numbers;
        git.diff_wrap_lines = self.wrap_lines;
        git.diff_highlight = match self.highlight {
            DiffHighlight::Line => "line".to_string(),
            DiffHighlight::Word => "word".to_string(),
            DiffHighlight::None => "none".to_string(),
        };
        git.diff_collapse_unchanged = self.collapse_unchanged;
    }
}

/// 后台计算产出的差异文档快照。
///
/// 由纯函数在后台线程池构造，再交由 UI 线程装载进 `DiffState`。
pub(crate) struct PreparedDiffDocument {
    pub files: Vec<DiffFile>,
    pub additions: usize,
    pub deletions: usize,
    pub hunk_count: usize,
    pub hunks: Vec<PreparedHunk>,
}

impl PreparedDiffDocument {
    /// 当前忽略规则下是否没有任何差异行。
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty() || (self.additions == 0 && self.deletions == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_view_options_roundtrip_with_git_settings() {
        let mut settings = transom_core::GitSettings::default();
        let original = DiffViewOptions {
            side_by_side: false,
            show_line_numbers: false,
            wrap_lines: false,
            ignore_whitespace: IgnoreWhitespace::BlankLines,
            highlight: DiffHighlight::Line,
            collapse_unchanged: false,
        };
        original.write_to_git_settings(&mut settings);
        let restored = DiffViewOptions::from_git_settings(&settings);
        assert_eq!(restored, original);
        assert!(settings.ignore_whitespace);
        assert_eq!(settings.default_diff_mode, "unified");
    }
}
