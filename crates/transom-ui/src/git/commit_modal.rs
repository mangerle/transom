//! 提交面板（模态框）与本地改动树。
//!
//! 设计原理（Why）：
//! - 改动树分三组，顺序与 IDEA 一致：已暂存的更改 / 更改 / 未进行版本管理的
//!   文件。三组对应 git 的三个真实状态区间（索引侧、工作区侧、未跟踪），
//!   不是随意分类；
//! - 分组头可折叠、可整组勾选，行内显示「文件名 + 灰色目录后缀」：这是 IDEA
//!   提交面板的默认（平铺）形态。按目录折叠是它的「分组依据」选项，等日志侧
//!   的过滤器接完再补，避免同时改两处；
//! - 提交面板现在常驻在标签页的提交页里：左侧改动树 + 提交消息 + 按钮，右侧
//!   直接看差异，不再是模态框；勾选与折叠态都存在标签页状态里，切到日志再切
//!   回来不丢；
//! - 勾选集合存在标签页状态里而不是模态框里：关掉模态框再打开，勾选不该丢。

mod change_tree;
mod file_row;
mod group_menu;

pub(crate) use file_row::{amend_file_row, worktree_diff_request};

use crate::app::AppShell;
use crate::git::ChangeAccent;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{GitAction, GitProject};

impl AppShell {
    /// 「修正」旁边的目标提交展示：对应 IDEA 的「上次提交 ▾」。
    ///
    /// 为什么不做成可下拉选任意提交：`git commit --amend` 只能改写 HEAD，
    /// 改写历史中间的提交要走 fixup + 自动压缩（交互式变基），那是另一条
    /// 尚未实现的能力。给一个能选但选了不生效的下拉框等于骗人。
    pub(crate) fn git_amend_target_button(
        &mut self,
        project: &GitProject,
        p: &Palette,
    ) -> AnyElement {
        let head = self
            .git_state_of(project.id)
            .and_then(|state| state.log.entries.first())
            .map(|entry| format!("{}  {}", entry.short_oid(), entry.subject))
            .unwrap_or_else(|| rust_i18n::t!("git.commit.no_commit").to_string());
        h_flex()
            .gap_1()
            .items_center()
            .px_2()
            .py_0p5()
            .rounded(px(3.0))
            .bg(p.muted)
            .child(
                Icon::new(IconName::GitCommitHorizontal)
                    .xsmall()
                    .text_color(p.primary),
            )
            .child(div().text_xs().text_color(p.muted_foreground).child(head))
            .into_any_element()
    }

    /// 「分组依据」菜单：平铺 / 按目录。
    ///
    /// 菜单回调只拿得到 `&mut App`，因此这里显式带上 `Entity<AppShell>`：
    /// 与 `log_view` 的引用栏同一套写法，避免在渲染期持有可变借用。
    pub(crate) fn git_grouping_menu(
        &mut self,
        app: Entity<AppShell>,
        project: GitProject,
        by_directory: bool,
    ) -> AnyElement {
        let expand_app = app.clone();
        let expand_project = project.clone();
        let collapse_app = app.clone();
        let collapse_project = project.clone();
        Button::new("git-grouping")
            .ghost()
            .xsmall()
            .icon(IconName::Eye)
            .tooltip(rust_i18n::t!("git.group.by").to_string())
            .dropdown_menu(move |menu, _window, _cx| {
                let flat_app = app.clone();
                let flat_project = project.clone();
                let dir_app = app.clone();
                let dir_project = project.clone();
                let expand_app = expand_app.clone();
                let expand_project = expand_project.clone();
                let collapse_app = collapse_app.clone();
                let collapse_project = collapse_project.clone();
                menu.item(menu_item(
                    rust_i18n::t!("git.group.flat").to_string(),
                    if by_directory {
                        IconName::Circle
                    } else {
                        IconName::Check
                    },
                    move |_, _, cx| {
                        if !by_directory {
                            return;
                        }
                        let target = flat_project.clone();
                        flat_app.update(cx, |this, cx| {
                            this.git_toggle_group_by_directory(&target, cx);
                        });
                    },
                ))
                .item(menu_item(
                    rust_i18n::t!("git.group.by_directory").to_string(),
                    if by_directory {
                        IconName::Check
                    } else {
                        IconName::Circle
                    },
                    move |_, _, cx| {
                        if by_directory {
                            return;
                        }
                        let target = dir_project.clone();
                        dir_app.update(cx, |this, cx| {
                            this.git_toggle_group_by_directory(&target, cx);
                        });
                    },
                ))
                .item(PopupMenuItem::separator())
                .item(menu_item(
                    rust_i18n::t!("git.group.expand_all").to_string(),
                    IconName::ChevronDown,
                    move |_, _, cx| {
                        let target = expand_project.clone();
                        expand_app.update(cx, |this, cx| {
                            this.git_expand_all_groups(&target, cx);
                        });
                    },
                ))
                .item(menu_item(
                    rust_i18n::t!("git.group.collapse_all").to_string(),
                    IconName::ChevronRight,
                    move |_, _, cx| {
                        let target = collapse_project.clone();
                        collapse_app.update(cx, |this, cx| {
                            this.git_collapse_all_groups(&target, cx);
                        });
                    },
                ))
            })
            .into_any_element()
    }

    /// 一个带勾选框的行内开关（修正 / 提交后推送）。
    pub(crate) fn git_check_toggle(
        &mut self,
        id: &'static str,
        label: String,
        checked: bool,
        p: &Palette,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        h_flex()
            .id(ElementId::Name(id.into()))
            .gap_1()
            .items_center()
            .cursor_pointer()
            .child(check_box(checked, p))
            .child(div().text_xs().text_color(p.muted_foreground).child(label))
            .on_click(handler)
            .into_any_element()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeGroup {
    Staged,
    Unstaged,
    Untracked,
    Ignored,
}

impl ChangeGroup {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Unstaged => "changes",
            Self::Untracked => "unversioned",
            Self::Ignored => "ignored",
        }
    }

    pub(crate) fn title(self) -> String {
        let key = match self {
            Self::Staged => "git.group.staged",
            Self::Unstaged => "git.group.changes",
            Self::Untracked => "git.group.unversioned",
            Self::Ignored => "git.group.ignored",
        };
        rust_i18n::t!(key).to_string()
    }

    /// 该组是否可操作。
    ///
    /// 设计原理（Why）：被 .gitignore 命中的文件不在版本管理内——git 既不能暂存
    /// 它们，也没有可比对的基线，唯一常规动作是改忽略规则（本版本未实现）。
    /// 因此忽略组只读展示，不给勾选框、差异按钮和整组菜单，
    /// 避免摆出点了没反应的控件骗人。
    pub(crate) fn actionable(self) -> bool {
        self != Self::Ignored
    }

    /// 该组对应的整组动作：暂存 / 取消暂存 / 删除未跟踪；忽略组没有任何动作。
    pub(crate) fn bulk_action(self) -> Option<GitAction> {
        match self {
            Self::Staged => Some(GitAction::Unstage),
            Self::Untracked => Some(GitAction::DeleteUntracked),
            Self::Unstaged => Some(GitAction::Stage),
            Self::Ignored => None,
        }
    }
}

/// 改动树的显示参数包：勾选集合与两种折叠状态。
pub(crate) struct GroupDisplay {
    pub checked: HashSet<String>,
    pub collapsed_groups: HashSet<String>,
    pub by_directory: bool,
    pub collapsed_dirs: HashSet<String>,
}

/// 拆出「文件名」与「所在目录前缀」，目录在前、文件名在后。
pub(crate) fn split_path(path: &str) -> (String, String) {
    match path.rfind('/') {
        Some(at) => (path[at + 1..].to_string(), path[..at].to_string()),
        None => (path.to_string(), String::new()),
    }
}

/// 勾选框本体。
pub(crate) fn check_box(checked: bool, p: &Palette) -> AnyElement {
    div()
        .size(px(13.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.0))
        .border_1()
        .border_color(if checked { p.primary } else { p.border })
        .bg(if checked { p.primary } else { p.background })
        .when(checked, |el| {
            el.child(
                Icon::new(IconName::Check)
                    .xsmall()
                    .text_color(p.sidebar_accent_foreground),
            )
        })
        .into_any_element()
}

/// 只读行的勾选位占位：与勾选框同宽，保证忽略组与其它组的列对齐。
pub(crate) fn check_placeholder() -> AnyElement {
    div().size(px(13.0)).flex_none().into_any_element()
}

pub(crate) fn visual_letter(accent: ChangeAccent) -> &'static str {
    match accent {
        ChangeAccent::Added => "A",
        ChangeAccent::Modified => "M",
        ChangeAccent::Deleted => "D",
        ChangeAccent::Conflict => "!",
        ChangeAccent::Neutral => "?",
    }
}

pub(crate) fn accent_color(accent: ChangeAccent, p: &Palette) -> Hsla {
    match accent {
        ChangeAccent::Added => p.success,
        ChangeAccent::Modified => p.warning,
        ChangeAccent::Deleted => p.danger,
        ChangeAccent::Conflict => p.danger,
        ChangeAccent::Neutral => p.muted_foreground,
    }
}

pub(crate) fn hint(text: String, p: &Palette) -> AnyElement {
    div()
        .p_3()
        .text_sm()
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}
