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

pub(super) mod change_tree;
pub(crate) mod dir_tree;
pub(super) mod file_row;
mod group_menu;

pub(crate) use file_row::{amend_file_row, worktree_diff_request};

use crate::git::GitWorkspace;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{CommitEntry, GitAction};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 「修正」旁边的目标提交展示与选择：
/// 只显示哈希，悬停显示完整说明，小倒三角点击下拉选择未推送到远端的本地提交。
pub(crate) fn git_amend_target_button(
    workspace: Entity<GitWorkspace>,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let (candidates, selected) = {
        let state = &workspace.read(cx).state;
        let ahead = state.status.as_ref().map(|s| s.ahead as usize).unwrap_or(0);
        let has_upstream = state
            .status
            .as_ref()
            .and_then(|s| s.upstream.as_ref())
            .is_some();
        let entries = &state.log.entries;
        let list: Vec<CommitEntry> = if has_upstream {
            if ahead > 0 {
                entries.iter().take(ahead).cloned().collect()
            } else {
                entries.iter().take(1).cloned().collect()
            }
        } else {
            entries.iter().take(20).cloned().collect()
        };

        let current = state
            .amend_target
            .as_ref()
            .and_then(|oid| list.iter().find(|e| &e.oid == oid))
            .or_else(|| list.first())
            .cloned();
        (list, current)
    };

    let Some(selected) = selected else {
        return h_flex()
            .gap_1()
            .items_center()
            .px_2()
            .py_0p5()
            .rounded(px(3.0))
            .bg(p.muted)
            .child(
                AppIconElement::new(AppIcon::Ui(UiIcon::COMMIT))
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(div().text_xs().text_color(p.muted_foreground).child("-"))
            .into_any_element();
    };

    let short_oid = selected.short_oid();
    let tooltip = format!("{}  {}", short_oid, selected.subject);
    let selected_oid = selected.oid.clone();

    Button::new("git-amend-target")
        .ghost()
        .xsmall()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::COMMIT)).small())
        .label(short_oid)
        .tooltip(tooltip)
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for item in &candidates {
                let item_ws = workspace.clone();
                let item_oid = item.oid.clone();
                let is_active = item.oid == selected_oid;
                let label = format!("{}  {}", item.short_oid(), item.subject);
                menu = menu.item(menu_item(
                    label,
                    if is_active {
                        AppIcon::Ui(UiIcon::CHECK)
                    } else {
                        AppIcon::Ui(UiIcon::SHOW)
                    },
                    move |_menu, window, cx| {
                        let oid = item_oid.clone();
                        item_ws.update(cx, |ws, cx| {
                            ws.select_amend_target(oid, window, cx);
                        });
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 「分组依据」菜单：平铺 / 按目录。
pub(crate) fn git_grouping_menu(workspace: Entity<GitWorkspace>, by_directory: bool) -> AnyElement {
    let expand_ws = workspace.clone();
    let collapse_ws = workspace.clone();
    Button::new("git-grouping")
        .ghost()
        .xsmall()
        .child(AppIconElement::new(UiIcon::SHOW))
        .accessibility_label(rust_i18n::t!("git.group.by").to_string())
        .tooltip(rust_i18n::t!("git.group.by").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            let flat_ws = workspace.clone();
            let dir_ws = workspace.clone();
            let expand_ws = expand_ws.clone();
            let collapse_ws = collapse_ws.clone();
            menu.item(menu_item(
                rust_i18n::t!("git.group.flat").to_string(),
                if by_directory {
                    AppIcon::Ui(UiIcon::SHOW)
                } else {
                    AppIcon::Ui(UiIcon::CHECK)
                },
                move |_, _, cx| {
                    if !by_directory {
                        return;
                    }
                    flat_ws.update(cx, |ws, cx| {
                        ws.toggle_group_by_directory(cx);
                    });
                },
            ))
            .item(menu_item(
                rust_i18n::t!("git.group.by_directory").to_string(),
                if by_directory {
                    AppIcon::Ui(UiIcon::CHECK)
                } else {
                    AppIcon::Ui(UiIcon::SHOW)
                },
                move |_, _, cx| {
                    if by_directory {
                        return;
                    }
                    dir_ws.update(cx, |ws, cx| {
                        ws.toggle_group_by_directory(cx);
                    });
                },
            ))
            .item(PopupMenuItem::separator())
            .item(menu_item(
                rust_i18n::t!("git.group.expand_all").to_string(),
                AppIcon::Ui(UiIcon::CHEVRON_DOWN),
                move |_, _, cx| {
                    expand_ws.update(cx, |ws, cx| {
                        ws.expand_all_groups(cx);
                    });
                },
            ))
            .item(menu_item(
                rust_i18n::t!("git.group.collapse_all").to_string(),
                AppIcon::Ui(UiIcon::CHEVRON_RIGHT),
                move |_, _, cx| {
                    collapse_ws.update(cx, |ws, cx| {
                        ws.collapse_all_groups(cx);
                    });
                },
            ))
        })
        .into_any_element()
}

/// 一个带勾选框的行内开关（修正 / 提交后推送）。
pub(crate) fn git_check_toggle(
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

/// 勾选框三态：未选、部分选中（半选横杠）、全选。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckState {
    Unchecked,
    Indeterminate,
    Checked,
}

/// 勾选框本体（支持三态，蓝色背景配纯白对勾/横杠，对齐 IDEA）。
pub(crate) fn check_box_state(state: CheckState, p: &Palette) -> AnyElement {
    let filled = state != CheckState::Unchecked;
    div()
        .size(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .border_color(if filled { p.primary } else { p.border })
        .bg(if filled { p.primary } else { p.background })
        .map(|el| match state {
            CheckState::Checked => el.child(
                AppIconElement::new(UiIcon::CHECK)
                    .with_size(px(10.0))
                    .monochrome(gpui_kit::white()),
            ),
            CheckState::Indeterminate => el.child(
                div()
                    .w(px(8.0))
                    .h(px(1.5))
                    .rounded(px(0.5))
                    .bg(gpui_kit::white()),
            ),
            CheckState::Unchecked => el,
        })
        .into_any_element()
}

/// 勾选框本体（二态兼容封装）。
pub(crate) fn check_box(checked: bool, p: &Palette) -> AnyElement {
    check_box_state(
        if checked {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        },
        p,
    )
}

/// 只读行的勾选位占位：与勾选框同宽，保证忽略组与其它组的列对齐。
pub(crate) fn check_placeholder() -> AnyElement {
    div().size(px(13.0)).flex_none().into_any_element()
}

pub(crate) fn hint(text: String, p: &Palette) -> AnyElement {
    div()
        .p_3()
        .text_sm()
        .text_color(p.muted_foreground)
        .child(text)
        .into_any_element()
}
