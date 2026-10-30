//! 远端列的共用渲染：主远端与并列远端共用同一套头部、工具栏、列表与拖放。
//!
//! 设计原理（Why）：
//! - 实现初衷：主远端与并列远端的状态已经合并为 `SftpRemotePaneState`，
//!   渲染若仍各写一套，能力与视觉就会继续分叉；本模块只认"目标列"，
//!   列类型差异（标题、关闭按钮、折叠入口）由 `side_index` 表达；
//! - 核心优势：工具栏、右键菜单、拖放、加载态与过滤栏只有一份实现，
//!   新增列能力天然对两种列同时生效；
//! - 代价与局限：同步导航只属于"主远端 ↔ 本地"这一对列，因此只在主远端显示；
//!   隐藏文件与过滤开关按列独立，互不影响。

use super::SftpTransferRequest;
use super::remote_content::{RemoteContentProps, build_remote_blank_context_menu};
use super::side_remote::{SftpDragItem, SftpRemoteCopyParams};
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sftp::{SftpRemotePaneRef, SftpRemotePaneState};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteFile, TabId, TransferDirection};

/// 渲染一条远端列所需的展示态与目标身份。
pub(super) struct RemotePaneProps<'a> {
    pub(super) tab_id: TabId,
    pub(super) target: SftpRemotePaneRef,
    /// 并列列在面板中的序号；主远端为 `None`。
    pub(super) side_index: Option<usize>,
    pub(super) title: String,
    pub(super) pane: &'a SftpRemotePaneState,
    pub(super) connection_id: ConnectionId,
    pub(super) show_filter: bool,
    pub(super) show_hidden: bool,
    pub(super) sync_navigation: bool,
    /// 是否渲染“最近编辑器”浮层：它是标签级入口，同一帧只能出现一次。
    pub(super) show_editor_list: bool,
    pub(super) remote_entries: Vec<RemoteFile>,
}

/// 列元素 ID 前缀：主远端与并列列互不冲突。
pub(super) fn pane_key(target: SftpRemotePaneRef) -> String {
    match target {
        SftpRemotePaneRef::Main => "main".to_string(),
        SftpRemotePaneRef::Side(connection_id) => format!("side-{connection_id}"),
    }
}

impl AppShell {
    /// 远端列：标题/路径导航、操作工具栏、过滤栏与文件内容区。
    pub(super) fn render_remote_pane(
        &self,
        props: RemotePaneProps<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemotePaneProps {
            tab_id,
            target,
            side_index,
            title,
            pane,
            connection_id,
            show_filter,
            show_hidden,
            sync_navigation,
            show_editor_list,
            remote_entries,
        } = props;
        let key = pane_key(target);
        let path_input = pane.path_input.clone();
        let filter_input = pane.filter_input.clone();
        let loading = pane.loading;
        let scroll_handle = pane.scroll_handle.clone();

        let up_btn = Button::new(format!("remote-up-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::ArrowUp)
            .tooltip(rust_i18n::t!("sftp.remote.go_up").to_string())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.sftp_go_up_pane(tab_id, target, window, cx)
            }));

        let refresh_btn = Button::new(format!("remote-refresh-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .tooltip(rust_i18n::t!("sftp.remote.refresh").to_string())
            .on_click(
                cx.listener(move |this, _, _, cx| this.sftp_refresh_pane(tab_id, target, cx)),
            );

        let enter_btn = Button::new(format!("remote-enter-{key}"))
            .ghost()
            .xsmall()
            .icon(IconName::ArrowRight)
            .tooltip(rust_i18n::t!("sftp.remote.enter_path").to_string())
            .on_click(
                cx.listener(move |this, _, _, cx| this.sftp_enter_path_pane(tab_id, target, cx)),
            );

        let collapse_btn = Button::new(format!("remote-collapse-{key}"))
            .ghost()
            .xsmall()
            .icon(if side_index.is_some() {
                IconName::PanelRightClose
            } else {
                IconName::PanelLeftClose
            })
            .tooltip(if side_index.is_some() {
                rust_i18n::t!("sftp.sidebar.collapse_side").to_string()
            } else {
                rust_i18n::t!("sftp.remote.collapse").to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| match side_index {
                Some(index) => this.sftp_toggle_side_remote_collapse(tab_id, index, cx),
                None => this.sftp_toggle_remote_collapse(tab_id, cx),
            }));

        let close_btn = side_index.map(|index| {
            Button::new(format!("remote-close-{key}"))
                .ghost()
                .xsmall()
                .icon(IconName::X)
                .tooltip(rust_i18n::t!("sftp.sidebar.close_side_remote").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sftp_close_side_remote(tab_id, index, cx);
                }))
        });

        let header = v_flex()
            .border_color(p.divider(p.sidebar_border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .p_2()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(IconName::Server)
                            .small()
                            .text_color(p.muted_foreground),
                    )
                    .child(
                        div()
                            .max_w(px(120.0))
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .truncate()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.muted_foreground)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(Input::new(&path_input).w_full()),
                    )
                    .child(up_btn)
                    .child(refresh_btn)
                    .child(enter_btn)
                    .child(collapse_btn)
                    .when_some(close_btn, |this, button| this.child(button)),
            )
            .child(self.render_remote_toolbar(
                RemoteToolbarProps {
                    tab_id,
                    target,
                    pane,
                    show_filter,
                    show_hidden,
                    sync_navigation,
                    show_editor_list,
                },
                p,
                cx,
            ))
            .when(show_filter, |this| {
                this.child(Input::new(&filter_input).w_full())
            });

        let content = self.render_remote_content(
            RemoteContentProps {
                remote_entries,
                loading,
                error: pane.error.clone(),
                tab_id,
                target,
                connection_id,
                remote_scroll_handle: scroll_handle,
            },
            p,
            cx,
        );

        let app_drop = cx.entity().clone();
        let app_upload_drop = cx.entity().clone();
        let app_blank_menu = cx.entity().clone();
        let target_dir = pane.path.clone();
        let drop_bg = p.sidebar_accent.opacity(0.3);
        let drop_border = p.primary;
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(
                v_flex()
                    .id(ElementId::Name(format!("sftp-remote-list-{key}").into()))
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .p_2()
                    .drag_over::<SftpDragItem>(move |style, _, _, _| {
                        style.bg(drop_bg).border_2().border_color(drop_border)
                    })
                    .drag_over::<ExternalPaths>(move |style, _, _, _| {
                        style.bg(drop_bg).border_2().border_color(drop_border)
                    })
                    .on_drop(move |paths: &ExternalPaths, _window, cx| {
                        let dropped = paths.paths().to_vec();
                        if dropped.is_empty() {
                            return;
                        }
                        app_upload_drop.update(cx, move |this, cx| {
                            this.handle_dropped_files_to_pane(tab_id, &dropped, target, cx);
                        });
                    })
                    .on_drop(move |drag: &SftpDragItem, _window, cx| {
                        let item = drag.clone();
                        let dir = target_dir.clone();
                        app_drop.update(cx, |this, cx| {
                            if item.source_connection_id == Some(connection_id) {
                                return; // 同端内部不触发
                            }
                            if let Some(src_conn) = item.source_connection_id {
                                for path in &item.paths {
                                    let fname = path.rsplit('/').next().unwrap_or(path);
                                    let target_path =
                                        format!("{}/{}", dir.trim_end_matches('/'), fname);
                                    this.sftp_enqueue_remote_transfer(
                                        tab_id,
                                        SftpRemoteCopyParams {
                                            source_connection_id: src_conn,
                                            source_path: path.clone(),
                                            target_connection_id: connection_id,
                                            target_path,
                                            is_dir: item.is_dir,
                                        },
                                        cx,
                                    );
                                }
                            } else {
                                for path_str in &item.paths {
                                    let path_buf = std::path::PathBuf::from(path_str);
                                    let fname = path_buf
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "upload".into());
                                    let target_path =
                                        format!("{}/{}", dir.trim_end_matches('/'), fname);
                                    this.sftp_enqueue_transfer(
                                        SftpTransferRequest {
                                            tab_id,
                                            connection_id,
                                            direction: TransferDirection::Upload,
                                            is_dir: item.is_dir,
                                            local_path: path_buf,
                                            remote_path: target_path,
                                        },
                                        cx,
                                    );
                                }
                            }
                        });
                    })
                    .child(content)
                    .vertical_scrollbar(&pane.scroll_handle)
                    // 空白区右键给目录级动作：行元素上的菜单命中时优先并停止冒泡。
                    .context_menu(move |menu, _window, _cx| {
                        build_remote_blank_context_menu(
                            menu,
                            app_blank_menu.clone(),
                            tab_id,
                            target,
                        )
                    }),
            )
            .into_any_element()
    }
}

/// 工具栏渲染输入：目标列与列级开关状态。
pub(super) struct RemoteToolbarProps<'a> {
    pub(super) tab_id: TabId,
    pub(super) target: SftpRemotePaneRef,
    pub(super) pane: &'a SftpRemotePaneState,
    pub(super) show_filter: bool,
    pub(super) show_hidden: bool,
    pub(super) sync_navigation: bool,
    pub(super) show_editor_list: bool,
}
