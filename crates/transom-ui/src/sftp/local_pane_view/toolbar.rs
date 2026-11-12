//! SFTP 本地文件浏览器头部操作工具栏与快捷目录按钮。
//!
//! 设计原理（Why）：
//! - 实现初衷：隔离本地文件列顶部的路径导航、快捷目录（主目录、桌面、下载）与增删改按钮；
//! - 核心优势：使本地主视图更加清晰，避免数百行工具栏布局逻辑杂糅在主面板结构中。

use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::separator::Separator;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

use super::LocalQuickPaths;

/// 本地文件浏览器工具栏属性。
pub(super) struct LocalToolbarProps<'a> {
    pub(super) tab_id: TabId,
    pub(super) show_filter: bool,
    pub(super) show_hidden: bool,
    pub(super) local_path_input: &'a Entity<InputState>,
    pub(super) local_filter_input: &'a Entity<InputState>,
    pub(super) quick_paths: &'a LocalQuickPaths,
    pub(super) remote_available: bool,
    /// 当前是否恰好选中一个本地文件，且远端目录存在同名文件。
    pub(super) diff_ready: bool,
}

/// 渲染本地文件面板顶部的双行工具栏与快速路径跳转。
pub(super) fn render_local_toolbar(
    props: LocalToolbarProps<'_>,
    p: &Palette,
    cx: &mut Context<SftpPanelView>,
) -> AnyElement {
    let tab_id = props.tab_id;
    let show_filter = props.show_filter;
    let show_hidden = props.show_hidden;
    let local_path_input = props.local_path_input;
    let local_filter_input = props.local_filter_input;
    let quick_paths = props.quick_paths;
    let remote_available = props.remote_available;
    let diff_ready = props.diff_ready;

    v_flex()
        .p_2()
        .gap_1()
        .border_color(p.divider(p.sidebar_border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(
            crate::widgets::toolbar()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted_foreground)
                        .mr_1()
                        .child(rust_i18n::t!("sftp.local_title").to_string()),
                )
                .child(div().flex_1().child(Input::new(local_path_input).w_full()))
                .child(
                    crate::widgets::IconButton::new(
                        "local-up",
                        AppIcon::Ui(UiIcon::UP),
                        rust_i18n::t!("sftp.menu.go_up").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.local_go_up(tab_id, window, cx)
                        }),
                    ),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-refresh",
                        AppIcon::Ui(UiIcon::REFRESH),
                        rust_i18n::t!("sftp.nav.refresh_local_dir").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| this.local_refresh(cx))),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-enter",
                        AppIcon::Ui(UiIcon::RIGHT),
                        rust_i18n::t!("sftp.nav.enter_path").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.local_enter_path(tab_id, window, cx)
                    })),
                )
                .when(remote_available, |this| {
                    this.child(
                        crate::widgets::IconButton::new(
                            "local-diff-remote",
                            AppIcon::Ui(UiIcon::DIFF),
                            rust_i18n::t!("sftp.nav.diff_with_remote").to_string(),
                        )
                        .disabled(!diff_ready)
                        .size(crate::widgets::IconButtonSize::Regular)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_local_diff(cx);
                        })),
                    )
                })
                .when(remote_available, |this| {
                    this.child(
                        crate::widgets::IconButton::new(
                            "local-collapse",
                            AppIcon::Ui(UiIcon::LEFT),
                            rust_i18n::t!("sftp.sidebar.collapse_local").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Regular)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_local_collapse(cx);
                        })),
                    )
                }),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(local_quick_path_button(
                    "local-quick-home",
                    AppIcon::Ui(UiIcon::HOME_FOLDER),
                    rust_i18n::t!("sftp.home").to_string(),
                    quick_paths.home.clone(),
                    tab_id,
                    cx,
                ))
                .child(local_quick_path_button(
                    "local-quick-desktop",
                    AppIcon::Ui(UiIcon::DESKTOP),
                    rust_i18n::t!("sftp.desktop").to_string(),
                    quick_paths.desktop.clone(),
                    tab_id,
                    cx,
                ))
                .child(local_quick_path_button(
                    "local-quick-downloads",
                    AppIcon::Ui(UiIcon::DOWNLOAD),
                    rust_i18n::t!("sftp.downloads").to_string(),
                    quick_paths.downloads.clone(),
                    tab_id,
                    cx,
                ))
                .child(
                    Separator::vertical()
                        .h(px(14.0))
                        .mx_1()
                        .color(p.sidebar_border),
                )
                .when(remote_available, |this| {
                    this.child(
                        crate::widgets::IconButton::new(
                            "local-action-upload",
                            AppIcon::Ui(UiIcon::UPLOAD),
                            rust_i18n::t!("sftp.upload").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Regular)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.queue_upload(tab_id, cx);
                        })),
                    )
                })
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-mkdir",
                        AppIcon::Ui(UiIcon::NEW_FOLDER),
                        rust_i18n::t!("sftp.menu.new_local_folder").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_local_mkdir_modal(window, cx);
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-new-file",
                        AppIcon::Ui(UiIcon::ADD_FILE),
                        rust_i18n::t!("sftp.menu.new_local_file").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_local_new_file_modal(window, cx);
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-rename",
                        AppIcon::Ui(UiIcon::EDIT),
                        rust_i18n::t!("sftp.rename").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_local_rename_modal(window, cx);
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-edit",
                        AppIcon::Ui(UiIcon::REAL_INTENTION_BULB),
                        rust_i18n::t!("common.edit").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let selected = this.state.local.selected.clone();
                        if let Some(target) = selected {
                            this.local_edit_file(target, cx);
                        } else {
                            this.state.set_local_error(
                                rust_i18n::t!("sftp.row.select_local_to_edit").to_string(),
                            );
                            cx.notify();
                        }
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-properties",
                        AppIcon::Custom(CustomIcon::INFO),
                        rust_i18n::t!("sftp.menu.properties").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let path = this
                            .state
                            .local
                            .selected
                            .clone()
                            .or_else(|| Some(this.state.local.path.clone()));
                        if let Some(target) = path {
                            this.show_local_properties(target, cx);
                        }
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-delete",
                        AppIcon::Ui(UiIcon::DELETE),
                        rust_i18n::t!("sftp.delete").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.local_delete_selection(tab_id, cx);
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-action-open-fm",
                        AppIcon::Ui(UiIcon::FOLDER),
                        rust_i18n::t!("sftp.menu.open_file_manager").to_string(),
                    )
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.local_open_file_manager(cx);
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-toggle-hidden",
                        if show_hidden {
                            AppIcon::Ui(UiIcon::SHOW)
                        } else {
                            AppIcon::Custom(CustomIcon::EYE_OFF)
                        },
                        if show_hidden {
                            rust_i18n::t!("sftp.nav.hide_dotfiles").to_string()
                        } else {
                            rust_i18n::t!("sftp.nav.show_dotfiles").to_string()
                        },
                    )
                    .selected(show_hidden)
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_local_hidden(cx);
                    })),
                )
                .child(
                    crate::widgets::IconButton::new(
                        "local-toggle-filter",
                        AppIcon::Ui(UiIcon::SEARCH),
                        rust_i18n::t!("sftp.nav.toggle_filter").to_string(),
                    )
                    .selected(show_filter)
                    .size(crate::widgets::IconButtonSize::Regular)
                    .render()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_filter_local(window, cx);
                    })),
                ),
        )
        .when(show_filter, |this| {
            this.child(Input::new(local_filter_input).w_full())
        })
        .into_any_element()
}

fn local_quick_path_button(
    id: &'static str,
    icon: AppIcon,
    label: String,
    target: Option<PathBuf>,
    tab_id: TabId,
    cx: &mut Context<SftpPanelView>,
) -> AnyElement {
    let enabled = target.is_some();
    let tooltip = if enabled {
        label
    } else {
        format!("{label} ({})", rust_i18n::t!("sftp.missing_dir"))
    };
    let button = crate::widgets::IconButton::new(id, icon, tooltip)
        .size(crate::widgets::IconButtonSize::Regular)
        .disabled(!enabled)
        .render();
    let Some(target) = target else {
        return button.into_any_element();
    };
    button
        .on_click(cx.listener(move |this, _, window, cx| {
            this.local_navigate(tab_id, target.clone(), window, cx);
        }))
        .into_any_element()
}
