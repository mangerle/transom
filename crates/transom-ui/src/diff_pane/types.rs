//! 公共 Diff 视图的数据模型与配置选项。
//!
//! 设计原理（Why）：
//! - Git 差异视图与 SFTP 远程文件编辑的保存前预览都需要同一套展示配置
//!   （并排/统一、忽略空白、高亮粒度、行号、换行、折叠未变更上下文），
//!   收拢到此共享层后两边行为、菜单与快捷键口径完全一致，杜绝重复定义。

use gpui_kit::component::diff::DiffFile;

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

/// 后台计算产出的差异文档快照。
///
/// 由纯函数在后台线程池构造，再交由 UI 线程装载进 `DiffState`。
pub(crate) struct PreparedDiffDocument {
    pub files: Vec<DiffFile>,
    pub additions: usize,
    pub deletions: usize,
    pub hunk_count: usize,
}

impl PreparedDiffDocument {
    /// 当前忽略规则下是否没有任何差异行。
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty() || (self.additions == 0 && self.deletions == 0)
    }
}
