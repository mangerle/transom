//! SFTP 双栏文件管理器主面板视图渲染。

use super::local_pane_view::SftpLocalPaneProps;
use super::queue_view::{SftpQueuePaneProps, SftpStatusBarProps};
use super::remote_content::RemoteContentProps;
use super::remote_pane::RemotePaneProps;
use super::{LocalFileEntry, fuzzy_match_name};
use crate::app::AppShell;
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::*;
use transom_core::{RemoteFile, TransferFilter, TransferStatus};

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
        let tab_conn_id = tab.connection_id;
        let Some(panel) = tab.sftp.as_ref() else {
            return div().into_any_element();
        };
        if panel.local_only && panel.side_remotes.is_empty() {
            return self.render_local_only_panel(tab_id, p, cx);
        }

        let is_local_only = panel.local_only;
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

        // 显示排除：与传输过滤共用 glob 语法，只影响可见性不拦截传输；
        // 规则非法时静默跳过（与传输过滤的宽容语义一致）。
        let hidden_matcher =
            TransferFilter::from_text(&self.settings.sftp.panel_hidden_patterns.clone())
                .matcher()
                .ok();
        let is_hidden = |name: &str| {
            hidden_matcher
                .as_ref()
                .is_some_and(|matcher| matcher.matches(name, false))
        };
        let mut remote_entries: Vec<RemoteFile> = panel
            .entries
            .iter()
            .filter(|entry| {
                (show_hidden || !entry.name.starts_with('.'))
                    && !is_hidden(&entry.name)
                    && (remote_filter_query.is_empty()
                        || fuzzy_match_name(&entry.name, &remote_filter_query))
            })
            .cloned()
            .collect();
        if panel.path != "/"
            && !panel.path.is_empty()
            && (remote_filter_query.is_empty() || fuzzy_match_name("..", &remote_filter_query))
        {
            let parent = super::parent_remote(&panel.path);
            remote_entries.insert(
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
        let remote_path_input = panel.path_input.clone();
        let loading = panel.loading;
        let remote_error = panel.error.clone();
        let status = panel.status.clone();
        let progress = panel.progress;
        let active_count = panel.active_transfers.len();

        let local_path_input = panel.local_path_input.clone();
        let mut local_entries: Vec<LocalFileEntry> = panel
            .local_entries
            .iter()
            .filter(|entry| {
                (show_hidden || !entry.name.starts_with('.'))
                    && (local_filter_query.is_empty()
                        || fuzzy_match_name(&entry.name, &local_filter_query))
            })
            .cloned()
            .collect();
        if let Some(parent) = panel.local_path.parent()
            && panel.local_path != parent
            && (local_filter_query.is_empty() || fuzzy_match_name("..", &local_filter_query))
        {
            local_entries.insert(
                0,
                LocalFileEntry {
                    name: "..".to_string(),
                    path: parent.to_path_buf(),
                    is_dir: true,
                    size: 0,
                },
            );
        }
        let local_error = panel.local_error.clone();
        let quick_paths = panel.local_quick_paths.clone();
        let local_collapsed = panel.local_collapsed;
        let remote_collapsed = panel.remote_collapsed;
        let side_remotes_info = panel
            .side_remotes
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.collapsed, s.title.clone()))
            .collect::<Vec<_>>();
        let remote_current_dir = panel.path.clone();

        let queue = panel.queue.clone();
        let total_speed: u64 = queue
            .iter()
            .filter(|t| t.status == TransferStatus::Transferring)
            .filter_map(|t| t.speed_bps)
            .sum();
        let total_speed_bps = if total_speed > 0 {
            Some(total_speed)
        } else {
            None
        };
        let resume_enabled = panel.resume_enabled;
        let verify_after_transfer = self.settings.sftp.verify_after_transfer;
        let max_concurrent_transfers = self.settings.sftp.max_concurrent_transfers.clamp(1, 8);
        let queue_expanded = panel.queue_expanded;
        // 没有活动任务时回退汇总暂停任务进度，避免暂停后进度条消失
        let paused_count = panel
            .queue
            .iter()
            .filter(|task| task.status == TransferStatus::Paused)
            .count();
        let paused_tasks = if active_count == 0 {
            panel
                .queue
                .iter()
                .filter(|task| task.status == TransferStatus::Paused)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let display_progress = if active_count == 0 && !paused_tasks.is_empty() {
            let transferred = paused_tasks
                .iter()
                .fold(0u64, |sum, task| sum.saturating_add(task.transferred));
            let mut total_known = true;
            let total = paused_tasks.iter().fold(0u64, |sum, task| match task.size {
                Some(size) => sum.saturating_add(size),
                None => {
                    total_known = false;
                    sum
                }
            });
            Some((transferred, total_known.then_some(total)))
        } else {
            progress
        };

        let local_pane = self.render_sftp_local_pane(
            SftpLocalPaneProps {
                tab_id,
                show_filter,
                local_path_input,
                local_filter_input,
                local_entries,
                local_scroll_handle,
                quick_paths,
                remote_available: true,
            },
            p,
            cx,
        );

        // 构建远程文件内容（虚拟列表）
        // 远程文件内容区（虚拟列表）的构建见 render_remote_content。
        let remote_content = self.render_remote_content(
            RemoteContentProps {
                remote_entries,
                loading,
                tab_id,
                tab_conn_id,
                remote_scroll_handle: remote_scroll_handle.clone(),
            },
            p,
            cx,
        );

        // 双栏右侧：远程 SFTP 文件系统
        let remote_pane = self.render_remote_pane(
            RemotePaneProps {
                tab_id,
                remote_current_dir,
                remote_path_input,
                remote_filter_input,
                show_filter,
                show_hidden,
                sync_navigation,
                remote_content,
                remote_scroll_handle,
            },
            p,
            cx,
        );

        let queue_pane = self.render_sftp_queue_pane(
            SftpQueuePaneProps {
                tab_id,
                queue: &queue,
                resume_enabled,
                verify_after_transfer,
                max_concurrent_transfers,
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
                active_count,
                paused_count,
                queue_expanded,
                remote_error,
                local_error,
                status,
                total_speed_bps,
            },
            p,
            cx,
        );

        let mut expanded_panels: Vec<AnyElement> = Vec::new();
        if !local_collapsed {
            expanded_panels.push(local_pane);
        }
        if !is_local_only && !remote_collapsed {
            expanded_panels.push(remote_pane);
        }
        for (side_index, is_collapsed, _) in &side_remotes_info {
            if !*is_collapsed {
                expanded_panels.push(self.render_sftp_side_remote_pane(tab_id, *side_index, p, cx));
            }
        }

        let is_all_collapsed = expanded_panels.is_empty();

        let header = if is_all_collapsed {
            div()
                .h(px(32.0))
                .w_full()
                .items_center()
                .justify_center()
                .border_b_1()
                .border_color(p.sidebar_border)
                .child(
                    Button::new("sftp-close-all-collapsed")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip(rust_i18n::t!("app.close").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                                tab.sftp = None;
                            }
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            let mount_btn = self.render_side_remote_mount_button(tab_id, p, cx);
            let title_text = if is_local_only {
                rust_i18n::t!("sftp.local_files_title").to_string()
            } else {
                rust_i18n::t!("sftp.title").to_string()
            };
            self.render_sftp_panel_header(title_text, Some(mount_btn), p, cx)
        };

        let center_content: Option<AnyElement> = match expanded_panels.len() {
            0 => None,
            1 => Some(
                div()
                    .size_full()
                    .overflow_hidden()
                    .child(expanded_panels.into_iter().next().unwrap())
                    .into_any_element(),
            ),
            _ => {
                let mut resizable =
                    h_resizable(ElementId::Name(format!("sftp-multi-pane-{tab_id}").into()));
                for pane in expanded_panels {
                    resizable = resizable.child(resizable_panel().child(pane));
                }
                Some(resizable.into_any_element())
            }
        };

        let mut columns_container = h_flex().size_full().overflow_hidden();

        if let Some(content) = center_content {
            columns_container =
                columns_container.child(div().flex_1().h_full().overflow_hidden().child(content));
        }

        // 所有折叠竖条统一靠在右侧并排展示
        if local_collapsed {
            columns_container =
                columns_container.child(self.render_sftp_local_collapsed_bar(tab_id, p, cx));
        }
        if !is_local_only && remote_collapsed {
            columns_container =
                columns_container.child(self.render_sftp_remote_collapsed_bar(tab_id, p, cx));
        }
        for (side_index, is_collapsed, side_title) in &side_remotes_info {
            if *is_collapsed {
                columns_container =
                    columns_container.child(self.render_sftp_side_remote_collapsed_bar(
                        tab_id,
                        *side_index,
                        side_title,
                        p,
                        cx,
                    ));
            }
        }

        let mut panel_element = v_flex()
            .id(ElementId::Name("sftp-panel-container".into()))
            .track_focus(&focus_handle)
            .key_context(keymap::SFTP_CONTEXT)
            .on_action(cx.listener(Self::on_focus_sftp_filter))
            .on_action(cx.listener(Self::on_toggle_sftp_hidden))
            .on_action(cx.listener(Self::on_delete_sftp_selection))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.control || event.keystroke.modifiers.platform {
                    if event.keystroke.key == "c" {
                        this.sftp_copy_selection(tab_id, cx);
                    } else if event.keystroke.key == "v" {
                        this.sftp_paste_clipboard(tab_id, window, cx);
                    }
                }
            }))
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
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(columns_container),
            );

        if !is_all_collapsed {
            panel_element = panel_element.child(queue_pane);
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
        }

        panel_element.into_any_element()
    }
}
