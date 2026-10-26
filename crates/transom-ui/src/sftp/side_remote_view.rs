//! SFTP 并列远端视图组件、挂载浮层与折叠栏渲染实现。
//!
//! 设计原理（Why）：
//! - 实现初衷：为多主机并排工作流提供视觉容器与交互载体，将本地列、主远端列与多个并列远端列
//!   无缝整合至多栏响应式布局中；
//! - 核心优势：
//!   1. 各并列远端具备独立完整的虚拟列表、路径导航与操作栏；
//!   2. 支持基于 GPUI 原生类型系统的跨列拖拽（Drag & Drop）与放置处理；
//!   3. 挂载弹窗自动聚合已有 SSH 会话，免去重复配置与多次鉴权认证。

use super::side_remote::{
    SftpActivePane, SftpClipboardData, SftpClipboardSource, SftpDragItem, SftpDragPreview,
    SftpRemoteCopyParams,
};
use super::{format_size, fuzzy_match_name, sftp_file_icon};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, v_virtual_list};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{ConnectionId, RemoteFile, TabId, format_octal_permissions};

impl AppShell {
    /// 渲染挂载并列远端按钮及其 Popover 菜单。
    pub(crate) fn render_side_remote_mount_button(
        &self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_ref())
            .map(|p| p.mount_menu_open)
            .unwrap_or(false);

        let app_open = cx.entity().clone();
        let trigger = Button::new("sftp-mount-side-remote-btn")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .tooltip("并列挂载其他已连接会话")
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                    panel.mount_menu_open = !panel.mount_menu_open;
                    cx.notify();
                }
            }));

        // 收集已建立连接的其他标签页
        let active_candidates: Vec<(ConnectionId, String, String)> = self
            .tabs
            .iter()
            .filter(|t| t.id != tab_id && t.connection_id.is_some())
            .filter_map(|t| {
                let conn_id = t.connection_id?;
                let title = t.title.clone();
                let initial_path = t
                    .sftp
                    .as_ref()
                    .map(|p| p.path.clone())
                    .unwrap_or_else(|| "/".to_string());
                Some((conn_id, title, initial_path))
            })
            .collect();

        let app_menu = cx.entity().clone();
        let palette = p.clone();

        Popover::new(ElementId::Name(
            format!("sftp-mount-popover-{tab_id}").into(),
        ))
        .anchor(Anchor::TopRight)
        .open(open)
        .on_open_change(move |is_open, _window, cx| {
            let is_open = *is_open;
            app_open.update(cx, |this, cx| {
                if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                    panel.mount_menu_open = is_open;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_window, _cx, _area| {
            let mut menu = v_flex()
                .min_w(px(240.0))
                .max_h(px(320.0))
                .overflow_hidden()
                .p_1()
                .gap_1()
                .bg(palette.popover)
                .text_color(palette.popover_foreground)
                .rounded(palette.radius.md)
                .border_1()
                .border_color(palette.border);

            if active_candidates.is_empty() {
                return menu
                    .p_3()
                    .items_center()
                    .text_xs()
                    .text_color(palette.muted_foreground)
                    .child("暂无其他已连接的远端会话")
                    .into_any_element();
            }

            menu = menu.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.muted_foreground)
                    .child("已连接的会话 (点击挂载)"),
            );
            for (conn_id, title, initial_path) in active_candidates.clone() {
                let app = app_menu.clone();
                let title_str = title.clone();
                let path_str = initial_path.clone();
                menu = menu.child(
                    h_flex()
                        .px_2()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .rounded(palette.radius.sm)
                        .cursor_pointer()
                        .hover(|style| {
                            style
                                .bg(palette.sidebar_accent)
                                .text_color(palette.sidebar_accent_foreground)
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.sftp_mount_side_remote(
                                    tab_id,
                                    conn_id,
                                    title_str.clone(),
                                    path_str.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .child(Icon::new(IconName::Server).small())
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .truncate()
                                .child(title),
                        ),
                );
            }

            menu.into_any_element()
        })
        .into_any_element()
    }

    /// 渲染本地文件列折叠后的 32px 竖条。
    pub(crate) fn render_sftp_local_collapsed_bar(
        &self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expand_btn = Button::new("expand-sftp-local-btn")
            .ghost()
            .xsmall()
            .icon(IconName::PanelLeftOpen)
            .tooltip("展开本地文件列")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_local_collapse(tab_id, cx);
            }));

        v_flex()
            .w(px(32.0))
            .h_full()
            .items_center()
            .py_2()
            .gap_2()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.sidebar_border)
            .child(expand_btn)
            .child(
                Icon::new(IconName::Laptop)
                    .small()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted_foreground)
                    .child("本地"),
            )
            .into_any_element()
    }

    /// 渲染单个并列远端列（包括展开态与折叠态）。
    pub(crate) fn render_sftp_side_remote_pane(
        &self,
        tab_id: TabId,
        side_index: usize,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return div().into_any_element();
        };
        let Some(panel) = tab.sftp.as_ref() else {
            return div().into_any_element();
        };
        let Some(side) = panel.side_remotes.get(side_index) else {
            return div().into_any_element();
        };

        if side.collapsed {
            // 渲染折叠竖条
            let expand_btn = Button::new(format!("expand-side-{side_index}"))
                .ghost()
                .xsmall()
                .icon(IconName::PanelRightOpen)
                .tooltip("展开远端列")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sftp_toggle_side_remote_collapse(tab_id, side_index, cx);
                }));

            let close_btn = Button::new(format!("close-side-{side_index}"))
                .ghost()
                .xsmall()
                .icon(IconName::X)
                .tooltip("关闭并列远端")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sftp_close_side_remote(tab_id, side_index, cx);
                }));

            return v_flex()
                .w(px(32.0))
                .h_full()
                .items_center()
                .py_2()
                .gap_2()
                .bg(p.sidebar)
                .border_r_1()
                .border_color(p.sidebar_border)
                .child(expand_btn)
                .child(close_btn)
                .child(
                    Icon::new(IconName::Server)
                        .small()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted_foreground)
                        .child(side.title.chars().take(2).collect::<String>()),
                )
                .into_any_element();
        }

        let connection_id = side.connection_id;
        let title = side.title.clone();
        let path = side.path.clone();
        let path_input = side.path_input.clone();
        let filter_input = side.filter_input.clone();
        let filter_query = filter_input.read(cx).value().trim().to_ascii_lowercase();
        let scroll_handle = side.scroll_handle.clone();
        let loading = side.loading;

        let filtered_entries: Vec<RemoteFile> = side
            .entries
            .iter()
            .filter(|e| filter_query.is_empty() || fuzzy_match_name(&e.name, &filter_query))
            .cloned()
            .collect();

        // 头部导航条
        let collapse_btn = Button::new(format!("collapse-side-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::ChevronLeft)
            .tooltip("收起此列")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_toggle_side_remote_collapse(tab_id, side_index, cx);
            }));

        let close_btn = Button::new(format!("close-side-expanded-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip("关闭并列远端")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sftp_close_side_remote(tab_id, side_index, cx);
            }));

        let path_val = path.clone();
        let up_btn = Button::new(format!("side-up-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::ArrowUp)
            .tooltip("上一级")
            .on_click(cx.listener(move |this, _, _, cx| {
                let parent = if path_val == "/" {
                    "/".to_string()
                } else {
                    let trimmed = path_val.trim_end_matches('/');
                    match trimmed.rsplit_once('/') {
                        Some(("", _)) => "/".to_string(),
                        Some((p, _)) => p.to_string(),
                        None => "/".to_string(),
                    }
                };
                this.request_sftp_list_for_connection(tab_id, connection_id, parent, cx);
            }));

        let cur_path = path.clone();
        let refresh_btn = Button::new(format!("side-refresh-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .tooltip("刷新目录")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.request_sftp_list_for_connection(tab_id, connection_id, cur_path.clone(), cx);
            }));

        let enter_path_input = path_input.clone();
        let enter_btn = Button::new(format!("side-enter-{side_index}"))
            .ghost()
            .xsmall()
            .icon(IconName::ArrowRight)
            .tooltip("进入路径")
            .on_click(cx.listener(move |this, _, _, cx| {
                let p = enter_path_input.read(cx).value().trim().to_string();
                if !p.is_empty() {
                    this.request_sftp_list_for_connection(tab_id, connection_id, p, cx);
                }
            }));

        let header = v_flex()
            .border_b_1()
            .border_color(p.sidebar_border)
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
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.muted_foreground)
                            .child(title),
                    )
                    .child(div().flex_1().child(Input::new(&path_input).w_full()))
                    .child(up_btn)
                    .child(refresh_btn)
                    .child(enter_btn)
                    .child(collapse_btn)
                    .child(close_btn),
            );

        // 构建虚拟列表内容
        let app_entity = cx.entity().clone();
        let list_content = if filtered_entries.is_empty() && !loading {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child("目录为空")
                .into_any_element()
        } else {
            let row_sizes = Rc::new(vec![size(px(0.), px(28.)); filtered_entries.len()]);
            let entries_rc = Rc::new(filtered_entries);
            let p_item = p.clone();

            v_virtual_list(
                cx.entity().clone(),
                format!("sftp-side-remote-list-{tab_id}-{side_index}"),
                row_sizes,
                move |_this, range, _window, _cx| {
                    let app_ent = app_entity.clone();
                    let p_row = p_item.clone();
                    let entries = entries_rc.clone();

                    range
                        .filter_map(|ix| {
                            let entry = entries.get(ix)?;
                            let entry_path = entry.path.clone();
                            let entry_name = entry.name.clone();
                            let is_dir = entry.is_dir;
                            let app_click = app_ent.clone();
                            let app_cm = app_ent.clone();
                            let p_cm = entry_path.clone();

                            let row = h_flex()
                                .id(ElementId::Name(
                                    format!("side-file-{side_index}-{}", entry.path).into(),
                                ))
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .rounded(p_row.radius.sm)
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(p_row.sidebar_accent)
                                        .text_color(p_row.sidebar_accent_foreground)
                                })
                                .child(sftp_file_icon(is_dir, &p_row))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_sm()
                                        .overflow_hidden()
                                        .child(entry.name.clone()),
                                )
                                .when_some(entry.permissions, |this, perms| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .px_1()
                                            .rounded(p_row.radius.sm)
                                            .bg(p_row.muted)
                                            .text_color(p_row.muted_foreground)
                                            .child(format_octal_permissions(perms)),
                                    )
                                })
                                .child(div().text_xs().text_color(p_row.muted_foreground).child(
                                    if is_dir {
                                        String::new()
                                    } else {
                                        format_size(entry.size)
                                    },
                                ));

                            // 双击与单击处理
                            let path_double = entry_path.clone();
                            let row = row.on_mouse_down(
                                MouseButton::Left,
                                move |event: &MouseDownEvent, _window, cx| {
                                    let path_d = path_double.clone();
                                    app_click.update(cx, |this, cx| {
                                        if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                                            panel.active_pane =
                                                SftpActivePane::SideRemote(side_index);
                                            if let Some(side_state) =
                                                panel.side_remotes.get_mut(side_index)
                                            {
                                                if event.click_count >= 2 {
                                                    if is_dir {
                                                        this.request_sftp_list_for_connection(
                                                            tab_id,
                                                            connection_id,
                                                            path_d,
                                                            cx,
                                                        );
                                                    }
                                                    return;
                                                }
                                                side_state.selected = Some(path_d.clone());
                                                side_state.selection.clear();
                                                side_state.selection.insert(path_d);
                                            }
                                            cx.notify();
                                        }
                                    });
                                },
                            );

                            // 拖拽源配置
                            let drag_payload = SftpDragItem {
                                source_tab_id: tab_id,
                                source_connection_id: Some(connection_id),
                                paths: vec![entry_path.clone()],
                                file_name: entry_name.clone(),
                                is_dir,
                            };
                            let preview_name = entry_name.clone();
                            let row = row.on_drag(drag_payload, move |_, _, _, cx| {
                                cx.new(|_| SftpDragPreview::new(preview_name.clone(), 1))
                            });

                            // 右键上下文菜单
                            let p_copy = p_cm.clone();
                            let p_transfer = p_cm.clone();
                            let p_name = entry_name.clone();
                            let row = row.context_menu(move |menu, _window, _cx| {
                                let app = app_cm.clone();
                                let path_cp = p_copy.clone();
                                let path_tr = p_transfer.clone();
                                let name_tr = p_name.clone();

                                menu.item(
                                    PopupMenuItem::new("下载到本地")
                                        .icon(IconName::Download)
                                        .on_click({
                                            let app = app.clone();
                                            let path_tr = path_tr.clone();
                                            let name_tr = name_tr.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) =
                                                        this.sftp_panel_mut_tab(tab_id)
                                                    {
                                                        let local_target =
                                                            panel.local_path.join(&name_tr);
                                                        this.sftp_enqueue_download_path(
                                                            tab_id,
                                                            path_tr.clone(),
                                                            local_target,
                                                            cx,
                                                        );
                                                    }
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("流式复制到主远端")
                                        .icon(IconName::ArrowLeftRight)
                                        .on_click({
                                            let app = app.clone();
                                            let path_tr = path_tr.clone();
                                            let name_tr = name_tr.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    let main_conn = this
                                                        .tabs
                                                        .iter()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.connection_id);
                                                    let main_path = this
                                                        .sftp_panel_mut_tab(tab_id)
                                                        .map(|p| p.path.clone());
                                                    if let (Some(target_conn), Some(target_dir)) =
                                                        (main_conn, main_path)
                                                    {
                                                        let target_file = format!(
                                                            "{}/{}",
                                                            target_dir.trim_end_matches('/'),
                                                            name_tr
                                                        );
                                                        this.sftp_enqueue_remote_transfer(
                                                            tab_id,
                                                            SftpRemoteCopyParams {
                                                                source_connection_id: connection_id,
                                                                source_path: path_tr.clone(),
                                                                target_connection_id: target_conn,
                                                                target_path: target_file,
                                                                is_dir,
                                                            },
                                                            cx,
                                                        );
                                                    }
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("复制条目 (Ctrl+C)")
                                        .icon(IconName::Copy)
                                        .on_click({
                                            let app = app.clone();
                                            let path_tr = path_tr.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) =
                                                        this.sftp_panel_mut_tab(tab_id)
                                                    {
                                                        panel.clipboard = Some(SftpClipboardData {
                                                            source: SftpClipboardSource::Remote(
                                                                connection_id,
                                                            ),
                                                            paths: vec![path_tr.clone()],
                                                        });
                                                        panel.status =
                                                            Some(format!("已复制条目: {path_tr}"));
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("复制绝对路径")
                                        .icon(IconName::Copy)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    path_cp.clone(),
                                                ));
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) =
                                                        this.sftp_panel_mut_tab(tab_id)
                                                    {
                                                        panel.status = Some(format!(
                                                            "已复制远端路径: {path_cp}"
                                                        ));
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        }),
                                )
                            });

                            Some(row.into_any_element())
                        })
                        .collect()
                },
            )
            .into_any_element()
        };

        // 放置目标容器支持跨端拖拽放置
        let app_drop = cx.entity().clone();
        let target_dir = path.clone();
        let drop_bg = p.sidebar_accent.opacity(0.3);
        let drop_border = p.primary;
        let drop_target = v_flex()
            .relative()
            .flex_1()
            .overflow_hidden()
            .p_2()
            .drag_over::<SftpDragItem>(move |style, _, _, _| {
                style.bg(drop_bg).border_2().border_color(drop_border)
            })
            .on_drop(move |drag: &SftpDragItem, _window, cx| {
                let item = drag.clone();
                let dir = target_dir.clone();
                app_drop.update(cx, |this, cx| {
                    if item.source_connection_id == Some(connection_id) {
                        return; // 同端内部暂不触发
                    }
                    if let Some(src_conn) = item.source_connection_id {
                        // 跨远端互传
                        for path in &item.paths {
                            let fname = path.rsplit('/').next().unwrap_or(path);
                            let target_path = format!("{}/{}", dir.trim_end_matches('/'), fname);
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
                        // 来自本地上传
                        for path_str in &item.paths {
                            let path_buf = std::path::PathBuf::from(path_str);
                            let fname = path_buf
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "upload".into());
                            let target_path = format!("{}/{}", dir.trim_end_matches('/'), fname);
                            this.sftp_enqueue_upload_path(tab_id, path_buf, target_path, cx);
                        }
                    }
                });
            })
            .child(list_content)
            .vertical_scrollbar(&scroll_handle);

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .border_r_1()
            .border_color(p.sidebar_border)
            .child(header)
            .child(drop_target)
            .into_any_element()
    }
}
