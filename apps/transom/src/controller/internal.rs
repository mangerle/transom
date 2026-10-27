//! 内部完成事件的处理：连接接入、SFTP/转发提交与断开通知。
//!
//! 设计原理（Why）：四类连接（SSH/本地/串口/Telnet）的接入此前各占一段
//! 约五十行的复制代码，差异只在"远端句柄、断开说明与本地共享订阅"三处；
//! 收敛为 [`handle_connected`] 里按 [`ReadyConnection`] 分支的单一接入
//! 路径后，新增连接类型只需扩展枚举与差异化的收尾步骤。

use super::Internal;
use super::ReadyConnection;
use super::lifecycle::{
    Admission, AdoptConnection, TabCleanupTables, admit_connection, adopt_connection,
    drop_local_stats, stop_forwards, stop_key_deploy, stop_stats, stop_traffic, stop_zmodem,
};
use super::samplers::{
    run_local_stats_sampler, run_local_traffic_sampler, spawn_remote_stats_sampler,
    spawn_traffic_sampler,
};
use super::sftp_exec;
use super::state::{ControllerCtx, ControllerState};
use transom_core::{AppEvent, ConnectionId, ForwardState, ForwardStatus, TabId, TerminalIo};
use transom_net::{SshRemote, SshSession, ZmodemConfig};

/// 分发一条内部完成事件。
pub(super) async fn handle_internal(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    match internal {
        Internal::Connected { .. } => handle_connected(state, ctx, internal).await,
        Internal::Disconnected {
            tab_id,
            connection_id,
        } => handle_disconnected(state, ctx, tab_id, connection_id).await,
        Internal::SftpReady { .. } => handle_sftp_ready(state, ctx, internal).await,
        Internal::ForwardReady { .. } => handle_forward_ready(state, ctx, internal).await,
    }
}

/// 连接接入：先按连接意图裁决，再走统一的落地路径。
///
/// 连接形态与 ZMODEM 配置随事件一起到达（配置由界面随命令携带），
/// 控制器不再为每次连接同步读一次设置文件。
async fn handle_connected(state: &mut ControllerState, ctx: &ControllerCtx, internal: Internal) {
    let Internal::Connected {
        tab_id,
        connection_id,
        kind,
        zmodem,
    } = internal
    else {
        // 调用方按变体精确调用，其它形态直接忽略（不可达路径）。
        return;
    };
    if matches!(
        admit_connection(&state.connecting, tab_id, connection_id),
        Admission::Reject
    ) {
        // 迟到或已作废的连接结果：释放资源、不触碰任何状态表，
        // 避免踢掉已接入的新连接。
        log::debug!(
            "拒收过期的连接完成结果并释放，窗格: {tab_id:?}，连接: {connection_id:?}，来源: {}",
            kind.label()
        );
        reject_connection(state, kind);
        return;
    }
    state.connecting.remove(&tab_id);

    match kind {
        ReadyConnection::Ssh(session) => {
            let SshSession {
                io,
                join,
                remote,
                traffic,
            } = session;
            let remote_clone = remote.clone();
            adopt_stream_connection(
                state,
                ctx,
                StreamConnection {
                    tab_id,
                    connection_id,
                    io,
                    join: async move {
                        let _ = join.await;
                    },
                    remote: Some(remote),
                    disconnect_reason: "reconnected",
                    zmodem,
                },
            )
            .await;

            let traffic_task =
                spawn_traffic_sampler(traffic, tab_id, connection_id, ctx.event_tx.clone());
            state.traffic_tasks.insert(tab_id, traffic_task);

            let stats_task = spawn_remote_stats_sampler(
                remote_clone,
                tab_id,
                connection_id,
                ctx.event_tx.clone(),
            );
            state.stats_tasks.insert(tab_id, stats_task);
        }
        ReadyConnection::Local { io, join } => {
            adopt_stream_connection(
                state,
                ctx,
                StreamConnection {
                    tab_id,
                    connection_id,
                    io,
                    join: async move {
                        let _ = join.await;
                    },
                    remote: None,
                    disconnect_reason: "reconnected to local",
                    zmodem,
                },
            )
            .await;
            register_local_shared_samplers(state, ctx, tab_id, connection_id);
        }
        ReadyConnection::Serial { io, join } => {
            adopt_stream_connection(
                state,
                ctx,
                StreamConnection {
                    tab_id,
                    connection_id,
                    io,
                    join: async move {
                        let _ = join.await;
                    },
                    remote: None,
                    disconnect_reason: "reconnected to serial",
                    zmodem,
                },
            )
            .await;
        }
        ReadyConnection::Telnet { io, join } => {
            adopt_stream_connection(
                state,
                ctx,
                StreamConnection {
                    tab_id,
                    connection_id,
                    io,
                    join: async move {
                        let _ = join.await;
                    },
                    remote: None,
                    disconnect_reason: "reconnected to telnet",
                    zmodem,
                },
            )
            .await;
        }
    }
}

/// 拒收过期连接：SSH 主动断开释放（服务器侧不必等 keepalive 超时），
/// 其余类型丢弃 IO 即关闭 PTY/串口/TCP 流，退出通知因连接不再现任被忽略。
fn reject_connection(state: &mut ControllerState, kind: ReadyConnection) {
    match kind {
        ReadyConnection::Ssh(session) => {
            state.cleanup_set.spawn(async move {
                session
                    .remote
                    .disconnect("superseded by newer connection")
                    .await;
            });
        }
        ReadyConnection::Local { .. }
        | ReadyConnection::Serial { .. }
        | ReadyConnection::Telnet { .. } => {}
    }
}

/// 三类"字节流"连接的接入描述：参数超过五个后收敛为结构体。
struct StreamConnection<J> {
    tab_id: TabId,
    connection_id: ConnectionId,
    io: TerminalIo,
    join: J,
    /// SSH 连接的新远端句柄；本地/串口/Telnet 为 None，只清理旧远端。
    remote: Option<SshRemote>,
    /// 顶掉旧连接时的断开说明，用于区分"SSH 重连"与"切回本地/串口"。
    disconnect_reason: &'static str,
    /// ZMODEM 监督层配置（随连接命令携带，控制器不回读设置文件）。
    zmodem: ZmodemConfig,
}

/// 三类"字节流"连接（含 SSH）的公共落地：登记当前连接、停旧任务、
/// 广播 TerminalReady 并挂接退出监听；差异只在远端句柄与断开说明。
async fn adopt_stream_connection<J>(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    setup: StreamConnection<J>,
) where
    J: std::future::Future<Output = ()> + Send + 'static,
{
    let StreamConnection {
        tab_id,
        connection_id,
        io,
        join,
        remote,
        disconnect_reason,
        zmodem,
    } = setup;
    adopt_connection(
        TabCleanupTables {
            current_connections: &mut state.current_connections,
            forwards: &mut state.forwards,
            traffic_tasks: &mut state.traffic_tasks,
            key_deploy_tasks: &mut state.key_deploy_tasks,
            stats_tasks: &mut state.stats_tasks,
            sftp_clients: &mut state.sftp_clients,
            remotes: &mut state.remotes,
            cleanup: &mut state.cleanup_set,
            zmodem_tasks: &mut state.zmodem_tasks,
            zmodem_controls: &mut state.zmodem_controls,
        },
        AdoptConnection {
            tab_id,
            connection_id,
            io,
            join,
            remote,
            disconnect_reason,
            zmodem_config: zmodem,
        },
        &ctx.event_tx,
        &ctx.internal_tx,
    )
    .await;
}

/// 登记本地终端的共享订阅：系统统计与网络流量都是机器级属性，各跑一份
/// 采集（多标签各自采样会在本地终端一多时变成每秒 N 份 /proc 读取与
/// N 个 df 子进程）；首个订阅者拉起采样器，标签清空后采样器自行退出。
fn register_local_shared_samplers(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
) {
    if let Ok(mut tabs) = state.local_stats_tabs.lock() {
        tabs.retain(|(tab, _)| *tab != tab_id);
        tabs.push((tab_id, connection_id));
    }
    if state
        .local_stats_task
        .as_ref()
        .is_none_or(|task| task.is_finished())
    {
        state.local_stats_task = Some(tokio::spawn(run_local_stats_sampler(
            ctx.event_tx.clone(),
            state.local_stats_tabs.clone(),
        )));
    }
    if let Ok(mut tabs) = state.local_traffic_tabs.lock() {
        tabs.retain(|(tab, _)| *tab != tab_id);
        tabs.push((tab_id, connection_id));
    }
    if state
        .local_traffic_task
        .as_ref()
        .is_none_or(|task| task.is_finished())
    {
        state.local_traffic_task = Some(tokio::spawn(run_local_traffic_sampler(
            ctx.event_tx.clone(),
            state.local_traffic_tabs.clone(),
        )));
    }
}

/// 断开通知：仅当前任连接生效时才清理全部附属任务与远端句柄。
async fn handle_disconnected(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    connection_id: ConnectionId,
) {
    let is_current = state.current_connections.get(&tab_id) == Some(&connection_id);
    if !is_current {
        return;
    }
    state.current_connections.remove(&tab_id);
    stop_forwards(&mut state.forwards, &mut state.cleanup_set, tab_id);
    stop_traffic(&mut state.traffic_tasks, tab_id);
    stop_key_deploy(&mut state.key_deploy_tasks, tab_id);
    stop_stats(&mut state.stats_tasks, tab_id);
    drop_local_stats(&state.local_stats_tabs, tab_id);
    drop_local_stats(&state.local_traffic_tabs, tab_id);
    stop_zmodem(&mut state.zmodem_tasks, &mut state.zmodem_controls, tab_id);
    // 优雅断开远端：直接丢弃 Handle 会跳过 disconnect 通知，
    // 服务器侧要等 keepalive 超时才回收会话。
    if let Some((_, remote)) = state.remotes.remove(&tab_id) {
        state.cleanup_set.spawn(async move {
            remote.disconnect("connection lost").await;
        });
    }
    state.sftp_clients.remove(&tab_id);
    state.sftp_tasks.retain(|(t, _), handle| {
        if *t == tab_id {
            handle.abort();
            false
        } else {
            true
        }
    });
    let _ = ctx
        .event_tx
        .send_async(AppEvent::Disconnected {
            tab_id,
            connection_id,
        })
        .await;
}

/// SFTP 客户端就绪：校验连接归属后写入缓存并派发操作。
async fn handle_sftp_ready(state: &mut ControllerState, ctx: &ControllerCtx, internal: Internal) {
    let Internal::SftpReady {
        tab_id,
        connection_id,
        op_id,
        op,
        result,
    } = internal
    else {
        // 调用方按变体精确调用，其它形态直接忽略（不可达路径）。
        return;
    };
    // 提交前校验归属：只要该连接仍在活跃连接表中即合法。
    let valid_owner = state
        .current_connections
        .iter()
        .find(|(_, c)| **c == connection_id)
        .map(|(t, _)| *t);
    let Some(owner_tab) = valid_owner else {
        drop(result);
        return;
    };
    match result {
        Ok(client) => {
            state.sftp_clients.insert(owner_tab, client.clone());
            sftp_exec::spawn_sftp_op(
                &mut state.sftp_tasks,
                ctx.event_tx.clone(),
                tab_id,
                connection_id,
                op_id,
                client,
                op,
            );
        }
        Err(error) => {
            let _ = ctx
                .event_tx
                .send_async(AppEvent::SftpFailed {
                    tab_id,
                    connection_id,
                    op_id,
                    error,
                })
                .await;
        }
    }
}

/// 端口转发建立完成：校验连接归属后登记任务并向界面回报状态。
async fn handle_forward_ready(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    let Internal::ForwardReady {
        tab_id,
        connection_id,
        rule_id,
        result,
    } = internal
    else {
        // 调用方按变体精确调用，其它形态直接忽略（不可达路径）。
        return;
    };
    let is_current = state
        .remotes
        .get(&tab_id)
        .is_some_and(|(current, _)| *current == connection_id);
    let status = match (is_current, result) {
        (false, Ok(task)) => {
            // 连接已失效：立即停止刚建立的转发并释放。
            state.cleanup_set.spawn(async move {
                task.stop().await;
            });
            ForwardStatus {
                rule_id,
                state: ForwardState::Stopped,
                error: None,
            }
        }
        (false, Err(_)) => return,
        (true, Ok(task)) => {
            let bound_port = task.bound_port;
            state
                .forwards
                .entry(tab_id)
                .or_default()
                .insert(rule_id, task);
            ForwardStatus {
                rule_id,
                state: ForwardState::Active { bound_port },
                error: None,
            }
        }
        (true, Err(error)) => ForwardStatus {
            rule_id,
            state: ForwardState::Failed,
            error: Some(error),
        },
    };
    let _ = ctx
        .event_tx
        .send_async(AppEvent::ForwardChanged {
            tab_id,
            connection_id,
            status,
        })
        .await;
}
