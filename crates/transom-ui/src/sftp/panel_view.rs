//! SFTP 双栏文件管理器主面板视图渲染。

use super::local_pane_view::SftpLocalPaneProps;
use super::queue_view::{SftpQueuePaneProps, SftpStatusBarProps};
use super::{LocalFileEntry, format_size, fuzzy_match_name, sftp_file_icon};
use crate::app::AppShell;
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Sizable, v_virtual_list};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{RemoteFile, TransferStatus, format_octal_permissions};

impl AppShell {
    pub(crate) fn render_sftp_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(tab_id) = self.active_tab else {
            return div().into_any_element();
        };
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return div().into_any_element();
        };
        let Some(panel) = tab.sftp.as_ref() else {
            return div().into_any_element();
        };

        let show_filter = panel.show_filter;
        let show_hidden = panel.show_hidden;
        let sync_navigation = panel.sync_navigation;
        let queue_height = panel.queue_height;
        let focus_handle = panel.focus_handle.clone();
        let local_scroll_handle = panel.local_scroll_handle.clone();
        let remote_scroll_handle = panel.remote_scroll_handle.clone();
        let remote_filter_input = panel.remote_filter_input.clone();
        let local_filter_input = panel.local_filter_input.clone();
        let remote_filter_query = remote_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let local_filter_query = local_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();

        let remote_entries: Vec<RemoteFile> = panel
            .entries
            .iter()
            .filter(|entry| {
                (show_hidden || !entry.name.starts_with('.'))
                    && (remote_filter_query.is_empty()
                        || fuzzy_match_name(&entry.name, &remote_filter_query))
            })
            .cloned()
            .collect();
        let remote_path_input = panel.path_input.clone();
        let loading = panel.loading;
        let remote_error = panel.error.clone();
        let status = panel.status.clone();
        let progress = panel.progress;
        let active_op = panel.active_op;

        let local_path_input = panel.local_path_input.clone();
        let local_entries: Vec<LocalFileEntry> = panel
            .local_entries
            .iter()
            .filter(|entry| {
                (show_hidden || !entry.name.starts_with('.'))
                    && (local_filter_query.is_empty()
                        || fuzzy_match_name(&entry.name, &local_filter_query))
            })
            .cloned()
            .collect();
        let local_error = panel.local_error.clone();

        let queue = panel.queue.clone();
        let resume_enabled = panel.resume_enabled;
        let verify_after_transfer = self.settings.sftp.verify_after_transfer;
        let queue_expanded = panel.queue_expanded;
        // 没有活动任务时回退到暂停任务进度，避免暂停后进度条消失
        let paused_task = if active_op.is_none() {
            panel
                .queue
                .iter()
                .rev()
                .find(|task| task.status == TransferStatus::Paused)
        } else {
            None
        };
        let paused_op = paused_task.map(|task| task.id);
        let display_progress = paused_task
            .map(|task| (task.transferred, task.size))
            .or(progress);

        let local_pane = self.render_sftp_local_pane(
            SftpLocalPaneProps {
                tab_id,
                show_filter,
                local_path_input,
                local_filter_input,
                local_entries,
                local_scroll_handle,
            },
            p,
            cx,
        );

        // 构建远程文件内容（虚拟列表）
        let remote_content = if remote_entries.is_empty() && !loading {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.empty").to_string())
                .into_any_element()
        } else {
            let remote_sizes = std::rc::Rc::new(vec![size(px(0.), px(28.)); remote_entries.len()]);
            let remote_entries_rc = std::rc::Rc::new(remote_entries);
            let p = *p;
            let app_entity = cx.entity().clone();
            v_virtual_list(
                cx.entity().clone(),
                format!("sftp-remote-virtual-list-{tab_id}"),
                remote_sizes,
                move |this, range, _window, cx| {
                    let remote_selected = this
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(|tab| tab.sftp.as_ref())
                        .and_then(|panel| panel.selected.clone());
                    let app_entity = app_entity.clone();
                    range
                        .filter_map(|ix| {
                            let entry = remote_entries_rc.get(ix)?;
                            let entry_path = entry.path.clone();
                            let entry_name = entry.name.clone();
                            let entry_is_dir = entry.is_dir;
                            let is_selected =
                                remote_selected.as_deref() == Some(entry.path.as_str());
                            let row = h_flex()
                                .id(ElementId::Name(
                                    format!("remote-file-{}", entry.path).into(),
                                ))
                                .px_2()
                                .py_1()
                                .gap_2()
                                .items_center()
                                .rounded_sm()
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
                            let app_cm = app_entity.clone();
                            let row = row
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        if let Some(panel) = this
                                            .tabs
                                            .iter()
                                            .find(|tab| tab.id == tab_id)
                                            .and_then(|tab| tab.sftp.as_ref())
                                        {
                                            window.focus(&panel.focus_handle, cx);
                                        }
                                        if event.click_count >= 2 {
                                            if entry_is_dir {
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
                                                    cx,
                                                );
                                            }
                                            return;
                                        }
                                        if let Some(panel) = this
                                            .tabs
                                            .iter_mut()
                                            .find(|tab| tab.id == tab_id)
                                            .and_then(|tab| tab.sftp.as_mut())
                                        {
                                            panel.select_remote(entry_path.clone());
                                            let name = entry_name.clone();
                                            panel.name_input.update(cx, |state, cx| {
                                                state.set_value(name, window, cx);
                                            });
                                        }
                                        cx.notify();
                                    }),
                                )
                                .child(sftp_file_icon(entry_is_dir, &p))
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
                                            .rounded_sm()
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
                                ))
                                .context_menu(move |menu, _window, _cx| {
                                    let app = app_cm.clone();
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
                                                            panel.select_remote(p_dl.clone());
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
                                                    move |_, _, cx| {
                                                        app.update(cx, |this, cx| {
                                                            this.sftp_edit_remote_file(
                                                                tab_id,
                                                                p_edit.clone(),
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
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(p_cp.clone()),
                                                    );
                                                    app.update(cx, |this, cx| {
                                                        if let Some(panel) = this
                                                            .tabs
                                                            .iter_mut()
                                                            .find(|t| t.id == tab_id)
                                                            .and_then(|t| t.sftp.as_mut())
                                                        {
                                                            panel.status = Some(format!(
                                                                "已复制远端路径: {p_cp}"
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
                                                        this.sftp_open_chmod_modal(
                                                            tab_id, window, cx,
                                                        );
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
                                    .item(
                                        PopupMenuItem::new("删除").icon(IconName::Trash).on_click(
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
                                                            panel.select_remote(p_del.clone());
                                                        }
                                                        this.sftp_delete(tab_id, cx);
                                                    });
                                                }
                                            },
                                        ),
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
                                    )
                                });
                            Some(row.into_any_element())
                        })
                        .collect()
                },
            )
            .track_scroll(&remote_scroll_handle)
            .into_any_element()
        };

        // 双栏右侧：远程 SFTP 文件系统
        let remote_pane = v_flex()
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
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
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
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let selected = this
                                            .tabs
                                            .iter()
                                            .find(|t| t.id == tab_id)
                                            .and_then(|t| t.sftp.as_ref())
                                            .and_then(|p| p.selected.clone());
                                        if let Some(remote_path) = selected {
                                            this.sftp_edit_remote_file(tab_id, remote_path, cx);
                                        }
                                    })),
                            )
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
                                        this.sftp_delete(tab_id, cx);
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
                                            .tabs
                                            .iter_mut()
                                            .find(|t| t.id == tab_id)
                                            .and_then(|t| t.sftp.as_mut())
                                        {
                                            panel.sync_navigation = !panel.sync_navigation;
                                            panel.status = Some(
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
            .child(
                v_flex()
                    .id(ElementId::Name("sftp-remote-list".into()))
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .p_2()
                    .child(remote_content)
                    .vertical_scrollbar(&remote_scroll_handle),
            )
            .into_any_element();

        let queue_pane = self.render_sftp_queue_pane(
            SftpQueuePaneProps {
                tab_id,
                queue: &queue,
                resume_enabled,
                verify_after_transfer,
                queue_expanded,
                queue_height,
            },
            p,
            cx,
        );
        let status_bar_element = self.render_sftp_status_bar(
            SftpStatusBarProps {
                tab_id,
                loading,
                progress: display_progress,
                active_op,
                paused_op,
                queue_expanded,
                remote_error,
                local_error,
                status,
            },
            p,
            cx,
        );

        let header = self.render_sftp_panel_header(p, cx);

        let mut panel_element = v_flex()
            .id(ElementId::Name("sftp-panel-container".into()))
            .track_focus(&focus_handle)
            .key_context(keymap::SFTP_CONTEXT)
            .on_action(cx.listener(Self::on_focus_sftp_filter))
            .on_action(cx.listener(Self::on_toggle_sftp_hidden))
            .on_action(cx.listener(Self::on_delete_sftp_selection))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                this.sftp_update_resize_queue(tab_id, event.position.y.into(), cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.sftp_stop_resize_queue(tab_id, cx);
                }),
            )
            .size_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .border_l_1()
            .border_color(p.sidebar_border)
            .child(header)
            .child(
                div().flex_1().w_full().overflow_hidden().child(
                    h_resizable(ElementId::Name(format!("sftp-dual-pane-{tab_id}").into()))
                        .child(resizable_panel().child(local_pane))
                        .child(resizable_panel().child(remote_pane)),
                ),
            )
            .child(queue_pane);

        if let Some(status_element) = status_bar_element {
            let status_element = if queue_expanded {
                status_element
            } else {
                // 队列收起时菜单与进度条之间没有列表行做分隔，补一条细线
                div()
                    .w_full()
                    .border_t_1()
                    .border_color(p.sidebar_border)
                    .child(status_element)
                    .into_any_element()
            };
            panel_element = panel_element.child(status_element);
        }

        panel_element.into_any_element()
    }
}
