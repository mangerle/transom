//! 连接接入：连接意图裁决、SSH/本地/串口/Telnet 的统一落地与本地采样订阅。

use crate::controller::lifecycle::{
    Admission, AdoptConnection, admit_connection, adopt_connection,
};
use crate::controller::samplers::{
    run_local_stats_sampler, run_local_traffic_sampler, spawn_remote_stats_sampler,
    spawn_traffic_sampler,
};
use crate::controller::state::{ControllerCtx, ControllerState};
use crate::controller::{Internal, ReadyConnection};
use transom_core::{ConnectionId, TabId, TerminalIo};
use transom_net::{SshRemote, SshSession};
use transom_zmodem::ZmodemConfig;

/// 连接接入：先按连接意图裁决，再走统一的落地路径。
///
/// 连接形态与 ZMODEM 配置随事件一起到达（配置由界面随命令携带），
/// 控制器不再为每次连接同步读一次设置文件。
pub(super) async fn handle_connected(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
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
    let stats_task = spawn_remote_stats_sampler(
        remote_clone,
        tab_id,
        connection_id,
        ctx.event_tx.clone(),
        state.stats_sampling.clone(),
    );
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
            state.stats_sampling.clone(),
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
