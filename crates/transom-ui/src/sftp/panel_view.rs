//! SFTP 双栏文件管理器主面板视图渲染。

use super::TransferStatus;
use super::columns::{SftpColumn, SftpColumnId};
use super::file_row::{insert_parent_local_entry, insert_parent_remote_entry};
use super::filter::filter_named_entries;
use super::local_pane_view::SftpLocalPaneProps;
use super::queue_view::{SftpQueuePaneProps, SftpStatusBarProps};
use super::remote_pane::RemotePaneProps;
use super::state::SftpRemotePaneState;
use super::{LocalFileEntry, SftpRemotePaneRef};
use crate::app::AppShell;
use crate::app::AppState;
use crate::app::app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_core::RemoteFile;

impl AppState {
    /// 过滤单个远端列的条目：隐藏开关、排除规则与列内过滤词共同生效。
    fn filter_remote_entries(
        pane: &SftpRemotePaneState,
        show_hidden: bool,
        filter_query: &str,
        hidden_patterns: &str,
    ) -> Vec<RemoteFile> {
        let mut remote_entries = filter_named_entries(
            &pane.entries,
            |entry| entry.name.as_str(),
            show_hidden,
            filter_query,
            hidden_patterns,
        );
        insert_parent_remote_entry(&mut remote_entries, &pane.path, filter_query);
        remote_entries
    }

    /// 过滤本地列条目：共享面板与本地单栏面板共用同一套规则与父目录注入。
    pub(crate) fn filter_local_entries(
        panel: &super::SftpPanelState,
        show_hidden: bool,
        filter_query: &str,
        hidden_patterns: &str,
    ) -> Vec<LocalFileEntry> {
        let mut local_entries = filter_named_entries(
            &panel.local.entries,
            |entry| entry.name.as_str(),
            show_hidden,
            filter_query,
            hidden_patterns,
        );
        insert_parent_local_entry(&mut local_entries, &panel.local.path, filter_query);
        local_entries
    }
}

impl AppShell {
    /// 当前活动窗格的 SFTP 面板元素：实体实现 `Render`，外壳只负责挂载。
    pub(crate) fn render_sftp_panel_element(&self) -> AnyElement {
        self.state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.sftp_view(tab_id))
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| div().into_any_element())
    }
}

impl SftpPanelView {
    /// 构建面板内容：状态、列、队列与状态栏都在实体上下文里完成。
    ///
    /// 外层容器由 `impl Render` 统一挂载；跨域数据（设置、标签连接）从外壳
    /// 只读获取，渲染路径不会回读本实体。
    pub(crate) fn render_panel_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::from(&*cx);
        let tab_id = self.tab_id;
        let state = &self.state;
        if state.local.only && state.side_remotes.is_empty() {
            return self.render_local_only_content(cx);
        }

        let is_local_only = state.local.only;
        let local_collapsed = state.local.collapsed;
        let _focus_handle = state.focus_handle.clone();
        // 先构建远端列：主远端与并列远端共用同一套视图，只按目标列取状态。
        // 远端列借用 `state`，必须在调用 &mut self 的本地列/队列渲染之前完成。
        // 列按固定次序编排：本地 | 主远端 | 并列远端；折叠列由固定 32px 折叠条
        // 占住自己的位置，所有列始终在同一个可拖拽分栏组里。
        let remote_columns = self.render_remote_columns(state, &p, cx);
        let side_titles = side_remote_titles(state);

        let local_pane = self.render_local_column(state, tab_id, &p, cx);

        let (queue_pane, status_bar_element, queue_expanded) =
            self.render_queue_and_status(state, tab_id, &p, cx);

        self.assemble_panel(
            SftpPanelAssembly {
                local_pane,
                local_collapsed,
                remote_columns,
                side_titles,
                tab_id,
                is_local_only,
                queue_pane,
                status_bar_element,
                queue_expanded,
            },
            &p,
            cx,
        )
    }

    /// 队列与底部状态栏：进度回退、暂停汇总、速度与传输错误。
    fn render_queue_and_status(
        &self,
        state: &super::state::SftpPanelState,
        tab_id: transom_core::TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>, bool) {
        let settings = app_settings(cx).sftp.clone();
        let queue_height = state.queue.height;
        let remote_error = state.queue.error.clone();
        let status = state.queue.status.clone();
        let progress = state.queue.progress;
        let active_count = state.queue.active_transfers.len();

        let local_error = state.local.error.clone();

        let queue = state.queue.tasks.clone();
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
        let resume_enabled = state.queue.resume_enabled;
        let verify_after_transfer = settings.verify_after_transfer;
        let max_concurrent_transfers = settings.max_concurrent_transfers.clamp(1, 8);
        let queue_expanded = state.queue.expanded;
        // 没有活动任务时回退汇总暂停任务进度，避免暂停后进度条消失
        let paused_count = state
            .queue
            .tasks
            .iter()
            .filter(|task| task.status == TransferStatus::Paused)
            .count();
        let paused_tasks = if active_count == 0 {
            state
                .queue
                .tasks
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

        (queue_pane, status_bar_element, queue_expanded)
    }

    /// 面板主体装配：本地列、远端列、顶栏、队列与状态栏。
    fn assemble_panel(
        &mut self,
        assembly: SftpPanelAssembly,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let SftpPanelAssembly {
            local_pane,
            local_collapsed,
            remote_columns,
            side_titles,
            tab_id,
            is_local_only,
            queue_pane,
            status_bar_element,
            queue_expanded,
        } = assembly;
        let local_content = if local_collapsed {
            self.render_sftp_collapsed_bar(SftpColumnId::Local, &side_titles, p, cx)
        } else {
            local_pane
        };
        let mut columns: Vec<SftpColumn> = Vec::with_capacity(remote_columns.len() + 1);
        columns.push(SftpColumn {
            id: SftpColumnId::Local,
            collapsed: local_collapsed,
            content: local_content,
        });
        columns.extend(remote_columns);

        let is_all_collapsed = columns.iter().all(|column| column.collapsed);
        let header =
            self.render_sftp_panel_top_header(tab_id, is_all_collapsed, is_local_only, p, cx);
        let columns_container = self.render_sftp_columns(tab_id, columns, cx);

        // 外层容器与键盘/鼠标监听归实体（见 `impl Render for SftpPanelView`）；
        // 这里只构建内容，避免同一元素上重复挂两套监听。
        let mut state_element = v_flex()
            .id(ElementId::Name("sftp-state-content".into()))
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
            state_element = state_element.child(queue_pane);
            if let Some(status_element) = status_bar_element {
                let status_element = if queue_expanded {
                    status_element
                } else {
                    // 队列收起时菜单与进度条之间没有列表行做分隔，补一条细线
                    div()
                        .w_full()
                        .border_color(p.divider(p.sidebar_border))
                        .child(status_element)
                        .divider_edge(DividerEdge::Top, p.divider_width)
                        .into_any_element()
                };
                state_element = state_element.child(status_element);
            }
        }

        state_element.into_any_element()
    }

    /// 渲染 SFTP 顶栏标题与操作区。
    fn render_sftp_panel_top_header(
        &self,
        tab_id: transom_core::TabId,
        is_all_collapsed: bool,
        is_local_only: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if is_all_collapsed {
            div()
                .h(rems(size::TAB_BAR_HEIGHT))
                .w_full()
                .items_center()
                .justify_center()
                .border_color(p.divider(p.sidebar_border))
                .child(
                    Button::new("sftp-close-all-collapsed")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip(rust_i18n::t!("app.close").to_string())
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(crate::sftp::SftpPanelEvent::ClosePanel { tab_id });
                            cx.notify();
                        })),
                )
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .into_any_element()
        } else {
            let mount_btn = self.render_side_remote_mount_button(p, cx);
            let title_text = if is_local_only {
                rust_i18n::t!("sftp.local_files_title").to_string()
            } else {
                rust_i18n::t!("sftp.title").to_string()
            };
            self.render_sftp_panel_header(title_text, Some(mount_btn), p, cx)
        }
    }

    /// 本地列：过滤、条目快照与差异可用性都在这里准备。
    fn render_local_column(
        &self,
        state: &super::state::SftpPanelState,
        tab_id: transom_core::TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = app_settings(cx).sftp.clone();
        let local_filter_input = state.local.filter_input.clone();
        let local_filter_query = local_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let local_entries = AppState::filter_local_entries(
            state,
            state.local.show_hidden,
            &local_filter_query,
            &settings.panel_hidden_patterns,
        );
        let diff_ready = matches!(
            crate::diff_viewer::local_diff_check(state.main_connection, state),
            crate::diff_viewer::LocalDiffCheck::Ready(_)
        );
        self.render_sftp_local_pane(
            SftpLocalPaneProps {
                tab_id,
                show_filter: state.local.show_filter,
                show_hidden: state.local.show_hidden,
                local_path_input: state.local.path_input.clone(),
                local_filter_input,
                local_entries,
                local_scroll_handle: state.local.scroll_handle.clone(),
                quick_paths: state.local.quick_paths.clone(),
                remote_available: true,
                local_selection: state.local.selection.clone(),
                local_primary: state.local.selected.clone(),
                diff_ready,
            },
            p,
            cx,
        )
    }

    /// 构建远端列：主远端与并列远端共用同一套折叠/过滤/渲染路径。
    ///
    /// 单独成方法后 `render_panel_content` 只负责列顺序与整体拼装；新增一种
    /// 远端列不需要再改主函数。
    fn render_remote_columns(
        &self,
        state: &super::state::SftpPanelState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<super::columns::SftpColumn> {
        let tab_id = self.tab_id;
        let mut remote_columns: Vec<SftpColumn> = Vec::new();
        if !state.local.only && state.main_connection.is_some() {
            let content = self.render_main_remote_content(state, tab_id, p, cx);
            remote_columns.push(SftpColumn {
                id: SftpColumnId::Remote,
                collapsed: state.remote.collapsed,
                content,
            });
        }
        for side_index in 0..state.side_remotes.len() {
            let Some(content) = self.render_side_remote_content(state, side_index, p, cx) else {
                continue;
            };
            remote_columns.push(SftpColumn {
                id: SftpColumnId::Side(side_index),
                collapsed: state.side_remotes[side_index].collapsed,
                content,
            });
        }
        remote_columns
    }

    /// 主远端列内容：折叠条或完整远端面板。
    fn render_main_remote_content(
        &self,
        state: &super::state::SftpPanelState,
        tab_id: transom_core::TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let side_titles = side_remote_titles(state);
        if state.remote.collapsed {
            return self.render_sftp_collapsed_bar(SftpColumnId::Remote, &side_titles, p, cx);
        }
        let Some(connection_id) = state.main_connection else {
            return div().into_any_element();
        };
        let hidden_patterns = app_settings(cx).sftp.panel_hidden_patterns.clone();
        let filter_query = state
            .remote
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let entries = AppState::filter_remote_entries(
            &state.remote,
            state.remote.show_hidden,
            &filter_query,
            &hidden_patterns,
        );
        self.render_remote_pane(
            RemotePaneProps {
                tab_id,
                target: SftpRemotePaneRef::Main,
                side_index: None,
                title: rust_i18n::t!("sftp.remote_title").to_string(),
                pane: &state.remote,
                connection_id,
                show_filter: state.remote.show_filter,
                show_hidden: state.remote.show_hidden,
                sync_navigation: state.sync_navigation,
                show_editor_list: true,
                remote_entries: entries,
                remote_selection: state.remote.selection.clone(),
                remote_primary: state.remote.selected.clone(),
            },
            p,
            cx,
        )
    }

    /// 并列远端列内容；没有连接号时返回 `None`，调用方跳过该列。
    fn render_side_remote_content(
        &self,
        state: &super::state::SftpPanelState,
        side_index: usize,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let side = state.side_remotes.get(side_index)?;
        let side_titles = side_remote_titles(state);
        if side.collapsed {
            return Some(self.render_sftp_collapsed_bar(
                SftpColumnId::Side(side_index),
                &side_titles,
                p,
                cx,
            ));
        }
        let connection_id = side.connection_id?;
        let hidden_patterns = app_settings(cx).sftp.panel_hidden_patterns.clone();
        let filter_query = side
            .filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let entries = AppState::filter_remote_entries(
            side,
            side.show_hidden,
            &filter_query,
            &hidden_patterns,
        );
        Some(self.render_remote_pane(
            RemotePaneProps {
                tab_id: self.tab_id,
                target: SftpRemotePaneRef::Side(connection_id),
                side_index: Some(side_index),
                title: side.title.clone().unwrap_or_default(),
                pane: side,
                connection_id,
                show_filter: side.show_filter,
                show_hidden: side.show_hidden,
                sync_navigation: state.sync_navigation,
                show_editor_list: state.local.only && side_index == 0,
                remote_entries: entries,
                remote_selection: side.selection.clone(),
                remote_primary: side.selected.clone(),
            },
            p,
            cx,
        ))
    }
}

/// 并列远端列的标题菜单项：索引 + 显示名。
fn side_remote_titles(state: &super::state::SftpPanelState) -> Vec<(usize, String)> {
    state
        .side_remotes
        .iter()
        .enumerate()
        .map(|(index, side)| (index, side.title.clone().unwrap_or_default()))
        .collect()
}

/// 面板主体装配输入：各列内容与折叠/队列状态。
struct SftpPanelAssembly {
    local_pane: AnyElement,
    local_collapsed: bool,
    remote_columns: Vec<super::columns::SftpColumn>,
    side_titles: Vec<(usize, String)>,
    tab_id: transom_core::TabId,
    is_local_only: bool,
    queue_pane: AnyElement,
    status_bar_element: Option<AnyElement>,
    queue_expanded: bool,
}
