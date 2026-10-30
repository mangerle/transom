//! SFTP 并列远端的挂载/折叠状态与跨列剪贴板、拖放动作。
//!
//! 列状态与渲染已统一到 `SftpRemotePaneState` 与 `remote_pane`；本模块只保留
//! 挂载生命周期、连接查询与跨列传输编排。
//!
//! 设计原理（Why）：
//! - 实现初衷：支持在单个终端标签的 SFTP 视图中挂载多个远端会话（本地 + 远端 A + 远端 B...），
//!   免去在多个标签页之间反复来回切换，直观实现同屏比对与跨主机流式复制；
//! - 核心优势：
//!   1. 各列（本地列与并列远端列）均支持独立一键收起折叠，最大化屏幕有效阅读空间；
//!   2. 支持跨列拖拽（Drag & Drop）、Ctrl+C / Ctrl+V 剪贴板与右键菜单三重交互闭环；
//!   3. 挂载器支持复用当前已建立连接的 SSH 会话（秒级挂载）或快速连接已保存会话；
//! - 代价与局限：并列多列受屏幕实际宽度影响，单列过窄时由横向自适应和列折叠共同吸收。

mod clipboard;
mod types;

pub(crate) use self::types::{
    SftpActivePane, SftpClipboardData, SftpDragItem, SftpDragPreview, SftpRemoteCopyParams,
    active_pane_after_removing,
};

use crate::app::AppShell;
use crate::sftp::columns::SftpColumnId;
use crate::sftp::{
    SftpPanelState, SftpRemotePaneRef, SftpRemotePaneState, SftpTransferRequest, TransferMode,
    TransferTask,
};
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use transom_core::{ConnectionId, TabId, TransferDirection};

/// 新挂载并列远端列的建议宽度。
///
/// 取当前展开列总宽按"挂载后列数"均分，再夹到可读区间：下限保证两行工具栏
/// 与路径框有基本空间，上限避免新列一上来就吃掉主远端；同时给现有列留出
/// 各自的最小宽度，面板本身很窄时不把已有列挤到裁切。分栏组还没有测量值时
/// 退回固定默认宽度。
fn side_column_width_hint(panel: &SftpPanelState, cx: &App) -> f32 {
    let sizes = panel.column_state.read(cx).sizes();
    let total: f32 = sizes.iter().map(|size| f32::from(*size)).sum();
    let columns = panel.expanded_column_ids().len().max(1);
    let fair = if total > 0.0 {
        total / (columns + 1) as f32
    } else {
        300.0
    };
    let hint = fair.clamp(300.0, 420.0);
    if total > 0.0 {
        // 现有每列至少保留 100px，剩下的才给新列；面板过窄时宁可让新列
        // 窄一些，也不把已有列压到裁切。
        hint.min((total - 100.0 * columns as f32).max(100.0))
    } else {
        hint
    }
}

impl AppShell {
    /// 挂载并列远端会话到当前 SFTP 面板。
    pub(crate) fn sftp_mount_side_remote(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        title: String,
        initial_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        // 避免重复挂载相同连接
        if panel
            .side_remotes
            .iter()
            .any(|s| s.connection_id == Some(connection_id))
        {
            panel.set_status(
                rust_i18n::t!("sftp.sidebar.already_mounted", title = title.as_str()).to_string(),
            );
            cx.notify();
            return;
        }

        let mut side_state =
            SftpRemotePaneState::new_side(connection_id, title, initial_path.clone(), window, cx);
        // 路径输入框回车 = 进入目录：并列列与主远端保持同一交互。
        let enter_sub = cx.subscribe_in(
            &side_state.path_input,
            window,
            move |this, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_enter_path_pane(tab_id, SftpRemotePaneRef::Side(connection_id), cx);
                }
            },
        );
        side_state.subscriptions.push(enter_sub);
        // 新列若交给分栏组自行测量，会被压到 0 或最小宽度：现有列已经把容器
        // 占满，新列没有可增长的空间，工具栏只剩一两个按钮甚至整列空白。挂载时
        // 按实测列宽重新分配现有展开列，给新列留出可读宽度；面板本身也放不下时
        // 再向工作区申请加宽。
        let width_hint = side_column_width_hint(panel, cx);
        let mount_plan = super::columns::plan_side_mount(panel, width_hint, cx);
        if let Some(plan) = &mount_plan {
            for (id, width) in &plan.widths {
                panel.column_widths.remember(*id, *width);
            }
        }
        let extra_width = mount_plan.map(|plan| plan.extra_width).unwrap_or(0.0);
        let side_index = panel.side_remotes.len();
        panel
            .column_widths
            .remember(SftpColumnId::Side(side_index), width_hint);
        panel.side_remotes.push(side_state);
        panel.mount_menu_open = false;
        self.reset_sftp_column_state(tab_id, cx);
        if extra_width > 0.5 {
            self.grow_sftp_panel_width(extra_width, window, cx);
        }
        log::info!(
            "挂载并列远端列，窗格: {tab_id}，连接: {connection_id}，初始目录: {initial_path}，建议列宽: {width_hint:.0}，面板加宽: {extra_width:.0}"
        );
        self.request_sftp_list_for_connection(tab_id, connection_id, initial_path, cx);
        cx.notify();
    }

    /// 关闭并列远端列。
    pub(crate) fn sftp_close_side_remote(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let removed = if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && index < panel.side_remotes.len()
        {
            let removed = panel.side_remotes[index].connection_id;
            panel.side_remotes.remove(index);
            if let Some(removed) = removed {
                panel.active_pane = active_pane_after_removing(panel.active_pane, removed);
            }
            true
        } else {
            false
        };
        if removed {
            self.reset_sftp_column_state(tab_id, cx);
            cx.notify();
        }
    }

    /// 切换并列远端列的展开与折叠。
    pub(crate) fn sftp_toggle_side_remote_collapse(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        // 折叠前先记下当前列宽：折叠后这一列不再参与展开宽度测量。
        self.remember_sftp_column_widths(tab_id, cx);
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id)
            && let Some(side) = panel.side_remotes.get_mut(index)
        {
            side.collapsed = !side.collapsed;
        }
        self.reset_sftp_column_state(tab_id, cx);
        cx.notify();
    }

    /// 切换本地列的展开与折叠。
    pub(crate) fn sftp_toggle_local_collapse(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        // 折叠前先记下当前列宽：折叠条不参与展开宽度测量。
        self.remember_sftp_column_widths(tab_id, cx);
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.local.collapsed = !panel.local.collapsed;
        }
        self.reset_sftp_column_state(tab_id, cx);
        cx.notify();
    }

    /// 切换主远端列的展开与折叠。
    pub(crate) fn sftp_toggle_remote_collapse(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        // 折叠前先记下当前列宽：折叠条不参与展开宽度测量。
        self.remember_sftp_column_widths(tab_id, cx);
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.remote.collapsed = !panel.remote.collapsed;
        }
        self.reset_sftp_column_state(tab_id, cx);
        cx.notify();
    }

    /// 跨远端互传任务入队。
    pub(crate) fn sftp_enqueue_remote_transfer(
        &mut self,
        tab_id: TabId,
        params: SftpRemoteCopyParams,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let src_path = params.source_path.clone();
        let tgt_path = params.target_path.clone();
        let task = TransferTask::new_remote_copy(
            params.source_connection_id,
            params.source_path,
            params.target_connection_id,
            params.target_path,
            None,
            params.is_dir,
        );
        panel.queue.tasks.push(task);
        panel.set_status(
            rust_i18n::t!(
                "sftp.sidebar.remote_copy_queued",
                src = src_path.as_str(),
                tgt = tgt_path.as_str()
            )
            .to_string(),
        );
        self.process_sftp_queue(tab_id, cx);
        cx.notify();
    }

    /// 把单条上传/下载请求加入队列并立即尝试调度。
    ///
    /// 连接由调用方显式给出：调度层不再回退主连接，跨列操作不会静默落到
    /// 另一台主机；目录类型同样由请求携带，避免目录被当作单文件传输。
    pub(crate) fn sftp_enqueue_transfer(
        &mut self,
        request: SftpTransferRequest,
        cx: &mut Context<Self>,
    ) {
        let verify = self.settings.sftp.verify_after_transfer;
        let verify_mode = self.settings.sftp.verify_mode;
        let checksum_algorithm = self.settings.sftp.checksum_algorithm;
        let Some(panel) = self.sftp_panel_mut_tab(request.tab_id) else {
            return;
        };
        let mode = if panel.queue.resume_enabled {
            TransferMode::Resume
        } else {
            TransferMode::Overwrite
        };
        let mut task = if request.is_dir {
            TransferTask::new_directory(
                request.direction,
                request.local_path.clone(),
                request.remote_path.clone(),
                None,
                mode,
            )
        } else {
            TransferTask::new(
                request.direction,
                request.local_path.clone(),
                request.remote_path.clone(),
                None,
                mode,
            )
        };
        match request.direction {
            TransferDirection::Upload => task.target_connection_id = Some(request.connection_id),
            TransferDirection::Download => task.source_connection_id = Some(request.connection_id),
            _ => {}
        }
        task.verify = verify;
        task.verify_mode = verify_mode;
        task.checksum_algorithm = checksum_algorithm;
        panel.queue.tasks.push(task);
        panel.set_status(match request.direction {
            TransferDirection::Upload => rust_i18n::t!(
                "sftp.sidebar.upload_queued_path",
                path = request.remote_path.as_str()
            )
            .to_string(),
            TransferDirection::Download => rust_i18n::t!(
                "sftp.sidebar.download_queued_path",
                path = request.remote_path.as_str()
            )
            .to_string(),
            _ => rust_i18n::t!(
                "sftp.sidebar.transfer_queued_path",
                path = request.remote_path.as_str()
            )
            .to_string(),
        });
        self.process_sftp_queue(request.tab_id, cx);
        cx.notify();
    }

    /// 辅助方法：直接按 tab_id 获取 sftp 面板可变借用。
    pub(crate) fn sftp_panel_mut_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<&mut super::SftpPanelState> {
        self.workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
    }

    /// 请求特定连接的远端目录列表（用于并列远端）。
    ///
    /// 连接归属主远端时按主列处理，否则按并列列处理；加载态与错误清理统一
    /// 走 `request_sftp_list_pane`，不再有第二套发送逻辑。
    pub(crate) fn request_sftp_list_for_connection(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let target = self.workspace.tab(tab_id).and_then(|tab| {
            tab.sftp()
                .and_then(|panel| panel.remote_pane_of_connection(connection_id, tab.connection_id))
        });
        let Some(target) = target else {
            return;
        };
        self.request_sftp_list_pane(tab_id, target, path, cx);
    }
}
