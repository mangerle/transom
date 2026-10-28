//! 内部完成事件的处理：连接接入、SFTP/转发提交与断开通知。
//!
//! 设计原理（Why）：四类连接（SSH/本地/串口/Telnet）的接入此前各占一段
//! 约五十行的复制代码，差异只在"远端句柄、断开说明与本地共享订阅"三处；
//! 收敛为 [`handle_connected`] 里按 [`ReadyConnection`] 分支的单一接入
//! 路径后，新增连接类型只需扩展枚举与差异化的收尾步骤。

use super::Internal;
use super::ReadyConnection;
use super::lifecycle::{
    Admission, AdoptConnection, admit_connection, adopt_connection, drop_local_stats,
};
use super::samplers::{
    run_local_stats_sampler, run_local_traffic_sampler, spawn_remote_stats_sampler,
    spawn_traffic_sampler,
};
use super::sftp_exec;
use super::state::{ControllerCtx, ControllerState};
use transom_core::{
    ConnectionId, ForwardAppEvent, ForwardState, ForwardStatus, SftpAppEvent, TabId,
    TerminalAppEvent, TerminalIo,
};
use transom_net::{SshRemote, SshSession};
use transom_zmodem::ZmodemConfig;

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

    adopt_ready_connection(
        state,
        ctx,
        ConnectedAdoption {
            tab_id,
            connection_id,
            kind,
            zmodem,
        },
    )
    .await;
}

/// 已通过接入裁决的连接：统一交给按类型分派的落地路径。
struct ConnectedAdoption {
    tab_id: TabId,
    connection_id: ConnectionId,
    kind: ReadyConnection,
    zmodem: ZmodemConfig,
}

/// 按连接类型落地：SSH 追加采样器，本地追加共享统计订阅。
async fn adopt_ready_connection(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    adoption: ConnectedAdoption,
) {
    let ConnectedAdoption {
        tab_id,
        connection_id,
        kind,
        zmodem,
    } = adoption;
    match kind {
        ReadyConnection::Ssh(session) => {
            adopt_ssh_connection(
                state,
                ctx,
                SshAdoption {
                    tab_id,
                    connection_id,
                    session,
                    zmodem,
                },
            )
            .await;
        }
        ReadyConnection::Local { io, join } => {
            let adoption = PlainStreamAdoption::local(tab_id, connection_id, io, join, zmodem);
            adopt_local_stream(state, ctx, adoption).await;
        }
        ReadyConnection::Serial { io, join } => {
            let adoption = PlainStreamAdoption::serial(tab_id, connection_id, io, join, zmodem);
            adopt_plain_stream(state, ctx, adoption).await;
        }
        ReadyConnection::Telnet { io, join } => {
            let adoption = PlainStreamAdoption::telnet(tab_id, connection_id, io, join, zmodem);
            adopt_plain_stream(state, ctx, adoption).await;
        }
    }
}

/// SSH 接入参数：与其它流连接共用落地路径，额外携带远端与流量计数。
struct SshAdoption {
    tab_id: TabId,
    connection_id: ConnectionId,
    session: SshSession,
    zmodem: ZmodemConfig,
}

/// SSH 接入：公共落地后追加流量与远端统计采样任务。
async fn adopt_ssh_connection(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    adoption: SshAdoption,
) {
    let SshAdoption {
        tab_id,
        connection_id,
        session,
        zmodem,
    } = adoption;
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

    let traffic_task = spawn_traffic_sampler(traffic, tab_id, connection_id, ctx.event_tx.clone());
    if let Some(session) = state.sessions.get_mut(&tab_id) {
        session.set_traffic_task(traffic_task);
    }
    let stats_task =
        spawn_remote_stats_sampler(remote_clone, tab_id, connection_id, ctx.event_tx.clone());
    if let Some(session) = state.sessions.get_mut(&tab_id) {
        session.set_stats_task(stats_task);
    }
}

/// 无远端句柄的连接接入参数（本地/串口/Telnet）。
struct PlainStreamAdoption {
    tab_id: TabId,
    connection_id: ConnectionId,
    io: TerminalIo,
    join: tokio::sync::oneshot::Receiver<()>,
    disconnect_reason: &'static str,
    zmodem: ZmodemConfig,
}

impl PlainStreamAdoption {
    fn local(
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
        zmodem: ZmodemConfig,
    ) -> Self {
        Self {
            tab_id,
            connection_id,
            io,
            join,
            disconnect_reason: "reconnected to local",
            zmodem,
        }
    }

    fn serial(
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
        zmodem: ZmodemConfig,
    ) -> Self {
        Self {
            tab_id,
            connection_id,
            io,
            join,
            disconnect_reason: "reconnected to serial",
            zmodem,
        }
    }

    fn telnet(
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
        zmodem: ZmodemConfig,
    ) -> Self {
        Self {
            tab_id,
            connection_id,
            io,
            join,
            disconnect_reason: "reconnected to telnet",
            zmodem,
        }
    }
}

/// 本地终端接入：公共落地后登记共享统计订阅。
async fn adopt_local_stream(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    adoption: PlainStreamAdoption,
) {
    let tab_id = adoption.tab_id;
    let connection_id = adoption.connection_id;
    adopt_plain_stream(state, ctx, adoption).await;
    register_local_shared_samplers(state, ctx, tab_id, connection_id);
}

/// 本地/串口/Telnet 接入：无远端句柄，只复用公共落地路径。
async fn adopt_plain_stream(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    adoption: PlainStreamAdoption,
) {
    adopt_stream_connection(
        state,
        ctx,
        StreamConnection {
            tab_id: adoption.tab_id,
            connection_id: adoption.connection_id,
            io: adoption.io,
            join: async move {
                let _ = adoption.join.await;
            },
            remote: None,
            disconnect_reason: adoption.disconnect_reason,
            zmodem: adoption.zmodem,
        },
    )
    .await;
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
        &mut state.sessions,
        &mut state.cleanup_set,
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
    let is_current = state.current_connection(tab_id) == Some(connection_id);
    if !is_current {
        return;
    }
    state.terminate_session(tab_id, "connection lost");
    drop_local_stats(&state.local_stats_tabs, tab_id);
    drop_local_stats(&state.local_traffic_tabs, tab_id);
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
        .send_async(
            TerminalAppEvent::Disconnected {
                tab_id,
                connection_id,
            }
            .into(),
        )
        .await;
}

/// SFTP 客户端就绪：校验连接归属后写入缓存并派发操作。
async fn handle_sftp_ready(state: &mut ControllerState, ctx: &ControllerCtx, internal: Internal) {
    let Internal::SftpReady {
        tab_id: requested_tab,
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
        .session_by_connection(connection_id)
        .map(|(tab_id, _)| tab_id);
    let Some(owner_tab) = valid_owner else {
        drop(result);
        return;
    };
    if requested_tab != owner_tab {
        log::debug!("SFTP 连接归属已变化，请求窗格: {requested_tab}，当前窗格: {owner_tab}");
    }
    // 打开期间可能已被取消或标签已关闭：任务表里没有该操作时丢弃已建立的
    // 客户端，避免"点了取消，删除/覆盖上传仍然执行"。
    if !state.sftp_tasks.contains_key(&(owner_tab, op_id)) {
        log::debug!("SFTP 操作在打开期间已被取消，丢弃已建立的客户端，窗格: {owner_tab}");
        drop(result);
        return;
    }
    state.sftp_tasks.remove(&(owner_tab, op_id));
    match result {
        Ok(client) => {
            if let Some(session) = state.sessions.get_mut(&owner_tab) {
                if op.is_transfer() {
                    session.transfer_sftp_client = Some(client.clone());
                } else {
                    session.sftp_client = Some(client.clone());
                }
            }
            sftp_exec::spawn_sftp_op(
                &mut state.sftp_tasks,
                ctx.event_tx.clone(),
                owner_tab,
                connection_id,
                op_id,
                client,
                op,
            );
        }
        Err(error) => {
            // 打开专用传输通道失败时尝试回退至主控制通道，兼顾只支持单会话（MaxSessions=1）的受限服务器
            if op.is_transfer()
                && let Some(session) = state.sessions.get(&owner_tab)
                && let Some(fallback_client) = session.sftp_client.clone()
            {
                log::warn!(
                    "打开专用 SFTP 传输通道失败，回退至主控制通道，标签: {owner_tab}，原因: {error}"
                );
                sftp_exec::spawn_sftp_op(
                    &mut state.sftp_tasks,
                    ctx.event_tx.clone(),
                    owner_tab,
                    connection_id,
                    op_id,
                    fallback_client,
                    op,
                );
                return;
            }
            let _ = ctx
                .event_tx
                .send_async(
                    SftpAppEvent::SftpFailed {
                        tab_id: owner_tab,
                        connection_id,
                        op_id,
                        error,
                    }
                    .into(),
                )
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
        .sessions
        .get(&tab_id)
        .is_some_and(|s| s.connection_id == connection_id && s.remote.is_some());
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
            if let Some(session) = state.sessions.get_mut(&tab_id) {
                session.forwards.insert(rule_id, task);
            }
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
        .send_async(
            ForwardAppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status,
            }
            .into(),
        )
        .await;
}
