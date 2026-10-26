//! 应用顶层事件分发与终端事件回调处理模块。

use crate::app::{AppShell, AuthPromptState, HostKeyConfirmState, TabStatus};
use crate::palette::localized_error;
use crate::sftp::{
    SftpPreviewResponse, SftpPropertiesModalState, format_size, remote_file_properties,
    send_desktop_notification,
};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{AppEvent, PromptKind, SecretKind, TabId, TransferDirection, TransferStatus};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

impl AppShell {
    pub(crate) fn handle_app_event(
        &mut self,
        event: AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::TerminalReady {
                tab_id,
                connection_id,
                io,
            } => {
                let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
                    return;
                };
                if self.tabs[index].connection_id != Some(connection_id) {
                    return;
                }
                let session_id = self.tabs[index].session_id;
                let mut profile = self
                    .catalog
                    .sessions
                    .iter()
                    .find(|session| session.id == session_id)
                    .map(|session| session.terminal.clone())
                    .unwrap_or_else(|| self.tabs[index].session.terminal.clone());
                // 光标外观与外观偏好（字体族、中文字体、字号、行高、关键词高亮）属于全局设置，
                // 覆盖会话里可能残留的旧默认值：界面上没有任何会话级字体入口，
                // 若这里不覆盖，用户在设置里改的字体只会作用于已打开的窗格。
                profile.cursor_shape = self.settings.terminal_profile.cursor_shape;
                profile.cursor_blink = self.settings.terminal_profile.cursor_blink;
                profile.font_family = self.settings.terminal_profile.font_family.clone();
                profile.cjk_font_family = self.settings.terminal_profile.cjk_font_family.clone();
                profile.font_size = self.settings.terminal_profile.font_size;
                profile.line_height = self.settings.terminal_profile.line_height;
                // 分隔符同样没有会话级入口，且老会话文件里存的是内核默认值；
                // 不覆盖的话，用户在设置里改的分隔符会被会话里的陈旧值压掉。
                profile.word_separators = self.settings.terminal_profile.word_separators.clone();
                profile.highlight = self.settings.terminal_profile.highlight.clone();
                profile.alternate_scroll = self.settings.terminal_profile.alternate_scroll;
                // 终端配色同理：会话文件里残留的旧调色板会压掉全局选择，
                // 表现为"改了终端配色但重开会话不生效"。运行期程序仍可通过
                // OSC 4/10/11 动态改色，不受此静态基线影响。
                profile.theme_name = self.settings.terminal_profile.theme_name.clone();
                profile.palette = self.settings.terminal_profile.palette.clone();
                // 终端背景透明度跟随窗口整体透明：窗口半透明时终端区也要透出
                // 桌面，否则只有界面变透明、终端仍是一块实色。
                profile.background_transparency = self.settings.window.window_transparency;
                let interaction = self.settings.terminal_interaction;
                let reserved = self.shortcut_recording.filter.clone();
                let key_filter: KeyFilter = Arc::new(move |keystroke| {
                    reserved
                        .read()
                        .iter()
                        .any(|binding| keystroke.should_match(binding))
                });
                // 审计落盘按连接粒度开启：路径在派发前算好，读线程内才创建文件。
                let audit_path =
                    crate::audit::audit_path_for(&self.tabs[index], self.settings.audit.enabled);
                let terminal = cx.new(|cx| {
                    TerminalView::new(
                        window,
                        cx,
                        io,
                        profile,
                        interaction,
                        Some(key_filter),
                        audit_path,
                    )
                });
                // 建议能力挂到视图：查询回调是全应用共享的一份内存快照。
                let provider = self.history_provider.clone();
                terminal.update(cx, |view, _| view.attach_history(provider));
                let focus_handle = terminal.read(cx).focus_handle().clone();
                // 用 subscribe_in：通知要不要弹取决于"窗口是否被系统聚焦"，
                // 普通 subscribe 拿不到 Window。
                let subscription = cx.subscribe_in(
                    &terminal,
                    window,
                    move |this, _, event: &TerminalEvent, window, cx| {
                        this.handle_terminal_event(tab_id, event, window, cx)
                    },
                );
                let tab = &mut self.tabs[index];
                tab.terminal = Some(terminal);
                tab.status = TabStatus::Connected;
                log::info!(
                    "连接建立完成，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
                    tab.session.name,
                    crate::connection::describe_target(&tab.session)
                );
                // 连接成功后自动重连计数清零，下次断线重新享有完整退避序列。
                tab.reconnect_attempts = 0;
                tab.network_traffic = None;
                tab._subscription = Some(subscription);
                // 连接建立期间用户可能已经切走：启动期的默认标签与深链目录标签
                // 会并发建连，晚完成者若无条件激活，会把焦点从后创建的深链标签
                // 抢回启动标签。只有该窗格仍是当前选中项时才切换组内焦点并聚焦。
                let still_active = self.active_tab == Some(tab_id);
                if still_active && let Some(group_index) = self.group_index_for_tab(tab_id) {
                    self.groups[group_index].active_pane = tab_id;
                }
                // 认证已确定通过，此刻才允许把用户勾选保存的凭据写入钥匙串。
                self.flush_pending_credential(session_id, connection_id, cx);
                if still_active {
                    window.focus(&focus_handle, cx);
                }
                self.schedule_startup_commands(tab_id, connection_id, window, cx);
                self.auto_start_forwards(tab_id, connection_id, session_id, cx);
            }
            AppEvent::ConnectionFailed {
                tab_id,
                connection_id,
                error,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    tab.status = TabStatus::Failed(error.clone());
                    tab.network_traffic = None;
                    let message = localized_error(&error);
                    log::error!(
                        "连接建立失败，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}，原因: {message}",
                        tab.session.name,
                        crate::connection::describe_target(&tab.session)
                    );
                    self.error = Some(message);
                }
                self.discard_pending_credential(connection_id);
                self.mark_forwards_stopped(tab_id);
                self.schedule_auto_reconnect(tab_id, connection_id, cx);
            }
            AppEvent::Disconnected {
                tab_id,
                connection_id,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                    && !matches!(tab.status, TabStatus::Failed(_))
                {
                    tab.status = TabStatus::Disconnected;
                    tab.network_traffic = None;
                    log::info!(
                        "连接已断开，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
                        tab.session.name,
                        crate::connection::describe_target(&tab.session)
                    );
                }
                self.mark_forwards_stopped(tab_id);
                self.schedule_auto_reconnect(tab_id, connection_id, cx);
            }
            AppEvent::AuthPrompt {
                tab_id,
                connection_id,
                name,
                instructions,
                prompts,
                responder,
            } => {
                let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
                    return;
                };
                if tab.connection_id != Some(connection_id) {
                    return;
                }
                let session_id = tab.session_id;
                // 同一连接的新问题会作废上一次答案的保存意图：只有最后一次提交的
                // 凭据才可能认证成功，留着旧值会把已被拒绝的口令写进钥匙串。
                self.discard_pending_credential(connection_id);
                let inputs = prompts
                    .iter()
                    .map(|prompt| {
                        let placeholder = prompt.prompt.clone();
                        let masked = !prompt.echo;
                        cx.new(|cx| {
                            let state = InputState::new(window, cx).placeholder(placeholder);
                            if masked { state.masked(true) } else { state }
                        })
                    })
                    .collect();
                // 只有我们自己索取的提示才具备"保存"的意义；服务器下发的 keyboard-interactive
                // 文本可能是一次性挑战（OTP），存下来下次也用不上。
                let secret_kind = match prompts.first().map(|info| info.kind) {
                    Some(PromptKind::Password) => Some(SecretKind::Password),
                    Some(PromptKind::Passphrase) => Some(SecretKind::KeyPassphrase),
                    _ => None,
                };
                self.auth_prompt = Some(AuthPromptState {
                    connection_id,
                    name,
                    instructions,
                    prompts,
                    inputs,
                    responder,
                    error: None,
                    session_id,
                    secret_kind,
                    save: false,
                });
            }
            AppEvent::HostKeyConfirm {
                tab_id,
                connection_id,
                host,
                port,
                key_type,
                fingerprint,
                responder,
            } => {
                let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
                    return;
                };
                if tab.connection_id != Some(connection_id) {
                    return;
                }
                self.host_key_confirm = Some(HostKeyConfirmState {
                    host,
                    port,
                    key_type,
                    fingerprint,
                    responder,
                });
            }
            AppEvent::SftpListing {
                tab_id,
                connection_id,
                path,
                entries,
            } => {
                let mut update_input = None;
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && let Some(panel) = tab.sftp.as_mut()
                {
                    if tab.connection_id == Some(connection_id) {
                        panel.path = path.clone();
                        panel.entries = entries;
                        panel.loading = false;
                        panel.error = None;
                        let valid_paths = panel
                            .entries
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect();
                        panel.sync_remote_selection(&valid_paths);
                        update_input = Some((panel.path_input.clone(), path));
                    } else if let Some(side) = panel
                        .side_remotes
                        .iter_mut()
                        .find(|s| s.connection_id == connection_id)
                    {
                        side.path = path.clone();
                        side.entries = entries;
                        side.loading = false;
                        side.error = None;
                        let valid_paths: std::collections::BTreeSet<String> = side
                            .entries
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect();
                        side.selection.retain(|p| valid_paths.contains(p));
                        if let Some(sel) = &side.selected
                            && !valid_paths.contains(sel)
                        {
                            side.selected = None;
                        }
                        update_input = Some((side.path_input.clone(), path));
                    }
                }
                if let Some((input, path)) = update_input {
                    input.update(cx, |state, cx| state.set_value(path, window, cx));
                }
            }
            AppEvent::SftpProgress {
                tab_id,
                connection_id,
                op_id,
                transferred,
                total,
                speed_bps,
                eta_secs,
            } => {
                let mut applied = false;
                if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id)
                    && let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id)
                {
                    task.transferred = transferred;
                    if total.is_some() {
                        task.size = total;
                    }
                    task.speed_bps = speed_bps;
                    task.eta_secs = eta_secs;
                    panel.recompute_progress();
                    applied = true;
                }
                // 进度事件高频到达：只有当前可见标签的传输队列在显示进度时
                // 才值得整帧重绘外壳；后台标签的数据照常更新，切回时自然呈现。
                if applied {
                    // 浮层的速率与重绘按"窗格是否真的在屏上"判断，比只看活动窗格更宽：
                    // 分屏里两个窗格可以同时显示各自的浮层。
                    self.note_drop_overlay_progress(tab_id, cx);
                }
                if applied && self.active_tab == Some(tab_id) && !self.immersive {
                    cx.notify();
                }
                return;
            }
            AppEvent::SftpFinished {
                tab_id,
                connection_id,
                op_id,
                transferred,
                skipped,
                verified,
            } => {
                let mut refresh = None;
                let mut refresh_side = None;
                let mut refresh_local = false;
                let mut notify_info: Option<(TransferDirection, String, u64)> = None;
                if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
                    let mut completed_transfer = false;
                    if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                        completed_transfer = true;
                        task.status = TransferStatus::Completed;
                        task.speed_bps = None;
                        task.eta_secs = None;
                        if let Some(bytes) = transferred {
                            task.transferred = bytes;
                        }
                        if task.direction == TransferDirection::Download
                            || task.direction == TransferDirection::Sync
                        {
                            refresh_local = true;
                        }
                        if task.direction == TransferDirection::RemoteCopy
                            && let (Some(target_conn), Some(target_path)) =
                                (task.target_connection_id, &task.target_remote_path)
                        {
                            let dir = target_path
                                .rsplit_once('/')
                                .map(|(d, _)| if d.is_empty() { "/" } else { d })
                                .unwrap_or("/");
                            refresh_side = Some((target_conn, dir.to_string()));
                        }
                        let file_name = if task.direction == TransferDirection::RemoteCopy {
                            task.target_remote_path
                                .clone()
                                .unwrap_or_else(|| task.remote_path.clone())
                        } else {
                            task.local_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| task.remote_path.clone())
                        };
                        notify_info = Some((task.direction, file_name, task.transferred));
                    }
                    if completed_transfer {
                        panel.active_transfers.remove(&op_id);
                        panel.recompute_progress();
                        if panel.active_transfers.is_empty() && panel.active_op.is_none() {
                            let base = match transferred {
                                Some(bytes) => format!(
                                    "{} ({})",
                                    rust_i18n::t!("sftp.transfer_done"),
                                    format_size(bytes)
                                ),
                                None => rust_i18n::t!("sftp.operation_done").to_string(),
                            };
                            let mut message = base;
                            if skipped > 0 {
                                message.push_str(&format!(
                                    " | {}: {skipped}",
                                    rust_i18n::t!("sftp.skipped_items")
                                ));
                            }
                            if verified {
                                message.push_str(&format!(
                                    " | {}",
                                    rust_i18n::t!("sftp.sha256_verified")
                                ));
                            }
                            Self::set_sftp_transient_status(panel, tab_id, message, cx);
                            panel.error = None;
                            refresh = Some(panel.path.clone());
                        }
                    } else if panel.active_op == Some(op_id) {
                        panel.active_op = None;
                        let base = match transferred {
                            Some(bytes) => format!(
                                "{} ({})",
                                rust_i18n::t!("sftp.transfer_done"),
                                format_size(bytes)
                            ),
                            None => rust_i18n::t!("sftp.operation_done").to_string(),
                        };
                        Self::set_sftp_transient_status(panel, tab_id, base, cx);
                        refresh = Some(panel.path.clone());
                    }
                }
                // 任务落定，浮层据此决定继续常驻还是进入 5 秒停留窗口。
                self.sync_drop_overlay(tab_id, cx);
                if let Some((dir, file_name, bytes)) = notify_info {
                    let action = match dir {
                        TransferDirection::Upload => "上传完成",
                        TransferDirection::Download => "下载完成",
                        TransferDirection::Sync => "同步完成",
                        TransferDirection::RemoteCopy => "跨远端传输完成",
                    };
                    send_desktop_notification(
                        &format!("Transom SFTP {action}"),
                        &format!("{file_name} ({})", format_size(bytes)),
                    );
                }
                if refresh_local {
                    self.sftp_local_refresh(tab_id, cx);
                }
                if let Some(path) = refresh {
                    self.request_sftp_list(tab_id, path, cx);
                }
                if let Some((target_conn, target_dir)) = refresh_side {
                    self.request_sftp_list_for_connection(tab_id, target_conn, target_dir, cx);
                }
                self.process_sftp_queue(tab_id, cx);
            }
            AppEvent::SftpMetadata {
                tab_id,
                connection_id,
                op_id,
                metadata,
            } => {
                let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) else {
                    // 标签页已关闭（或连接已切换）：忽略迟到的属性响应，不再弹出属性窗
                    return;
                };
                if panel.active_op == Some(op_id) {
                    panel.active_op = None;
                }
                self.sftp_modals.properties = Some(SftpPropertiesModalState {
                    scope: "远端".to_string(),
                    properties: remote_file_properties(&metadata),
                });
                self.process_sftp_queue(tab_id, cx);
            }
            AppEvent::SftpPreview {
                tab_id,
                connection_id,
                op_id,
                bytes,
                truncated,
            } => {
                self.handle_sftp_preview(
                    SftpPreviewResponse {
                        tab_id,
                        connection_id,
                        op_id,
                        result: Ok((bytes, truncated)),
                    },
                    cx,
                );
            }
            AppEvent::SftpPreviewFailed {
                tab_id,
                connection_id,
                op_id,
                error,
            } => {
                self.handle_sftp_preview(
                    SftpPreviewResponse {
                        tab_id,
                        connection_id,
                        op_id,
                        result: Err(error),
                    },
                    cx,
                );
            }
            AppEvent::SftpTextRead {
                op_id,
                content,
                meta,
                ..
            } => {
                self.apply_editor_text(op_id, content, meta, window, cx);
            }
            AppEvent::SftpTextWritten { op_id, meta, .. } => {
                self.note_editor_text_written(op_id, meta, window, cx);
            }
            AppEvent::SftpTextFailed { op_id, error, .. } => {
                // 编辑读写失败只回到面板内显示，不写远端栏错误、不进传输队列统计。
                self.note_editor_text_failed(op_id, localized_error(&error), cx);
            }
            AppEvent::SftpTextConflict { op_id, error, .. } => {
                // 版本指纹拦截不是普通失败：面板要弹出三选一冲突横幅。
                self.note_editor_text_conflict(op_id, localized_error(&error), cx);
            }
            AppEvent::SftpDirCompared {
                tab_id: _,
                connection_id,
                op_id,
                comparison,
            } => {
                if let Some(modal) = self.sftp_modals.sync.as_mut()
                    && modal.op_id == op_id
                    && modal.connection_id == connection_id
                {
                    modal.scanning = false;
                    modal.skipped = comparison.skipped;
                    modal.selected = comparison
                        .entries
                        .iter()
                        .map(|entry| entry.sync_item().is_some())
                        .collect();
                    modal.entries = comparison.entries;
                    modal.error = None;
                }
            }
            AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error,
            } => {
                if let Some(modal) = self.sftp_modals.sync.as_mut()
                    && modal.op_id == op_id
                    && modal.connection_id == connection_id
                {
                    modal.scanning = false;
                    modal.error = Some(localized_error(&error));
                    cx.notify();
                    return;
                }
                let mut failed_notify: Option<(String, String)> = None;
                if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
                    let err_msg = localized_error(&error);
                    let mut failed_transfer = false;
                    if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                        failed_transfer = true;
                        task.status = TransferStatus::Failed(err_msg.clone());
                        let file_name = task
                            .local_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| task.remote_path.clone());
                        failed_notify = Some((file_name, err_msg.clone()));
                    }
                    if failed_transfer {
                        panel.active_transfers.remove(&op_id);
                        panel.recompute_progress();
                        panel.error = Some(err_msg);
                    } else if panel.active_op == Some(op_id) || panel.active_op.is_none() {
                        panel.active_op = None;
                        panel.loading = false;
                        panel.error = Some(err_msg);
                    }
                }
                self.sync_drop_overlay(tab_id, cx);
                if let Some((file_name, err_msg)) = failed_notify {
                    send_desktop_notification(
                        "Transom SFTP 传输失败",
                        &format!("{file_name}: {err_msg}"),
                    );
                }
                self.process_sftp_queue(tab_id, cx);
            }
            AppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    if let Some(error) = &status.error {
                        self.error = Some(localized_error(error));
                    }
                    tab.forward_statuses.insert(status.rule_id, status);
                }
            }
            AppEvent::SystemStatsUpdated {
                tab_id,
                connection_id,
                stats,
            } => {
                let mut applied = false;
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    tab.system_stats = Some(stats);
                    applied = true;
                }
                // 周期采样事件只在数据真的属于可见标签时才重绘底栏。
                if applied && self.active_tab == Some(tab_id) && !self.immersive {
                    cx.notify();
                }
                return;
            }
            AppEvent::NetworkTrafficUpdated {
                tab_id,
                connection_id,
                traffic,
            } => {
                let mut applied = false;
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    tab.network_traffic = Some(traffic);
                    applied = true;
                }
                if applied && self.active_tab == Some(tab_id) && !self.immersive {
                    cx.notify();
                }
                return;
            }
            AppEvent::PublicKeyDeployed {
                tab_id: _,
                connection_id: _,
                key_name,
                error,
            } => {
                self.handle_public_key_deployed(key_name, error, cx);
            }
            AppEvent::CloudSyncPushed { result } => {
                self.handle_cloud_sync_pushed(result, cx);
                return;
            }
            AppEvent::Zmodem {
                tab_id,
                connection_id: _,
                event,
            } => {
                self.handle_zmodem_event(tab_id, event, cx);
                return;
            }
            AppEvent::CloudSyncPulled { result } => {
                match result {
                    Ok(document) => self.finish_cloud_sync_pull(document, window, cx),
                    Err(error) if error.code == transom_core::ErrorCode::CloudSyncMissing => {
                        self.handle_cloud_sync_missing(cx);
                    }
                    Err(error) => {
                        self.error = Some(localized_error(&error));
                    }
                }
                return;
            }
            AppEvent::HostActionResult {
                session_id,
                outcome,
            } => {
                self.handle_probe_result(session_id, outcome, cx);
                return;
            }
            AppEvent::DeepLink { request } => {
                self.handle_deep_link(request, window, cx);
                return;
            }
        }
        cx.notify();
    }

    pub(crate) fn handle_terminal_event(
        &mut self,
        tab_id: TabId,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalEvent::Title(title) => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.title = title.clone();
                }
                self.sftp_sync_with_terminal_title(tab_id, title, cx);
            }
            TerminalEvent::ResetTitle => {}
            TerminalEvent::Bell => {
                self.status = Some(rust_i18n::t!("status.bell").to_string());
            }
            TerminalEvent::ClipboardStored => {
                self.status = Some(rust_i18n::t!("status.copied").to_string());
            }
            TerminalEvent::Exit => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && !matches!(tab.status, TabStatus::Failed(_))
                {
                    tab.status = TabStatus::Disconnected;
                }
            }
            TerminalEvent::ContextMenuRequested { .. } => {}
            TerminalEvent::CommandRecorded { command } => {
                self.record_history_command(command, cx);
            }
            TerminalEvent::FilesDropped(paths) => {
                self.handle_dropped_files(tab_id, paths, window, cx);
            }
            TerminalEvent::CommandFinished {
                exit_code,
                duration_ms,
            } => {
                self.on_command_finished(tab_id, *exit_code, *duration_ms, window, cx);
            }
            TerminalEvent::DesktopNotification { title, body } => {
                self.on_desktop_notification(title, body, cx);
            }
        }
        cx.notify();
    }
}
