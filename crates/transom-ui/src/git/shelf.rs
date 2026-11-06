//! 搁置（Shelve）面板。
//!
//! 设计原理（Why）：
//! - 形态对齐 IDEA 的「搁置」页签：一列搁置项，每项可应用 / 弹出 / 删除，
//!   顶部一个「新建搁置」入口。这里做成模态框而不是第四个常驻页签，是因为
//!   工具窗口已经三栏占满，搁置是低频但需要完整列表的操作；
//! - 「应用」与「弹出」分列两个按钮：前者保留栈项、可反复应用到多个分支，
//!   后者会把栈项吃掉——这个区别必须让界面说清楚，不能合成一个按钮；
//! - 删除是破坏性且不可恢复的（`git stash drop` 之后只能靠 reflog 找回），
//!   因此删除走独立确认，不与列表点击混在一起。

use super::commit_modal::{accent_color, split_path};
use crate::app::AppShell;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{GitAction, GitProject, TabId};

impl AppShell {
    /// 搁置内联面板：提交页左栏的「搁置」页签内容。
    pub(crate) fn render_git_shelf_panel(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.git_state_of_tab(tab_id) else {
            return div().into_any_element();
        };
        let project = state.project.clone();
        let entries = state.stashes.clone();
        let expanded = state.expanded_stash.clone();
        let files = state.stash_files.clone();
        let loading = state.stash_files_loading;

        let refresh_project = project.clone();
        let toolbar = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(
                Button::new("git-shelf-refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCw)
                    .tooltip(rust_i18n::t!("git.menu.refresh").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_refresh(&refresh_project, cx);
                        this.git_reload_stashes(cx);
                    })),
            )
            .child(
                Button::new("git-shelf-new")
                    .primary()
                    .xsmall()
                    .icon(IconName::PackagePlus)
                    .label(rust_i18n::t!("git.shelf.create").to_string())
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.git_shelve_checked(tab_id, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!(
                        "{} ({})",
                        rust_i18n::t!("git.shelf.title"),
                        entries.len()
                    )),
            );

        let mut list = v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_1()
            .overflow_y_scrollbar();
        if entries.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("git.shelf.empty").to_string()),
            );
        }
        for entry in entries {
            let shown = if expanded.as_deref() == Some(entry.selector.as_str()) {
                files.as_slice()
            } else {
                &[]
            };
            let view = ShelfRowView {
                project: &project,
                selector: &entry.selector,
                display: entry.display(),
                expanded: expanded.as_deref() == Some(entry.selector.as_str()),
                loading,
                files: shown,
                tab_id,
            };
            list = list.child(self.shelf_row(&view, p, cx));
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(list)
            .into_any_element()
    }

    /// 一条搁置项：标题行 + 展开后的文件清单。
    ///
    /// 设计原理（Why）：折叠交互挂在标题上而不是整行——这一行右侧已有「应用」
    /// 「弹出」两个按钮，整行吞点击会让按按钮同时把清单展开，观感上像出了 bug。
    fn shelf_row(
        &mut self,
        view: &ShelfRowView<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = view.selector.to_string();
        let toggle_area = h_flex()
            .id(ElementId::Name(format!("git-shelf-head-{selector}").into()))
            .flex_1()
            .min_w_0()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .child(
                Icon::new(if view.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall()
                .text_color(p.muted_foreground),
            )
            .child(Icon::new(IconName::Package).xsmall().text_color(p.primary))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(p.foreground)
                            .child(view.display.to_string()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(selector.clone()),
                    ),
            )
            .on_click(cx.listener({
                let project = view.project.clone();
                let target = selector.clone();
                move |this, _, _window, cx| {
                    this.git_toggle_stash_files(&project, &target, cx);
                }
            }));
        let title = h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .items_center()
            .child(toggle_area)
            .child(
                Button::new(ElementId::Name(
                    format!("git-shelf-apply-{selector}").into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::Play)
                .tooltip(rust_i18n::t!("git.shelf.apply").to_string())
                .on_click(cx.listener({
                    let project = view.project.clone();
                    let target = selector.clone();
                    move |this, _, _window, cx| {
                        this.git_stash_action(&project, GitAction::StashApply, target.clone(), cx);
                    }
                })),
            )
            .child(
                Button::new(ElementId::Name(format!("git-shelf-pop-{selector}").into()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowDownToLine)
                    .tooltip(rust_i18n::t!("git.shelf.pop").to_string())
                    .on_click(cx.listener({
                        let project = view.project.clone();
                        let target = selector.clone();
                        move |this, _, _window, cx| {
                            this.git_stash_action(
                                &project,
                                GitAction::StashPop,
                                target.clone(),
                                cx,
                            );
                        }
                    })),
            )
            .context_menu({
                let app: Entity<AppShell> = cx.entity().clone();
                let project = view.project.clone();
                let target = selector.clone();
                move |menu, _window, _cx| {
                    render_shelf_menu(menu, app.clone(), &project, target.clone())
                }
            });

        let mut rows = v_flex()
            .gap_0()
            .border_color(p.divider(p.border))
            .border_b_1()
            .child(title);
        if !view.expanded {
            return rows.into_any_element();
        }
        if view.files.is_empty() {
            let text = if view.loading {
                rust_i18n::t!("git.shelf.files_loading").to_string()
            } else {
                rust_i18n::t!("git.shelf.no_files").to_string()
            };
            return rows
                .child(
                    div()
                        .pl_6()
                        .py_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(text),
                )
                .into_any_element();
        }
        for file in view.files {
            rows = rows.child(self.shelf_file_row(view, file, p, cx));
        }
        rows.into_any_element()
    }

    /// 搁置清单里的一行文件：点击即看「搁置前 vs 搁置后」的差异。
    fn shelf_file_row(
        &mut self,
        view: &ShelfRowView<'_>,
        file: &transom_core::StashFile,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visual = crate::git::change_visual(file.change);
        let (name, dir_prefix) = split_path(&file.path);
        let selector = view.selector.to_string();
        let tab_id = view.tab_id;
        let open_file = file.clone();
        let selected = self
            .git_state_of_tab(tab_id)
            .and_then(|state| state.diff.as_ref())
            .is_some_and(|diff| diff.path == file.path);
        let tooltip_path = file.path.clone();
        h_flex()
            .id(ElementId::Name(
                format!("git-shelf-file-{selector}-{}", file.path).into(),
            ))
            .h(px(22.0))
            .pl_6()
            .pr_2()
            .gap_2()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|style| style.bg(p.list_hover))
            .when(selected, |el| el.bg(p.list_active))
            .tooltip(move |window, cx| Tooltip::new(tooltip_path.clone()).build(window, cx))
            .child(
                Icon::new(visual.icon)
                    .xsmall()
                    .text_color(accent_color(visual.accent, p)),
            )
            .child(
                div()
                    .text_sm()
                    .flex_none()
                    .max_w(px(180.0))
                    .overflow_hidden()
                    .truncate()
                    .child(name),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(dir_prefix),
            )
            .child(
                file.old_path
                    .as_ref()
                    .map(|old| {
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(old.clone())
                            .into_any_element()
                    })
                    .unwrap_or_else(|| div().into_any_element()),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.git_open_stash_diff(tab_id, &selector, &open_file, cx);
            }))
            .into_any_element()
    }

    /// 把勾选中的改动收进一个新搁置项（IDEA 的「搁置更改」）。
    ///
    /// 一个文件都没勾时按「全部改动」处理：搁置的常见意图就是"把手上的活先放
    /// 下"，此时要求用户先勾满反而更啰嗦；未跟踪文件不裹进去，避免误收构建产物。
    pub(crate) fn git_shelve_checked(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(state) = self.git_state_of_tab(tab_id) else {
            return;
        };
        let message = if state.checked.is_empty() {
            rust_i18n::t!("git.shelf.auto_message").to_string()
        } else {
            format!(
                "{} ({})",
                rust_i18n::t!("git.shelf.title"),
                state.checked.len()
            )
        };
        let project = state.project.clone();
        self.git_stash_save(&project, message, false, cx);
        if let Some(state) = self.git_state_of_tab_mut(tab_id) {
            state.checked.clear();
        }
        cx.notify();
    }

    /// 重新拉取搁置列表。
    fn git_reload_stashes(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_git_tab() else {
            return;
        };
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        let project = state.project.clone();
        let request_id = transom_core::GitRequestId::new();
        state.pending.push(request_id);
        self.send_command(transom_core::GitCommand::ListStashes {
            project,
            request_id,
        });
        cx.notify();
    }
}

/// 搁置行的展示参数包：项目、选择器、标题、展开态、在途标记与清单一次传齐，
/// 平铺入参会超过规范上限，收成一个视图结构。
struct ShelfRowView<'a> {
    project: &'a GitProject,
    selector: &'a str,
    display: &'a str,
    expanded: bool,
    loading: bool,
    files: &'a [transom_core::StashFile],
    tab_id: TabId,
}

/// 搁置项右键菜单。
fn render_shelf_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    project: &GitProject,
    selector: String,
) -> PopupMenu {
    let drop_app = app.clone();
    let drop_project = project.clone();
    let drop_selector = selector.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.shelf.apply").to_string(),
        IconName::Play,
        {
            let item_app = app.clone();
            let item_project = project.clone();
            let item_selector = selector.clone();
            move |_, _, cx| {
                let project = item_project.clone();
                let selector = item_selector.clone();
                item_app.update(cx, |this, cx| {
                    this.git_stash_action(&project, GitAction::StashApply, selector, cx);
                });
            }
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.shelf.pop").to_string(),
        IconName::ArrowDownToLine,
        {
            let item_app = app.clone();
            let item_project = project.clone();
            move |_, _, cx| {
                let project = item_project.clone();
                let selector = selector.clone();
                item_app.update(cx, |this, cx| {
                    this.git_stash_action(&project, GitAction::StashPop, selector, cx);
                });
            }
        },
    ))
    .item(PopupMenuItem::separator())
    .item(menu_item(
        rust_i18n::t!("git.shelf.drop").to_string(),
        IconName::Trash,
        move |_, _, cx| {
            let project = drop_project.clone();
            let selector = drop_selector.clone();
            drop_app.update(cx, |this, cx| {
                this.git_stash_action(&project, GitAction::StashDrop, selector, cx);
            });
        },
    ))
}
