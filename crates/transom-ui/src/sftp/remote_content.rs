//! SFTP 远程内容区：空态提示与虚拟列表行（选择、双击、右键与拖拽）。
//!
//! 设计原理（Why）：这段渲染合计约五百行，包含行内全部交互；独立成文件
//! 后主面板与远程列只负责布局编排，行渲染的输入（条目、加载态、标签归属
//! 与滚动句柄）以结构体显式传递。

use super::preview::{PreviewKey, SftpHoverTarget};
use super::side_remote::{SftpDragItem, SftpDragPreview};
use super::{format_size, sftp_file_icon};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteFile, TabId, format_octal_permissions};

impl AppShell {
    /// 远程文件内容区：空态提示或虚拟列表（行内承载选择、双击、右键与拖拽）。
    ///
    /// 设计原理（Why）：近五百行的行渲染与交互从主渲染函数抽出后，主函数
    /// 只保留布局编排；行渲染的输入收敛为参数，不再与整列其他状态耦合。
    pub(super) fn render_remote_content(
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
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(|tab| tab.sftp.as_ref())
                        .map(|panel| (panel.remote_selection.clone(), panel.selected.clone()))
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
                            let row = h_flex()
                                .id(ElementId::Name(
                                    format!("remote-file-{}", entry.path).into(),
                                ))
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .rounded(p.radius.sm)
                                .cursor_pointer();
                            let row = if is_selected {
                                row.bg(p.sidebar_accent)
                                    .text_color(p.sidebar_accent_foreground)
                            } else {
                                row.hover(|style| {
                                    style
                                        .bg(p.sidebar_accent)
                                        .text_color(p.sidebar_accent_foreground)
                                })
                            };
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
                                        if let Some(panel) = this
                                            .tabs
                                            .iter()
                                            .find(|tab| tab.id == tab_id)
                                            .and_then(|tab| tab.sftp.as_ref())
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
                                            .tabs
                                            .iter_mut()
                                            .find(|tab| tab.id == tab_id)
                                            .and_then(|tab| tab.sftp.as_mut())
                                        else {
                                            return;
                                        };
                                        if event.modifiers.shift {
                                            let anchor_index =
                                                panel.remote_anchor.as_ref().and_then(|anchor| {
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
                                            panel.status =
                                                Some(format!("已选择 {selected_count} 个远端项目"));
                                        }
                                        let name = entry_name.clone();
                                        panel.name_input.update(cx, |state, cx| {
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
                                            .tabs
                                            .iter_mut()
                                            .find(|tab| tab.id == tab_id)
                                            .and_then(|tab| tab.sftp.as_mut())
                                        {
                                            if panel.remote_selection.contains(&entry_path_right) {
                                                panel.set_remote_primary(entry_path_right.clone());
                                            } else {
                                                panel.select_remote(entry_path_right.clone());
                                            }
                                            window.focus(&panel.focus_handle, cx);
                                        }
                                        cx.notify();
                                    }),
                                )
                                .child(if is_parent_entry {
                                    Icon::new(IconName::Folder)
                                        .small()
                                        .text_color(p.muted_foreground)
                                        .into_any_element()
                                } else {
                                    sftp_file_icon(entry_is_dir, &p)
                                })
                                .child(div().flex_1().text_sm().overflow_hidden().child(
                                    if is_parent_entry {
                                        ".. (返回上一级)".to_string()
                                    } else {
                                        entry.name.clone()
                                    },
                                ))
                                .when_some(entry.permissions, |this, perms| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .px_1()
                                            .rounded(p.radius.sm)
                                            .bg(p.muted)
                                            .text_color(p.muted_foreground)
                                            .child(format_octal_permissions(perms)),
                                    )
                                })
                                .child(div().text_xs().text_color(p.muted_foreground).child(
                                    if entry_is_dir {
                                        String::new()
                                    } else {
                                        format_size(entry.size)
                                    },
                                ));

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
                                let app = app_cm.clone();
                                if is_parent_entry {
                                    return menu
                                        .item(
                                            PopupMenuItem::new("返回上一级")
                                                .icon(IconName::ArrowUp)
                                                .on_click({
                                                    let app = app.clone();
                                                    move |_, window, cx| {
                                                        app.update(cx, |this, cx| {
                                                            this.sftp_go_up(tab_id, window, cx);
                                                        });
                                                    }
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new("刷新")
                                                .icon(IconName::RefreshCw)
                                                .on_click({
                                                    let app = app.clone();
                                                    move |_, _, cx| {
                                                        app.update(cx, |this, cx| {
                                                            this.sftp_refresh(tab_id, cx);
                                                        });
                                                    }
                                                }),
                                        );
                                }
                                let p_dl = entry_path_cm.clone();
                                let p_edit = entry_path_cm.clone();
                                let p_cp = entry_path_cm.clone();
                                let p_sym = entry_path_cm.clone();
                                let p_prop = entry_path_cm.clone();
                                let p_chmod = entry_path_cm.clone();
                                let p_ren = entry_path_cm.clone();
                                let p_del = entry_path_cm.clone();
                                let mut m = menu.item(
                                    PopupMenuItem::new("下载")
                                        .icon(IconName::Download)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) = this
                                                        .tabs
                                                        .iter_mut()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.sftp.as_mut())
                                                    {
                                                        if panel.remote_selection.contains(&p_dl) {
                                                            panel.set_remote_primary(p_dl.clone());
                                                        } else {
                                                            panel.select_remote(p_dl.clone());
                                                        }
                                                    }
                                                    this.sftp_queue_download(tab_id, cx);
                                                });
                                            }
                                        }),
                                );
                                if !entry_is_dir {
                                    m = m.item(
                                        PopupMenuItem::new("内置编辑")
                                            .icon(IconName::FilePen)
                                            .on_click({
                                                let app = app.clone();
                                                move |_, window, cx| {
                                                    app.update(cx, |this, cx| {
                                                        this.sftp_edit_remote_file(
                                                            tab_id,
                                                            p_edit.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            }),
                                    );
                                }
                                m.item(
                                    PopupMenuItem::new("复制绝对路径")
                                        .icon(IconName::Copy)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    p_cp.clone(),
                                                ));
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) = this
                                                        .tabs
                                                        .iter_mut()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.sftp.as_mut())
                                                    {
                                                        panel.status =
                                                            Some(format!("已复制远端路径: {p_cp}"));
                                                    }
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("创建软链接")
                                        .icon(IconName::Link2)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) = this
                                                        .tabs
                                                        .iter_mut()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.sftp.as_mut())
                                                    {
                                                        panel.select_remote(p_sym.clone());
                                                    }
                                                    this.sftp_open_symlink_modal(
                                                        tab_id, true, window, cx,
                                                    );
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("查看属性")
                                        .icon(IconName::Info)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_request_remote_properties(
                                                        tab_id,
                                                        p_prop.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("修改权限 (Chmod)")
                                        .icon(IconName::Settings)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) = this
                                                        .tabs
                                                        .iter_mut()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.sftp.as_mut())
                                                    {
                                                        panel.select_remote(p_chmod.clone());
                                                    }
                                                    this.sftp_open_chmod_modal(tab_id, window, cx);
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("新建远端文件夹")
                                        .icon(IconName::FolderPlus)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_open_mkdir_modal(
                                                        tab_id, true, window, cx,
                                                    );
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("重命名")
                                        .icon(IconName::Pencil)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) = this
                                                        .tabs
                                                        .iter_mut()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.sftp.as_mut())
                                                    {
                                                        panel.select_remote(p_ren.clone());
                                                    }
                                                    this.sftp_open_rename_modal(
                                                        tab_id, true, window, cx,
                                                    );
                                                });
                                            }
                                        }),
                                )
                                .item(PopupMenuItem::new("删除").icon(IconName::Trash).on_click({
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            if let Some(panel) = this
                                                .tabs
                                                .iter_mut()
                                                .find(|t| t.id == tab_id)
                                                .and_then(|t| t.sftp.as_mut())
                                                && !panel.remote_selection.contains(&p_del)
                                            {
                                                panel.select_remote(p_del.clone());
                                            }
                                            this.sftp_delete_selection(tab_id, cx);
                                        });
                                    }
                                }))
                                .item(
                                    PopupMenuItem::new("刷新")
                                        .icon(IconName::RefreshCw)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_refresh(tab_id, cx);
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("复制条目 (Ctrl+C)")
                                        .icon(IconName::Copy)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_copy_selection(tab_id, cx);
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("粘贴至此处 (Ctrl+V)")
                                        .icon(IconName::Clipboard)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_paste_clipboard(tab_id, window, cx);
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
            .track_scroll(&remote_scroll_handle)
            .into_any_element()
        }
    }
}

/// 远程内容区的渲染输入：条目、加载态、标签归属与滚动句柄。
pub(super) struct RemoteContentProps {
    pub(super) remote_entries: Vec<RemoteFile>,
    pub(super) loading: bool,
    pub(super) tab_id: TabId,
    pub(super) tab_conn_id: Option<ConnectionId>,
    pub(super) remote_scroll_handle: VirtualListScrollHandle,
}
