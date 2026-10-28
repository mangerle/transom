//! SFTP 本地文件浏览器头部操作工具栏与快捷目录按钮。
//!
//! 设计原理（Why）：
//! - 实现初衷：隔离本地文件列顶部的路径导航、快捷目录（主目录、桌面、下载）与增删改按钮；
//! - 核心优势：使本地主视图更加清晰，避免数百行工具栏布局逻辑杂糅在主面板结构中。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::separator::Separator;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;

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
}

/// 渲染本地文件面板顶部的双行工具栏与快速路径跳转。
pub(super) fn render_local_toolbar(
    props: LocalToolbarProps<'_>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let tab_id = props.tab_id;
    let show_filter = props.show_filter;
    let show_hidden = props.show_hidden;
    let local_path_input = props.local_path_input;
    let local_filter_input = props.local_filter_input;
    let quick_paths = props.quick_paths;
    let remote_available = props.remote_available;

    v_flex()
        .p_2()
        .gap_1()
        .border_b_1()
        .border_color(p.sidebar_border)
        .child(
            h_flex()
                .gap_1()
                .items_center()
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
                    Button::new("local-up")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowUp)
                        .tooltip("返回上一级")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sftp_local_go_up(tab_id, window, cx)
                        })),
                )
                .child(
                    Button::new("local-refresh")
                        .ghost()
                        .xsmall()
                        .icon(IconName::RefreshCw)
                        .tooltip("刷新本地目录")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.sftp_local_refresh(tab_id, cx)),
                        ),
                )
                .child(
                    Button::new("local-enter")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowRight)
                        .tooltip("进入路径")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sftp_local_enter_path(tab_id, window, cx)
                        })),
                )
                .when(remote_available, |this| {
                    this.child(
                        Button::new("local-collapse")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelLeftClose)
                            .tooltip("收起本地文件列")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.sftp_toggle_local_collapse(tab_id, cx);
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
                    IconName::House,
                    rust_i18n::t!("sftp.home").to_string(),
                    quick_paths.home.clone(),
                    tab_id,
                    cx,
                ))
                .child(local_quick_path_button(
                    "local-quick-desktop",
                    IconName::Monitor,
                    rust_i18n::t!("sftp.desktop").to_string(),
                    quick_paths.desktop.clone(),
                    tab_id,
                    cx,
                ))
                .child(local_quick_path_button(
                    "local-quick-downloads",
                    IconName::FolderDown,
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
                        Button::new("local-action-upload")
                            .primary()
                            .xsmall()
                            .icon(IconName::Upload)
                            .tooltip("上传")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.sftp_queue_upload(tab_id, cx);
                            })),
                    )
                })
                .child(
                    Button::new("local-action-mkdir")
                        .ghost()
                        .xsmall()
                        .icon(IconName::FolderPlus)
                        .tooltip("新建本地文件夹")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sftp_open_mkdir_modal(tab_id, false, window, cx);
                        })),
                )
                .child(
                    Button::new("local-action-new-file")
                        .ghost()
                        .xsmall()
                        .icon(IconName::FilePlus)
                        .tooltip("新建本地文件")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sftp_open_local_new_file_modal(tab_id, window, cx);
                        })),
                )
                .child(
                    Button::new("local-action-rename")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Pencil)
                        .tooltip("重命名")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sftp_open_rename_modal(tab_id, false, window, cx);
                        })),
                )
                .child(
                    Button::new("local-action-edit")
                        .ghost()
                        .xsmall()
                        .icon(IconName::FilePen)
                        .tooltip("编辑")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let selected = this
                                .workspace
                                .tab(tab_id)
                                .and_then(|t| t.sftp())
                                .and_then(|p| p.local.selected.clone());
                            if let Some(target) = selected {
                                this.sftp_edit_local_file(tab_id, target, cx);
                            } else if let Some(panel) =
                                this.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
                            {
                                panel.set_local_error("请先选择一个本地文件进行编辑".to_string());
                                cx.notify();
                            }
                        })),
                )
                .child(
                    Button::new("local-action-properties")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Info)
                        .tooltip("查看属性")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let path =
                                this.workspace
                                    .tab(tab_id)
                                    .and_then(|t| t.sftp())
                                    .and_then(|p| {
                                        p.local
                                            .selected
                                            .clone()
                                            .or_else(|| Some(p.local.path.clone()))
                                    });
                            if let Some(target) = path {
                                this.sftp_show_local_properties(tab_id, target, cx);
                            }
                        })),
                )
                .child(
                    Button::new("local-action-delete")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .tooltip("删除")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_local_delete_selection(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("local-action-open-fm")
                        .ghost()
                        .xsmall()
                        .icon(IconName::FolderOpen)
                        .tooltip("打开文件管理器")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_open_local_file_manager(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("local-toggle-hidden")
                        .ghost()
                        .xsmall()
                        .icon(if show_hidden {
                            IconName::Eye
                        } else {
                            IconName::EyeOff
                        })
                        .selected(show_hidden)
                        .tooltip(if show_hidden {
                            "隐藏点号文件 (Ctrl+H)"
                        } else {
                            "显示隐藏文件 (Ctrl+H)"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_toggle_hidden(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("local-toggle-filter")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Search)
                        .selected(show_filter)
                        .tooltip("目录内模糊过滤 (Ctrl+F)")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.sftp_toggle_filter(tab_id, window, cx);
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
    icon: IconName,
    label: String,
    target: Option<PathBuf>,
    tab_id: TabId,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let enabled = target.is_some();
    let tooltip = if enabled {
        label
    } else {
        format!("{label} ({})", rust_i18n::t!("sftp.missing_dir"))
    };
    let mut button = Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(!enabled);
    if let Some(target) = target {
        button = button.on_click(cx.listener(move |this, _, window, cx| {
            this.sftp_local_navigate(tab_id, target.clone(), window, cx);
        }));
    }
    button.into_any_element()
}
