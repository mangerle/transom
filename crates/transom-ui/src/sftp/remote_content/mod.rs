//! SFTP 远程内容区：空态提示与虚拟列表行（选择、双击、右键与拖拽）。
//!
//! 设计原理（Why）：行渲染与交互从主面板抽出后，主函数只负责布局编排；
//! 上下文菜单与行操作分别下沉至子模块，参数以对象模式显式收敛。

mod context_menu;

use self::context_menu::{RemoteContextMenuProps, build_remote_context_menu};
use super::file_row::{FileRowProps, file_row};
use super::preview::{PreviewKey, SftpHoverTarget};
use super::side_remote::{SftpDragItem, SftpDragPreview};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteFile, TabId};

/// 远程内容区的渲染输入：条目、加载态、标签归属与滚动句柄。
pub(crate) struct RemoteContentProps {
    pub(crate) remote_entries: Vec<RemoteFile>,
    pub(crate) loading: bool,
    pub(crate) tab_id: TabId,
    pub(crate) tab_conn_id: Option<ConnectionId>,
    pub(crate) remote_scroll_handle: VirtualListScrollHandle,
}

impl AppShell {
    /// 远程文件内容区：空态提示或虚拟列表（行内承载选择、双击、右键与拖拽）。
    pub(crate) fn render_remote_content(
        &self,
        props: RemoteContentProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteContentProps {
            remote_entries,
            loading,
            tab_id,
            tab_conn_id,
            remote_scroll_handle,
        } = props;
        if remote_entries.is_empty() && !loading {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.empty").to_string())
                .into_any_element()
        } else {
            let remote_sizes = std::rc::Rc::new(vec![size(px(0.), px(28.)); remote_entries.len()]);
            let remote_entries_rc = std::rc::Rc::new(remote_entries);
            let p = p.clone();
            let app_entity = cx.entity().clone();
            v_virtual_list(
                cx.entity().clone(),
                format!("sftp-remote-virtual-list-{tab_id}"),
                remote_sizes,
                move |this, range, _window, cx| {
                    let (remote_selection, remote_primary) = this
                        .workspace
                        .tab(tab_id)
                        .and_then(|tab| tab.sftp())
                        .map(|panel| {
                            (
                                panel.remote.selection.clone(),
                                panel.remote.selected.clone(),
                            )
                        })
                        .unwrap_or_default();
                    let app_entity = app_entity.clone();
                    range
                        .filter_map(|ix| {
                            let entry = remote_entries_rc.get(ix)?;
                            let entry_path = entry.path.clone();
                            let entry_name = entry.name.clone();
                            let entry_is_dir = entry.is_dir;
                            let is_parent_entry = entry_name == "..";
                            let is_selected = remote_selection.contains(&entry.path)
                                || remote_primary.as_ref() == Some(&entry.path);
                            let row = file_row(FileRowProps {
                                id: format!("remote-file-{}", entry.path),
                                name: &entry.name,
                                is_dir: entry_is_dir,
                                is_parent: is_parent_entry,
                                size: entry.size,
                                permissions: entry.permissions,
                                selected: is_selected,
                                palette: &p,
                            });
                            let entry_path_cm = entry.path.clone();
                            let entry_path_right = entry.path.clone();
                            let app_cm = app_entity.clone();
                            let click_entries = remote_entries_rc.clone();
                            let hover_target = SftpHoverTarget {
                                tab_id,
                                key: PreviewKey {
                                    is_remote: true,
                                    path: entry.path.clone(),
                                },
                                name: entry.name.clone(),
                                is_dir: entry_is_dir,
                                size: entry.size,
                            };
                            let row = row
                                .on_hover(cx.listener(move |this, hovered, window, cx| {
                                    this.sftp_hover_preview(
                                        hover_target.clone(),
                                        *hovered,
                                        window,
                                        cx,
                                    );
                                }))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        this.dismiss_sftp_hover_preview(tab_id, cx);
                                        if let Some(panel) =
                                            this.workspace.tab(tab_id).and_then(|tab| tab.sftp())
                                        {
                                            window.focus(&panel.focus_handle, cx);
                                        }
                                        if event.click_count >= 2 {
                                            if is_parent_entry {
                                                this.sftp_go_up(tab_id, window, cx);
                                            } else if entry_is_dir {
                                                this.sftp_navigate(
                                                    tab_id,
                                                    entry_path.clone(),
                                                    window,
                                                    cx,
                                                );
                                            } else {
                                                this.sftp_edit_remote_file(
                                                    tab_id,
                                                    entry_path.clone(),
                                                    window,
                                                    cx,
                                                );
                                            }
                                            return;
                                        }
                                        if is_parent_entry {
                                            return;
                                        }
                                        let Some(panel) = this
                                            .workspace
                                            .tab_mut(tab_id)
                                            .and_then(|tab| tab.sftp_mut())
                                        else {
                                            return;
                                        };
                                        if event.modifiers.shift {
                                            let anchor_index =
                                                panel.remote.anchor.as_ref().and_then(|anchor| {
                                                    click_entries
                                                        .iter()
                                                        .position(|entry| &entry.path == anchor)
                                                });
                                            if let Some(anchor_index) = anchor_index {
                                                let (start, end) = if anchor_index <= ix {
                                                    (anchor_index, ix)
                                                } else {
                                                    (ix, anchor_index)
                                                };
                                                let paths = click_entries[start..=end]
                                                    .iter()
                                                    .map(|entry| entry.path.clone())
                                                    .collect::<Vec<_>>();
                                                panel
                                                    .select_remote_range(paths, entry_path.clone());
                                            } else {
                                                panel.select_remote(entry_path.clone());
                                            }
                                        } else if event.modifiers.control
                                            || event.modifiers.platform
                                        {
                                            panel.toggle_remote_selection(entry_path.clone());
                                        } else {
                                            panel.select_remote(entry_path.clone());
                                        }
                                        let selected_count = panel.selected_remote_paths().len();
                                        if selected_count > 1 {
                                            panel.queue.status =
                                                Some(format!("已选择 {selected_count} 个远端项目"));
                                        }
                                        let name = entry_name.clone();
                                        panel.remote.name_input.update(cx, |state, cx| {
                                            state.set_value(name, window, cx);
                                        });
                                        cx.notify();
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, _, window, cx| {
                                        this.dismiss_sftp_hover_preview(tab_id, cx);
                                        if let Some(panel) = this
                                            .workspace
                                            .tab_mut(tab_id)
                                            .and_then(|tab| tab.sftp_mut())
                                        {
                                            if panel.remote.selection.contains(&entry_path_right) {
                                                panel.set_remote_primary(entry_path_right.clone());
                                            } else {
                                                panel.select_remote(entry_path_right.clone());
                                            }
                                            window.focus(&panel.focus_handle, cx);
                                        }
                                        cx.notify();
                                    }),
                                );

                            let row = if is_parent_entry {
                                row
                            } else {
                                let drag_payload = SftpDragItem {
                                    source_tab_id: tab_id,
                                    source_connection_id: tab_conn_id,
                                    paths: vec![entry.path.clone()],
                                    file_name: entry.name.clone(),
                                    is_dir: entry_is_dir,
                                };
                                let preview_name = entry.name.clone();
                                row.on_drag(drag_payload, move |_, _, _, cx| {
                                    cx.new(|_| SftpDragPreview::new(preview_name.clone(), 1))
                                })
                            };

                            let row = row.context_menu(move |menu, _window, _cx| {
                                build_remote_context_menu(
                                    menu,
                                    RemoteContextMenuProps {
                                        app: app_cm.clone(),
                                        tab_id,
                                        entry_path: entry_path_cm.clone(),
                                        is_dir: entry_is_dir,
                                        is_parent_entry,
                                    },
                                )
                            });
                            Some(row.into_any_element())
                        })
                        .collect()
                },
            )
            .track_scroll(&remote_scroll_handle)
            .into_any_element()
        }
    }
}
