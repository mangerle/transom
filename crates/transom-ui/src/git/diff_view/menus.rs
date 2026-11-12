//! Git 差异工具条扩展菜单（替代原置灰占位项，提供真实可用的上下文折叠、刷新与批量标签操作）。

use crate::app::AppShell;
use crate::diff_pane::DiffViewOptions;
use crate::git::actions::DiffTabCloseScope;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{GitRequestId, TabId};
use transom_ui_icons::UiIcon;

/// 差异工具条「更多操作」菜单渲染参数。
pub(super) struct DiffMoreMenuParams {
    pub app: Entity<AppShell>,
    pub tab_id: TabId,
    pub active_request_id: GitRequestId,
    pub total_tabs: usize,
    pub collapse_unchanged: bool,
}

/// 齿轮扩展菜单：展开/收起全部未变更上下文、刷新当前差异、打开源文件、批量关闭差异标签。
pub(super) fn render_settings_menu(params: DiffMoreMenuParams) -> AnyElement {
    let DiffMoreMenuParams {
        app,
        tab_id,
        active_request_id,
        total_tabs,
        collapse_unchanged,
    } = params;
    Button::new("git-diff-gear")
        .ghost()
        .xsmall()
        .icon(UiIcon::SETTINGS)
        .accessibility_label(rust_i18n::t!("common.more").to_string())
        .tooltip(rust_i18n::t!("common.more").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            let toggle_app = app.clone();
            let refresh_app = app.clone();
            let open_app = app.clone();
            let close_other_app = app.clone();
            let close_all_app = app.clone();
            let collapse_label = if collapse_unchanged {
                rust_i18n::t!("git.diff.expand_all_unchanged").to_string()
            } else {
                rust_i18n::t!("git.diff.collapse_unchanged_action").to_string()
            };
            menu.item(PopupMenuItem::new(collapse_label).on_click(move |_, _, cx| {
                let next_collapse = !collapse_unchanged;
                toggle_app.update(cx, |this, cx| {
                    this.git_update_diff_options(
                        tab_id,
                        Box::new(move |opts: &mut DiffViewOptions| {
                            opts.collapse_unchanged = next_collapse
                        }),
                        cx,
                    );
                });
            }))
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.refresh").to_string()).on_click(
                    move |_, _, cx| {
                        refresh_app.update(cx, |this, cx| {
                            this.git_refresh_active_diff(tab_id, cx);
                        });
                    },
                ),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.open_source").to_string()).on_click(
                    move |_, _, cx| {
                        open_app.update(cx, |this, cx| {
                            this.git_open_diff_source(tab_id, cx);
                        });
                    },
                ),
            )
            .separator()
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_other_tabs").to_string())
                    .disabled(total_tabs <= 1)
                    .on_click(move |_, _, cx| {
                        close_other_app.update(cx, |this, cx| {
                            this.git_close_diff_tabs_scope(
                                tab_id,
                                Some(active_request_id),
                                DiffTabCloseScope::Other,
                                cx,
                            );
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.close_all_tabs").to_string())
                    .disabled(total_tabs == 0)
                    .on_click(move |_, _, cx| {
                        close_all_app.update(cx, |this, cx| {
                            this.git_close_diff_tabs_scope(
                                tab_id,
                                None,
                                DiffTabCloseScope::All,
                                cx,
                            );
                        });
                    }),
            )
        })
        .into_any_element()
}
