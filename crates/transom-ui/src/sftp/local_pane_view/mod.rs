//! SFTP 本地文件浏览器左栏视图渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为本地文件系统操作提供响应式虚拟文件列表与多选、拖拽支持；
//! - 核心优势：
//!   1. 支持 Shift/Ctrl 连续多选与主项标记；
//!   2. 支持悬浮快速预览；
//!   3. 支持作为原生拖拽源（跨远端上传）。

mod context_menu;
mod panel_header;
mod toolbar;

use self::context_menu::{LocalContextMenuProps, build_local_context_menu};
use self::toolbar::{LocalToolbarProps, render_local_toolbar};
use super::preview::{PreviewKey, SftpHoverTarget};
use super::side_remote::{SftpDragItem, SftpDragPreview};
use super::{LocalFileEntry, LocalQuickPaths, format_size, sftp_file_icon};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::*;
use transom_core::TabId;

pub(crate) struct SftpLocalPaneProps {
    pub(crate) tab_id: TabId,
    pub(crate) show_filter: bool,
    pub(crate) local_path_input: Entity<InputState>,
    pub(crate) local_filter_input: Entity<InputState>,
    pub(crate) local_entries: Vec<LocalFileEntry>,
    pub(crate) local_scroll_handle: VirtualListScrollHandle,
    /// 主目录/桌面/下载等快捷路径；目录不存在时对应按钮禁用。
    pub(crate) quick_paths: LocalQuickPaths,
    /// 本地终端模式没有远端，上传入口必须隐藏。
    pub(crate) remote_available: bool,
}

impl AppShell {
    pub(crate) fn render_sftp_local_pane(
        &mut self,
        props: SftpLocalPaneProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = props.tab_id;
        let show_filter = props.show_filter;
        let local_path_input = props.local_path_input;
        let local_filter_input = props.local_filter_input;
        let local_entries = props.local_entries;
        let local_scroll_handle = props.local_scroll_handle;
        let quick_paths = props.quick_paths;
        let remote_available = props.remote_available;

        let local_content = if local_entries.is_empty() {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.empty").to_string())
                .into_any_element()
        } else {
            let local_sizes = std::rc::Rc::new(vec![size(px(0.), px(28.)); local_entries.len()]);
            let local_entries_rc = std::rc::Rc::new(local_entries);
            let p = p.clone();
            let app_entity = cx.entity().clone();
            v_virtual_list(
                cx.entity().clone(),
                format!("sftp-local-virtual-list-{tab_id}"),
                local_sizes,
                move |this, range, _window, cx| {
                    let (local_selection, local_primary) = this
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(|tab| tab.sftp.as_ref())
                        .map(|panel| (panel.local_selection.clone(), panel.local_selected.clone()))
                        .unwrap_or_default();
                    let app_entity = app_entity.clone();
                    range
                        .filter_map(|ix| {
                            let entry = local_entries_rc.get(ix)?;
                            let entry_path = entry.path.clone();
                            let entry_name = entry.name.clone();
                            let is_dir = entry.is_dir;
                            let is_parent_entry = entry_name == "..";
                            let is_selected = local_selection.contains(&entry.path)
                                || local_primary.as_ref() == Some(&entry.path);
                            let row = h_flex()
                                .id(ElementId::Name(format!("local-file-{}", entry.name).into()))
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
                            let click_entries = local_entries_rc.clone();
                            let hover_target = SftpHoverTarget {
                                tab_id,
                                key: PreviewKey {
                                    is_remote: false,
                                    path: entry.path.to_string_lossy().to_string(),
                                },
                                name: entry.name.clone(),
                                is_dir,
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
                                                this.sftp_local_go_up(tab_id, window, cx);
                                            } else if is_dir {
                                                this.sftp_local_navigate(
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
                                                panel.local_anchor.as_ref().and_then(|anchor| {
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
                                                panel.select_local_range(paths, entry_path.clone());
                                            } else {
                                                panel.select_local(entry_path.clone());
                                            }
                                        } else if event.modifiers.control
                                            || event.modifiers.platform
                                        {
                                            panel.toggle_local_selection(entry_path.clone());
                                        } else {
                                            panel.select_local(entry_path.clone());
                                        }
                                        let selected_count = panel.selected_local_paths().len();
                                        if selected_count > 1 {
                                            panel.status =
                                                Some(format!("已选择 {selected_count} 个本地项目"));
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
                                            if panel.local_selection.contains(&entry_path_right) {
                                                panel.set_local_primary(entry_path_right.clone());
                                            } else {
                                                panel.select_local(entry_path_right.clone());
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
                                    sftp_file_icon(is_dir, &p)
                                })
                                .child(div().flex_1().text_sm().overflow_hidden().child(
                                    if is_parent_entry {
                                        ".. (返回上一级)".to_string()
                                    } else {
                                        entry.name.clone()
                                    },
                                ))
                                .child(div().text_xs().text_color(p.muted_foreground).child(
                                    if is_dir {
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
                                    source_connection_id: None,
                                    paths: vec![entry.path.to_string_lossy().to_string()],
                                    file_name: entry.name.clone(),
                                    is_dir,
                                };
                                let preview_name = entry.name.clone();
                                row.on_drag(drag_payload, move |_, _, _, cx| {
                                    cx.new(|_| SftpDragPreview::new(preview_name.clone(), 1))
                                })
                            };

                            let row = row.context_menu(move |menu, _window, _cx| {
                                build_local_context_menu(
                                    menu,
                                    app_cm.clone(),
                                    LocalContextMenuProps {
                                        tab_id,
                                        entry_path: entry_path_cm.clone(),
                                        is_parent_entry,
                                        remote_available,
                                    },
                                )
                            });
                            Some(row.into_any_element())
                        })
                        .collect()
                },
            )
            .track_scroll(&local_scroll_handle)
            .into_any_element()
        };

        let toolbar = render_local_toolbar(
            LocalToolbarProps {
                tab_id,
                show_filter,
                local_path_input: &local_path_input,
                local_filter_input: &local_filter_input,
                quick_paths: &quick_paths,
                remote_available,
            },
            p,
            cx,
        );

        let app_local_drop = cx.entity().clone();
        let drop_bg = p.sidebar_accent.opacity(0.3);
        let drop_border = p.primary;
        let drop_container = v_flex()
            .id(ElementId::Name("sftp-local-list".into()))
            .relative()
            .flex_1()
            .overflow_hidden()
            .p_2()
            .drag_over::<SftpDragItem>(move |style, _, _, _| {
                style.bg(drop_bg).border_2().border_color(drop_border)
            })
            .on_drop(move |drag: &SftpDragItem, _window, cx| {
                let item = drag.clone();
                app_local_drop.update(cx, |this, cx| {
                    if item.source_connection_id.is_none() {
                        return;
                    }
                    if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                        let local_dir = panel.local_path.clone();
                        for path in &item.paths {
                            let fname = path.rsplit('/').next().unwrap_or(path);
                            let target_local = local_dir.join(fname);
                            this.sftp_enqueue_download_path(tab_id, path.clone(), target_local, cx);
                        }
                    }
                });
            })
            .child(local_content)
            .vertical_scrollbar(&local_scroll_handle);

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(toolbar)
            .child(drop_container)
            .into_any_element()
    }
}
