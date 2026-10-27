//! SFTP 并列远端视图组件与挂载操作渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为多主机并排工作流提供视觉容器与交互载体，将本地列、主远端列与多个并列远端列
//!   无缝整合至多栏响应式布局中；
//! - 核心优势：
//!   1. 各并列远端具备独立完整的虚拟列表、路径导航与操作栏；
//!   2. 支持基于 GPUI 原生类型系统的跨列拖拽（Drag & Drop）与放置处理；
//!   3. 挂载弹窗自动聚合已有 SSH 会话，免去重复配置与多次鉴权认证。

mod bars;
mod context_menu;
mod header;

use self::context_menu::build_side_remote_context_menu;
use self::header::render_side_remote_header;
use super::side_remote::{SftpActivePane, SftpDragItem, SftpDragPreview, SftpRemoteCopyParams};
use super::{VfsRowRenderOptions, fuzzy_match_name, render_vfs_row_content};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::v_virtual_list;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{RemoteFile, TabId};

impl AppShell {
    /// 渲染单个并列远端列（展开态）。
    pub(crate) fn render_sftp_side_remote_pane(
        &self,
        tab_id: TabId,
        side_index: usize,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.workspace.tabs.iter().find(|t| t.id == tab_id) else {
            return div().into_any_element();
        };
        let Some(panel) = tab.sftp() else {
            return div().into_any_element();
        };
        let Some(side) = panel.side_remotes.get(side_index) else {
            return div().into_any_element();
        };

        let connection_id = side.connection_id;
        let title = side.title.clone();
        let path = side.path.clone();
        let path_input = side.path_input.clone();
        let filter_input = side.filter_input.clone();
        let filter_query = filter_input.read(cx).value().trim().to_ascii_lowercase();
        let scroll_handle = side.scroll_handle.clone();
        let loading = side.loading;

        let mut filtered_entries: Vec<RemoteFile> = side
            .entries
            .iter()
            .filter(|e| filter_query.is_empty() || fuzzy_match_name(&e.name, &filter_query))
            .cloned()
            .collect();

        if path != "/"
            && !path.is_empty()
            && (filter_query.is_empty() || fuzzy_match_name("..", &filter_query))
        {
            let parent = super::parent_remote(&path);
            filtered_entries.insert(
                0,
                RemoteFile {
                    name: "..".to_string(),
                    path: parent,
                    is_dir: true,
                    is_symlink: false,
                    size: 0,
                    modified: None,
                    permissions: None,
                },
            );
        }

        let header = render_side_remote_header(
            self::header::SideRemoteHeaderProps {
                tab_id,
                side_index,
                connection_id,
                title,
                path: path.clone(),
                path_input: &path_input,
            },
            p,
            cx,
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
            let current_dir = path.clone();

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
                            let is_parent_entry = entry_name == "..";
                            let app_click = app_ent.clone();
                            let app_cm = app_ent.clone();
                            let p_cm = entry_path.clone();
                            let dir_cur = current_dir.clone();

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
                                .child(render_vfs_row_content(VfsRowRenderOptions {
                                    name: &entry.name,
                                    is_dir,
                                    is_parent: is_parent_entry,
                                    size: entry.size,
                                    permissions: entry.permissions,
                                    palette: &p_row,
                                }));

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
                                                    if is_parent_entry || is_dir {
                                                        this.request_sftp_list_for_connection(
                                                            tab_id,
                                                            connection_id,
                                                            path_d,
                                                            cx,
                                                        );
                                                    }
                                                    return;
                                                }
                                                if !is_parent_entry {
                                                    side_state.selected = Some(path_d.clone());
                                                    side_state.selection.clear();
                                                    side_state.selection.insert(path_d);
                                                }
                                            }
                                            cx.notify();
                                        }
                                    });
                                },
                            );

                            // 拖拽源配置（非 .. 目录才可作为拖拽源）
                            let row = if is_parent_entry {
                                row
                            } else {
                                let drag_payload = SftpDragItem {
                                    source_tab_id: tab_id,
                                    source_connection_id: Some(connection_id),
                                    paths: vec![entry_path.clone()],
                                    file_name: entry_name.clone(),
                                    is_dir,
                                };
                                let preview_name = entry_name.clone();
                                row.on_drag(drag_payload, move |_, _, _, cx| {
                                    cx.new(|_| SftpDragPreview::new(preview_name.clone(), 1))
                                })
                            };

                            // 右键上下文菜单
                            let row = row.context_menu(move |menu, _window, _cx| {
                                build_side_remote_context_menu(
                                    menu,
                                    app_cm.clone(),
                                    self::context_menu::SideRemoteMenuContext {
                                        tab_id,
                                        connection_id,
                                        entry_path: p_cm.clone(),
                                        entry_name: entry_name.clone(),
                                        is_dir,
                                        is_parent_entry,
                                        current_dir: dir_cur.clone(),
                                    },
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
        let target_dir = path;
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
