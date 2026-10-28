//! 用户命令的分发与处理。
//!
//! 设计原理（Why）：命令处理此前内联在控制循环的巨型 match 里（单函数
//! 超过七百行，四条命令分支之外还混着状态表管理）；拆到本模块后，主循环
//! 只负责"收事件、委托处理、判定是否停机"，新增命令不再触碰循环本体。

use super::Internal;
use super::host_exec::run_batch;
use super::lifecycle::{
    ConnectingTab, KeyDeployParams, ShutdownTables, deploy_key, drop_local_stats, shutdown_all,
};
use super::pty::{ConnectDispatch, dispatch_connect};
use super::samplers;
use super::sftp_exec::{SftpDispatch, dispatch_sftp_op};
use super::state::{ControllerCtx, ControllerState};
use super::sync_exec::{CloudSyncJob, dispatch_cloud_sync};
use transom_core::{
    AppCommand, AppError, ConnectParams, ConnectionId, ErrorCode, ForwardAppEvent, ForwardCommand,
    ForwardId, ForwardRule, ForwardState, ForwardStatus, HostCommand, SftpCommand, SyncCommand,
    TabId, TerminalCommand, ZmodemCommand,
};
use transom_net::start_forward;
use transom_zmodem::ZmodemConfig;

/// 主循环的继续/停机判定。
pub(super) enum LoopControl {
    /// 继续处理下一条事件。
    Continue,
    /// 收到停机命令或通道关闭：执行收尾后退出主循环。
    Shutdown,
}

/// 分发一条用户命令。
pub(super) async fn handle_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: AppCommand,
) -> LoopControl {
    match command {
        AppCommand::Terminal(cmd) => handle_terminal_command(state, ctx, cmd).await,
        AppCommand::Sftp(cmd) => {
            handle_sftp_command(state, ctx, cmd);
            LoopControl::Continue
        }
        AppCommand::Forward(cmd) => {
            handle_forward_command(state, ctx, cmd).await;
            LoopControl::Continue
        }
        AppCommand::Host(cmd) => {
            handle_host_command(state, ctx, cmd).await;
            LoopControl::Continue
        }
        AppCommand::Sync(cmd) => {
            handle_sync_command(state, ctx, cmd).await;
            LoopControl::Continue
        }
        AppCommand::Zmodem(cmd) => {
            handle_zmodem_command(state, cmd);
            LoopControl::Continue
        }
    }
}

async fn handle_terminal_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: TerminalCommand,
) -> LoopControl {
    match command {
        TerminalCommand::Connect(params) => {
            let ConnectParams {
                tab_id,
                connection_id,
                session,
                jumps,
                proxy,
                size,
                secrets,
                zmodem,
                allow_sha1_publickey_auth,
            } = *params;
            start_connect(
                state,
                ConnectDispatch {
                    known_hosts: ctx.known_hosts.clone(),
                    event_tx: ctx.event_tx.clone(),
                    internal_tx: ctx.internal_tx.clone(),
                    tab_id,
                    connection_id,
                    session: Box::new(session),
                    jumps,
                    proxy,
                    size,
                    secrets,
                    zmodem: ZmodemConfig {
                        download_dir: zmodem.resolved_download_dir(),
                        auto_receive: zmodem.auto_receive,
                    },
                    allow_sha1_publickey_auth,
                },
            );
            LoopControl::Continue
        }
        TerminalCommand::CloseTab { tab_id } => {
            close_tab(state, tab_id);
            LoopControl::Continue
        }
        TerminalCommand::Shutdown => LoopControl::Shutdown,
    }
}

fn handle_sftp_command(state: &mut ControllerState, ctx: &ControllerCtx, command: SftpCommand) {
    match command {
        SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op,
        } => {
            dispatch_sftp_op(SftpDispatch {
                sessions: &mut state.sessions,
                sftp_tasks: &mut state.sftp_tasks,
                event_tx: ctx.event_tx.clone(),
                internal_tx: ctx.internal_tx.clone(),
                tab_id,
                connection_id,
                op_id,
                op,
            });
        }
        SftpCommand::SftpCancel { tab_id, op_id } => {
            if let Some(handle) = state.sftp_tasks.remove(&(tab_id, op_id)) {
                handle.abort();
            }
        }
    }
}

async fn handle_forward_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ForwardCommand,
) {
    match command {
        ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        } => {
            handle_forward_start(state, ctx, tab_id, connection_id, rule).await;
        }
        ForwardCommand::ForwardStop {
            tab_id,
            connection_id,
            rule_id,
        } => {
            handle_forward_stop(state, ctx, tab_id, connection_id, rule_id).await;
        }
    }
}

async fn handle_host_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: HostCommand,
) {
    match command {
        HostCommand::DeployPublicKey {
            tab_id,
            connection_id,
            key_name,
            public_key,
        } => {
            deploy_key(KeyDeployParams {
                sessions: &mut state.sessions,
                event_tx: &ctx.event_tx,
                tab_id,
                connection_id,
                key_name,
                public_key,
            })
            .await;
        }
        HostCommand::RunHostActions { actions } => {
            state.host_action_tasks.spawn(run_batch(
                actions,
                ctx.event_tx.clone(),
                ctx.action_permits.clone(),
            ));
        }
        HostCommand::SampleProcesses {
            tab_id,
            connection_id,
            sort,
        } => {
            state
                .monitor_set
                .spawn(samplers::run_process_sample(samplers::ProcessTarget {
                    remote: state.current_remote(tab_id, connection_id),
                    tab_id,
                    connection_id,
                    sort,
                    event_tx: ctx.event_tx.clone(),
                }));
        }
        HostCommand::KillProcess {
            tab_id,
            connection_id,
            pid,
            force,
        } => {
            state.monitor_set.spawn(samplers::run_process_kill(
                samplers::ProcessTarget {
                    remote: state.current_remote(tab_id, connection_id),
                    tab_id,
                    connection_id,
                    sort: transom_core::ProcessSort::default(),
                    event_tx: ctx.event_tx.clone(),
                },
                pid,
                force,
            ));
        }
    }
}

async fn handle_sync_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: SyncCommand,
) {
    match command {
        SyncCommand::CloudSyncPush { settings, catalog } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Push { settings, catalog },
            )
            .await;
        }
        SyncCommand::CloudSyncPull { settings } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Pull { settings },
            )
            .await;
        }
        SyncCommand::GitSync { settings, catalog } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Git { settings, catalog },
            )
            .await;
        }
    }
}

fn handle_zmodem_command(state: &mut ControllerState, command: ZmodemCommand) {
    match command {
        ZmodemCommand::ZmodemUpload { tab_id, files } => {
            let control = state
                .sessions
                .get(&tab_id)
                .and_then(|s| s.zmodem_control.clone());
            if let Some(control) = control
                && !control.upload(files)
            {
                log::warn!("ZMODEM 上传决策被丢弃，标签 {tab_id:?} 无等待中的传输会话");
            }
        }
        ZmodemCommand::ZmodemCancel { tab_id } => {
            let control = state
                .sessions
                .get(&tab_id)
                .and_then(|s| s.zmodem_control.clone());
            if let Some(control) = control {
                control.cancel();
            }
        }
    }
}

/// 发起连接：登记最新意图并派发建连任务。
///
/// 同标签的旧建连任务作废并取消，它迟到的完成结果会被接入裁决拒收，
/// 不再覆盖新连接。
fn start_connect(state: &mut ControllerState, dispatch: ConnectDispatch) {
    let tab_id = dispatch.tab_id;
    let connection_id = dispatch.connection_id;
    if let Some(previous) = state.connecting.remove(&tab_id) {
        previous.task.abort();
    }
    let task = dispatch_connect(dispatch);
    state.connecting.insert(
        tab_id,
        ConnectingTab {
            connection_id,
            task,
        },
    );
}

/// 启动端口转发：建立有多次网络往返，移入任务执行；结果经 ForwardReady
/// 回流提交，分发循环不再等待远端应答。
async fn handle_forward_start(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
    rule: ForwardRule,
) {
    let remote = state.current_remote(tab_id, connection_id);

    let Some(remote) = remote else {
        let _ = ctx
            .event_tx
            .send_async(
                ForwardAppEvent::ForwardChanged {
                    tab_id,
                    connection_id,
                    status: ForwardStatus {
                        rule_id: rule.id,
                        state: ForwardState::Failed,
                        error: Some(AppError::with_detail(
                            ErrorCode::ForwardFailed,
                            "SSH connection is not ready",
                        )),
                    },
                }
                .into(),
            )
            .await;
        return;
    };

    let internal_tx = ctx.internal_tx.clone();
    state.cleanup_set.spawn(async move {
        let result = start_forward(&remote, &rule).await;
        let _ = internal_tx
            .send_async(Internal::ForwardReady {
                tab_id,
                connection_id,
                rule_id: rule.id,
                result,
            })
            .await;
    });
}

/// 停止端口转发：停止动作纳入统一收尾集合，停机时限时等待。
async fn handle_forward_stop(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
    rule_id: ForwardId,
) {
    let Some(session) = state.sessions.get_mut(&tab_id) else {
        return;
    };
    if session.connection_id != connection_id {
        return;
    }
    if let Some(task) = session.forwards.remove(&rule_id) {
        state.cleanup_set.spawn(async move {
            task.stop().await;
        });
    }
    let _ = ctx
        .event_tx
        .send_async(
            ForwardAppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status: ForwardStatus {
                    rule_id,
                    state: ForwardState::Stopped,
                    error: None,
                },
            }
            .into(),
        )
        .await;
}

/// 关闭标签：注销连接意图与全部附属任务，优雅断开远端。
fn close_tab(state: &mut ControllerState, tab_id: TabId) {
    // 关闭即注销连接意图并取消建连任务：DNS/握手/认证等待中的连接随任务
    // 中止，迟到的完成结果被接入裁决拒收。
    if let Some(entry) = state.connecting.remove(&tab_id) {
        entry.task.abort();
    }
    drop_local_stats(&state.local_traffic_tabs, tab_id);
    drop_local_stats(&state.local_stats_tabs, tab_id);
    state.terminate_session(tab_id, "tab closed");
    state.sftp_tasks.retain(|(t, _), handle| {
        if *t == tab_id {
            handle.abort();
            false
        } else {
            true
        }
    });
}

/// 停机收尾：中止在途云同步、停全部后台任务、断开全部远端并限时等待。
///
/// 收尾完成后通过 `shutdown_tx` 通知 `Controller::shutdown`；提前发送会
/// 让远端断开会话随运行时关闭被取消。
pub(super) async fn shutdown(
    state: &mut ControllerState,
    shutdown_tx: &std::sync::mpsc::Sender<()>,
) {
    // 在途云同步没有可续传的事务，停机即中止。
    if let Some(task) = state.sync_task.take() {
        task.abort();
    }
    shutdown_all(ShutdownTables {
        connecting: &mut state.connecting,
        sessions: &mut state.sessions,
        sftp_tasks: &mut state.sftp_tasks,
        cleanup: &mut state.cleanup_set,
        local_stats_task: &mut state.local_stats_task,
        local_traffic_task: &mut state.local_traffic_task,
    })
    .await;
    state.monitor_set.shutdown().await;
    let _ = shutdown_tx.send(());
}
