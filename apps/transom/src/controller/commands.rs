//! 用户命令的分发与处理。
//!
//! 设计原理（Why）：命令处理此前内联在控制循环的巨型 match 里（单函数
//! 超过七百行，四条命令分支之外还混着状态表管理）；拆到本模块后，主循环
//! 只负责"收事件、委托处理、判定是否停机"，新增命令不再触碰循环本体。

use super::Internal;
use super::host_exec::run_batch;
use super::lifecycle::{
    ConnectingTab, KeyDeployParams, ShutdownTables, deploy_key, drop_local_stats, shutdown_all,
    stop_forwards, stop_key_deploy, stop_stats, stop_traffic,
};
use super::pty::{ConnectDispatch, dispatch_connect};
use super::samplers;
use super::sftp_exec::{SftpDispatch, dispatch_sftp_op};
use super::state::{ControllerCtx, ControllerState};
use super::sync_exec::{CloudSyncJob, dispatch_cloud_sync};
use transom_core::{
    AppCommand, AppError, ConnectionId, ErrorCode, ForwardAppEvent, ForwardId, ForwardRule,
    ForwardState, ForwardStatus, TabId,
};
use transom_net::{SshRemote, ZmodemConfig, start_forward};

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
    command: transom_core::TerminalCommand,
) -> LoopControl {
    match command {
        transom_core::TerminalCommand::Connect(params) => {
            let transom_core::ConnectParams {
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
        transom_core::TerminalCommand::CloseTab { tab_id } => {
            close_tab(state, tab_id);
            LoopControl::Continue
        }
        transom_core::TerminalCommand::Shutdown => LoopControl::Shutdown,
    }
}

fn handle_sftp_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: transom_core::SftpCommand,
) {
    match command {
        transom_core::SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op,
        } => {
            dispatch_sftp_op(SftpDispatch {
                remotes: &state.remotes,
                sftp_clients: &mut state.sftp_clients,
                sftp_tasks: &mut state.sftp_tasks,
                event_tx: ctx.event_tx.clone(),
                internal_tx: ctx.internal_tx.clone(),
                tab_id,
                connection_id,
                op_id,
                op,
            });
        }
        transom_core::SftpCommand::SftpCancel { tab_id, op_id } => {
            if let Some(handle) = state.sftp_tasks.remove(&(tab_id, op_id)) {
                handle.abort();
            }
        }
    }
}

async fn handle_forward_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: transom_core::ForwardCommand,
) {
    match command {
        transom_core::ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        } => {
            handle_forward_start(state, ctx, tab_id, connection_id, rule).await;
        }
        transom_core::ForwardCommand::ForwardStop {
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
    command: transom_core::HostCommand,
) {
    match command {
        transom_core::HostCommand::DeployPublicKey {
            tab_id,
            connection_id,
            key_name,
            public_key,
        } => {
            deploy_key(KeyDeployParams {
                remotes: &mut state.remotes,
                key_deploy_tasks: &mut state.key_deploy_tasks,
                event_tx: &ctx.event_tx,
                tab_id,
                connection_id,
                key_name,
                public_key,
            })
            .await;
        }
        transom_core::HostCommand::RunHostActions { actions } => {
            state.host_action_tasks.spawn(run_batch(
                actions,
                ctx.event_tx.clone(),
                ctx.action_permits.clone(),
            ));
        }
        transom_core::HostCommand::SampleProcesses {
            tab_id,
            connection_id,
        } => {
            state
                .monitor_set
                .spawn(samplers::run_process_sample(samplers::ProcessTarget {
                    remote: current_remote(state, tab_id, connection_id),
                    tab_id,
                    connection_id,
                    event_tx: ctx.event_tx.clone(),
                }));
        }
        transom_core::HostCommand::KillProcess {
            tab_id,
            connection_id,
            pid,
            force,
        } => {
            state.monitor_set.spawn(samplers::run_process_kill(
                samplers::ProcessTarget {
                    remote: current_remote(state, tab_id, connection_id),
                    tab_id,
                    connection_id,
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
    command: transom_core::SyncCommand,
) {
    match command {
        transom_core::SyncCommand::CloudSyncPush { settings, catalog } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Push { settings, catalog },
            )
            .await;
        }
        transom_core::SyncCommand::CloudSyncPull { settings } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Pull { settings },
            )
            .await;
        }
        transom_core::SyncCommand::GitSync { settings, catalog } => {
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

fn handle_zmodem_command(state: &mut ControllerState, command: transom_core::ZmodemCommand) {
    match command {
        transom_core::ZmodemCommand::ZmodemUpload { tab_id, files } => {
            if let Some(control) = state.zmodem_controls.get(&tab_id)
                && !control.upload(files)
            {
                log::warn!("ZMODEM 上传决策被丢弃，标签 {tab_id:?} 无等待中的传输会话");
            }
        }
        transom_core::ZmodemCommand::ZmodemCancel { tab_id } => {
            if let Some(control) = state.zmodem_controls.get(&tab_id) {
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
    let Some((current_connection, remote)) = state.remotes.get(&tab_id).cloned() else {
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
    if current_connection != connection_id {
        return;
    }
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
    let Some((current_connection, _)) = state.remotes.get(&tab_id) else {
        return;
    };
    if *current_connection != connection_id {
        return;
    }
    if let Some(tasks) = state.forwards.get_mut(&tab_id)
        && let Some(task) = tasks.remove(&rule_id)
    {
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
    state.current_connections.remove(&tab_id);
    // 关闭即注销连接意图并取消建连任务：DNS/握手/认证等待中的连接随任务
    // 中止，迟到的完成结果被接入裁决拒收。
    if let Some(entry) = state.connecting.remove(&tab_id) {
        entry.task.abort();
    }
    drop_local_stats(&state.local_traffic_tabs, tab_id);
    stop_forwards(&mut state.forwards, &mut state.cleanup_set, tab_id);
    stop_traffic(&mut state.traffic_tasks, tab_id);
    stop_key_deploy(&mut state.key_deploy_tasks, tab_id);
    stop_stats(&mut state.stats_tasks, tab_id);
    drop_local_stats(&state.local_stats_tabs, tab_id);
    state.sftp_clients.remove(&tab_id);
    state.sftp_tasks.retain(|(t, _), handle| {
        if *t == tab_id {
            handle.abort();
            false
        } else {
            true
        }
    });
    if let Some((_, remote)) = state.remotes.remove(&tab_id) {
        state.cleanup_set.spawn(async move {
            remote.disconnect("tab closed").await;
        });
    }
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
        forwards: &mut state.forwards,
        traffic_tasks: &mut state.traffic_tasks,
        key_deploy_tasks: &mut state.key_deploy_tasks,
        stats_tasks: &mut state.stats_tasks,
        sftp_tasks: &mut state.sftp_tasks,
        remotes: &mut state.remotes,
        cleanup: &mut state.cleanup_set,
        local_stats_task: &mut state.local_stats_task,
        local_traffic_task: &mut state.local_traffic_task,
        zmodem_tasks: &mut state.zmodem_tasks,
    })
    .await;
    state.monitor_set.shutdown().await;
    let _ = shutdown_tx.send(());
}

/// 取标签当前有效的远端句柄：连接已切换或不存在时返回 `None`。
fn current_remote(
    state: &ControllerState,
    tab_id: TabId,
    connection_id: ConnectionId,
) -> Option<SshRemote> {
    state
        .remotes
        .get(&tab_id)
        .filter(|(current, _)| *current == connection_id)
        .map(|(_, remote)| remote.clone())
}
