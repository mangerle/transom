//! SFTP 本地文件浏览器左栏视图渲染。

use super::preview::{PreviewKey, SftpHoverTarget};
use super::side_remote::{SftpDragItem, SftpDragPreview};
use super::{LocalFileEntry, LocalQuickPaths, format_size, sftp_file_icon};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
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
                                                            this.sftp_local_go_up(
                                                                tab_id, window, cx,
                                                            );
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
                                                            this.sftp_local_refresh(tab_id, cx);
                                                        });
                                                    }
                                                }),
                                        );
                                }
                                let p_up = entry_path_cm.clone();
                                let p_cp = entry_path_cm.clone();
                                let p_sym = entry_path_cm.clone();
                                let p_prop = entry_path_cm.clone();
                                let p_ren = entry_path_cm.clone();
                                let p_del = entry_path_cm.clone();
                                let mut menu = menu;
                                if remote_available {
                                    menu = menu.item(
                                        PopupMenuItem::new("上传").icon(IconName::Upload).on_click(
                                            {
                                                let app = app.clone();
                                                move |_, _, cx| {
                                                    app.update(cx, |this, cx| {
                                                        if let Some(panel) = this
                                                            .tabs
                                                            .iter_mut()
                                                            .find(|t| t.id == tab_id)
                                                            .and_then(|t| t.sftp.as_mut())
                                                        {
                                                            if panel.local_selection.contains(&p_up)
                                                            {
                                                                panel.set_local_primary(
                                                                    p_up.clone(),
                                                                );
                                                            } else {
                                                                panel.select_local(p_up.clone());
                                                            }
                                                        }
                                                        this.sftp_queue_upload(tab_id, cx);
                                                    });
                                                }
                                            },
                                        ),
                                    );
                                }
                                menu.item(
                                    PopupMenuItem::new("复制绝对路径")
                                        .icon(IconName::Copy)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    p_cp.display().to_string(),
                                                ));
                                                app.update(cx, |this, cx| {
                                                    if let Some(panel) = this
                                                        .tabs
                                                        .iter_mut()
                                                        .find(|t| t.id == tab_id)
                                                        .and_then(|t| t.sftp.as_mut())
                                                    {
                                                        panel.status = Some(format!(
                                                            "已复制本地路径: {}",
                                                            p_cp.display()
                                                        ));
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
                                                        panel.select_local(p_sym.clone());
                                                    }
                                                    this.sftp_open_symlink_modal(
                                                        tab_id, false, window, cx,
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
                                                    this.sftp_show_local_properties(
                                                        tab_id,
                                                        p_prop.clone(),
                                                        cx,
                                                    );
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("新建本地文件夹")
                                        .icon(IconName::FolderPlus)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, window, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_open_mkdir_modal(
                                                        tab_id, false, window, cx,
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
                                                        panel.select_local(p_ren.clone());
                                                    }
                                                    this.sftp_open_rename_modal(
                                                        tab_id, false, window, cx,
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
                                                && !panel.local_selection.contains(&p_del)
                                            {
                                                panel.select_local(p_del.clone());
                                            }
                                            this.sftp_local_delete_selection(tab_id, cx);
                                        });
                                    }
                                }))
                                .item(
                                    PopupMenuItem::new("打开文件管理器")
                                        .icon(IconName::FolderOpen)
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.sftp_open_local_file_manager(tab_id, cx);
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
                                                    this.sftp_local_refresh(tab_id, cx);
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
            .track_scroll(&local_scroll_handle)
            .into_any_element()
        };

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
                                    .child(rust_i18n::t!("sftp.local_title").to_string()),
                            )
                            .child(div().flex_1().child(Input::new(&local_path_input).w_full()))
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
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_local_refresh(tab_id, cx)
                                    })),
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
                            .child(Self::local_quick_path_button(
                                "local-quick-home",
                                IconName::House,
                                rust_i18n::t!("sftp.home").to_string(),
                                quick_paths.home.clone(),
                                tab_id,
                                cx,
                            ))
                            .child(Self::local_quick_path_button(
                                "local-quick-desktop",
                                IconName::Monitor,
                                rust_i18n::t!("sftp.desktop").to_string(),
                                quick_paths.desktop.clone(),
                                tab_id,
                                cx,
                            ))
                            .child(Self::local_quick_path_button(
                                "local-quick-downloads",
                                IconName::FolderDown,
                                rust_i18n::t!("sftp.downloads").to_string(),
                                quick_paths.downloads.clone(),
                                tab_id,
                                cx,
                            ))
                            .child(div().w(px(1.0)).h(px(14.0)).mx_1().bg(p.sidebar_border))
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
                            ),
                    )
                    .when(show_filter, |this| {
                        this.child(Input::new(&local_filter_input).w_full())
                    }),
            )
            .child({
                let app_local_drop = cx.entity().clone();
                let drop_bg = p.sidebar_accent.opacity(0.3);
                let drop_border = p.primary;
                v_flex()
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
                                    this.sftp_enqueue_download_path(
                                        tab_id,
                                        path.clone(),
                                        target_local,
                                        cx,
                                    );
                                }
                            }
                        });
                    })
                    .child(local_content)
                    .vertical_scrollbar(&local_scroll_handle)
            })
            .into_any_element()
    }

    fn local_quick_path_button(
        id: &'static str,
        icon: IconName,
        label: String,
        target: Option<PathBuf>,
        tab_id: TabId,
        cx: &mut Context<Self>,
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

    pub(crate) fn render_sftp_panel_header(
        &self,
        title: String,
        extra: Option<AnyElement>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let close_btn = Button::new("close-sftp")
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip("关闭面板")
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(tab_id) = this.active_tab
                    && let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == tab_id)
                {
                    tab.sftp = None;
                }
                cx.notify();
            }));

        let mut right_actions = h_flex().gap_1().items_center();
        if let Some(extra_element) = extra {
            right_actions = right_actions.child(extra_element);
        }
        right_actions = right_actions.child(close_btn);

        h_flex()
            .h(px(32.0))
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(p.sidebar_border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(right_actions)
            .into_any_element()
    }
}
