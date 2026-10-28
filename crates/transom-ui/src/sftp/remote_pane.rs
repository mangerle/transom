//! SFTP 远程列与内容区的渲染：列工具栏、虚拟列表行与交互。
//!
//! 设计原理（Why）：两段渲染合计八百余行，留在主面板渲染函数里会让
//! "布局编排"与"行内交互"混为一体；拆到独立文件后，主函数只剩列的
//! 组合，行渲染的输入以结构体显式传递。

use super::side_remote::{SftpDragItem, SftpRemoteCopyParams};
use crate::app::AppShell;
use crate::editor::render_editor_list_popover;
use crate::palette::Palette;
use crate::sftp::SftpTransferRequest;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{Sizable, VirtualListScrollHandle};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{TabId, TransferDirection};

impl AppShell {
    /// 远程列：标题栏（路径输入与操作按钮）、内容区与过滤栏。
    pub(super) fn render_remote_pane(
        &self,
        props: RemotePaneProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemotePaneProps {
            tab_id,
            remote_current_dir,
            remote_path_input,
            remote_filter_input,
            show_filter,
            show_hidden,
            sync_navigation,
            remote_content,
            remote_scroll_handle,
        } = props;
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(
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
                                    .child(rust_i18n::t!("sftp.remote_title").to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(Input::new(&remote_path_input).w_full()),
                            )
                            .child(
                                Button::new("sftp-up")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowUp)
                                    .tooltip("返回上一级")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.sftp_go_up(tab_id, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("sftp-refresh")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::RefreshCw)
                                    .tooltip("刷新远程目录")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_refresh(tab_id, cx)
                                    })),
                            )
                            .child(
                                Button::new("sftp-go")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowRight)
                                    .tooltip("进入路径")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_enter_path(tab_id, cx)
                                    })),
                            )
                            .child(
                                Button::new("sftp-collapse")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::PanelLeftClose)
                                    .tooltip("收起主远端列")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_toggle_remote_collapse(tab_id, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("sftp-home")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::House)
                                    .tooltip(rust_i18n::t!("sftp.home").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.sftp_remote_go_home(tab_id, window, cx);
                                    })),
                            )
                            .child(
                                Separator::vertical()
                                    .h(px(14.0))
                                    .mx_1()
                                    .color(p.sidebar_border),
                            )
                            .child(
                                Button::new("sftp-action-download")
                                    .primary()
                                    .xsmall()
                                    .icon(IconName::Download)
                                    .tooltip(rust_i18n::t!("sftp.download").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_queue_download(tab_id, cx);
                                    })),
                            )
                            .child(
                                Button::new("sftp-mkdir")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::FolderPlus)
                                    .tooltip(rust_i18n::t!("sftp.new_folder").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.sftp_open_mkdir_modal(tab_id, true, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("sftp-new-file")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::FilePlus)
                                    .tooltip("新建文件并编辑")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.sftp_open_editor_new_file_modal(tab_id, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("sftp-rename")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Pencil)
                                    .tooltip(rust_i18n::t!("sftp.rename").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.sftp_open_rename_modal(tab_id, true, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("sftp-edit")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::FilePen)
                                    .tooltip("编辑")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let selected = this
                                            .workspace
                                            .tab(tab_id)
                                            .and_then(|t| t.sftp())
                                            .and_then(|p| p.remote.selected.clone());
                                        if let Some(remote_path) = selected {
                                            this.sftp_edit_remote_file(
                                                tab_id,
                                                remote_path,
                                                window,
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(render_editor_list_popover(self, tab_id, p, cx))
                            .child(
                                Button::new("sftp-chmod")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Settings)
                                    .tooltip("修改权限 (Chmod)")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.sftp_open_chmod_modal(tab_id, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("sftp-delete")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Trash)
                                    .tooltip(rust_i18n::t!("sftp.delete").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_delete_selection(tab_id, cx);
                                    })),
                            )
                            .child(
                                Button::new("remote-toggle-hidden")
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
                                Button::new("toggle-sftp-sync-nav")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowLeftRight)
                                    .selected(sync_navigation)
                                    .tooltip(if sync_navigation {
                                        "同步导航（已开启，点击关闭）"
                                    } else {
                                        "同步导航（已关闭，点击开启）"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(panel) = this
                                            .workspace
                                            .tab_mut(tab_id)
                                            .and_then(|t| t.sftp_mut())
                                        {
                                            panel.sync_navigation = !panel.sync_navigation;
                                            panel.set_status(
                                                if panel.sync_navigation {
                                                    "已开启双栏同步导航联动"
                                                } else {
                                                    "已关闭双栏同步导航联动"
                                                }
                                                .to_string(),
                                            );
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Button::new("sftp-compare-sync")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::GitCompareArrows)
                                    .tooltip("目录对比与增量同步")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_open_sync_modal(tab_id, cx);
                                    })),
                            )
                            .child(
                                Button::new("toggle-sftp-filter")
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
                        this.child(Input::new(&remote_filter_input).w_full())
                    }),
            )
            .child({
                let app_drop = cx.entity().clone();
                let dir_drop = remote_current_dir.clone();
                let drop_bg = p.sidebar_accent.opacity(0.3);
                let drop_border = p.primary;
                v_flex()
                    .id(ElementId::Name("sftp-remote-list".into()))
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .p_2()
                    .drag_over::<SftpDragItem>(move |style, _, _, _| {
                        style.bg(drop_bg).border_2().border_color(drop_border)
                    })
                    .on_drop(move |drag: &SftpDragItem, _window, cx| {
                        let item = drag.clone();
                        let dir = dir_drop.clone();
                        app_drop.update(cx, |this, cx| {
                            let Some(tab) = this.workspace.tab(tab_id) else {
                                return;
                            };
                            let Some(main_conn) = tab.connection_id else {
                                return;
                            };
                            if item.source_connection_id == Some(main_conn) {
                                return;
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
                                            target_connection_id: main_conn,
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
                                            connection_id: main_conn,
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
                    .child(remote_content)
                    .vertical_scrollbar(&remote_scroll_handle)
            })
            .into_any_element()
    }
}

/// 远程列的渲染输入：字段都是主渲染函数算好的展示态。
pub(super) struct RemotePaneProps {
    pub(super) tab_id: TabId,
    pub(super) remote_current_dir: String,
    pub(super) remote_path_input: Entity<InputState>,
    pub(super) remote_filter_input: Entity<InputState>,
    pub(super) show_filter: bool,
    pub(super) show_hidden: bool,
    pub(super) sync_navigation: bool,
    pub(super) remote_content: AnyElement,
    pub(super) remote_scroll_handle: VirtualListScrollHandle,
}
