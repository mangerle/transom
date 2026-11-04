//! 容器管理工作区命令处理。
//!
//! 设计原理（Why）：
//! - 工作区以标签为单位，控制器保存已探测的 [`Engine`]；后续查询只带
//!   `tab_id`，不必每次重新发现；
//! - 全部命令共用两条骨架：一次性查询进 `container_ops` JoinSet，长任务
//!   （日志/统计/传输/exec）按请求 ID 登记可取消句柄；`one_shot` /
//!   `run_action` / `spawn_long` 三个 helper 消化了原本每个分支都要重写的
//!   "取引擎 → 执行 → 回报 → 失败转事件"样板；
//! - 关闭工作区或标签时统一中止长任务并清理取消标志，不留脱缰任务。

mod engine;
mod files;
mod lifecycle;
mod query;
mod streams;

use super::Internal;
use super::state::{ControllerCtx, ControllerState};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::task::JoinHandle;
use transom_container::{Engine, discover_all};
use transom_core::{
    AppError, ContainerAppEvent, ContainerCommand, ContainerEngineProfile,
    ContainerLifecycleAction, ContainerRequestId, ContainerStats, ErrorCode, TabId,
};

/// 一个打开的工作区：引擎实例 + 可取消的长任务。
pub(super) struct ContainerWorkspace {
    pub(super) engine: Engine,
    tasks: HashMap<ContainerRequestId, TaskEntry>,
}

struct TaskEntry {
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl ContainerWorkspace {
    pub(super) fn new(engine: Engine) -> Self {
        Self {
            engine,
            tasks: HashMap::new(),
        }
    }

    /// 登记长任务；同一请求 ID 重复登记时先中止旧任务（例如重复开启统计）。
    fn insert_task(
        &mut self,
        request_id: ContainerRequestId,
        cancel: Arc<AtomicBool>,
        handle: JoinHandle<()>,
    ) {
        if let Some(previous) = self.tasks.insert(request_id, TaskEntry { cancel, handle }) {
            previous.cancel.store(true, Ordering::Relaxed);
            previous.handle.abort();
        }
    }

    fn cancel(&mut self, request_id: ContainerRequestId) -> bool {
        let Some(entry) = self.tasks.remove(&request_id) else {
            return false;
        };
        entry.cancel.store(true, Ordering::Relaxed);
        entry.handle.abort();
        true
    }

    fn remove_finished(&mut self, request_id: ContainerRequestId) {
        self.tasks.remove(&request_id);
    }

    /// 中止全部长任务；工作区关闭/标签关闭时调用。
    ///
    /// 返回被中止的请求 ID，供调用方清理取消标志登记（停机路径可忽略）。
    pub(super) fn shutdown(&mut self) -> Vec<ContainerRequestId> {
        let request_ids = self.tasks.keys().copied().collect();
        for (_, entry) in self.tasks.drain() {
            entry.cancel.store(true, Ordering::Relaxed);
            entry.handle.abort();
        }
        request_ids
    }
}

/// 处理容器管理命令。
pub(super) fn handle_container_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
    match command {
        ContainerCommand::Connect { .. }
        | ContainerCommand::Disconnect { .. }
        | ContainerCommand::TestConnection { .. }
        | ContainerCommand::Discover { .. } => engine::handle(state, ctx, command),
        ContainerCommand::ListContainers { .. }
        | ContainerCommand::InspectContainer { .. }
        | ContainerCommand::DiffContainer { .. }
        | ContainerCommand::ListImages { .. }
        | ContainerCommand::InspectImage { .. }
        | ContainerCommand::ListVolumes { .. }
        | ContainerCommand::InspectVolume { .. }
        | ContainerCommand::ListNetworks { .. }
        | ContainerCommand::InspectNetwork { .. }
        | ContainerCommand::ListComposeProjects { .. }
        | ContainerCommand::PruneVolumes { .. }
        | ContainerCommand::PruneImages { .. }
        | ContainerCommand::PruneNetworks { .. }
        | ContainerCommand::SystemDf { .. }
        | ContainerCommand::SystemPrune { .. }
        | ContainerCommand::SearchImages { .. } => query::handle(state, ctx, command),
        ContainerCommand::RunCompose { .. }
        | ContainerCommand::Lifecycle { .. }
        | ContainerCommand::PullImage { .. }
        | ContainerCommand::PullImageTask { .. }
        | ContainerCommand::CancelPullTask { .. }
        | ContainerCommand::RemoveImage { .. }
        | ContainerCommand::TagImage { .. }
        | ContainerCommand::PushImage { .. }
        | ContainerCommand::BuildImage { .. }
        | ContainerCommand::CreateVolume { .. }
        | ContainerCommand::RemoveVolume { .. }
        | ContainerCommand::CreateNetwork { .. }
        | ContainerCommand::RemoveNetwork { .. }
        | ContainerCommand::ConnectNetwork { .. }
        | ContainerCommand::DisconnectNetwork { .. }
        | ContainerCommand::Rename { .. }
        | ContainerCommand::RenameContainer { .. }
        | ContainerCommand::UpdateContainer { .. }
        | ContainerCommand::CommitContainer { .. }
        | ContainerCommand::CreateContainer { .. } => lifecycle::handle(state, ctx, command),
        ContainerCommand::Cancel { .. }
        | ContainerCommand::ExecTerminal { .. }
        | ContainerCommand::Logs { .. }
        | ContainerCommand::ProjectLogs { .. }
        | ContainerCommand::Stats { .. }
        | ContainerCommand::ListProcesses { .. } => streams::handle(state, ctx, command),
        ContainerCommand::ListDir { .. }
        | ContainerCommand::ListVolumeDir { .. }
        | ContainerCommand::ReadFile { .. }
        | ContainerCommand::ReadVolumeFile { .. }
        | ContainerCommand::Download { .. }
        | ContainerCommand::DownloadVolumeFile { .. }
        | ContainerCommand::Upload { .. }
        | ContainerCommand::RemovePath { .. }
        | ContainerCommand::CreateDir { .. }
        | ContainerCommand::SaveFile { .. }
        | ContainerCommand::SaveVolumeFile { .. }
        | ContainerCommand::SaveImage { .. }
        | ContainerCommand::LoadImage { .. } => files::handle(state, ctx, command),
    }
}

/// 解析并登记引擎工作区；探测结果由 `Internal::ContainerConnected` 回流。
fn connect(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    profile: ContainerEngineProfile,
) {
    if let Some(previous) = state.container_connects.remove(&tab_id) {
        previous.abort();
    }
    if let Some(mut workspace) = state.container_workspaces.remove(&tab_id) {
        for request_id in workspace.shutdown() {
            ctx.container.finish(request_id);
        }
    }
    let service = Arc::clone(&ctx.container);
    let internal_tx = ctx.internal_tx.clone();
    let handle = tokio::spawn(async move {
        let result = service.connect(&profile).await.map_err(AppError::from);
        let _ = internal_tx
            .send_async(Internal::ContainerConnected { tab_id, result })
            .await;
    });
    state.container_connects.insert(tab_id, handle);
}

fn cancel_task(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
) {
    ctx.container.cancel(request_id);
    if let Some(workspace) = state.container_workspaces.get_mut(&tab_id) {
        workspace.cancel(request_id);
    }
    ctx.container.finish(request_id);
}

/// 长任务自行结束时的句柄回收。
pub(super) fn finish_task(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
) {
    if let Some(workspace) = state.container_workspaces.get_mut(&tab_id) {
        workspace.remove_finished(request_id);
    }
    ctx.container.finish(request_id);
}

/// 一次性查询：取引擎 → 执行 → 用 `reply` 包装成事件。
fn one_shot<T, Fut, Q, R>(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
    query: Q,
    reply: R,
) where
    T: Send + 'static,
    Fut: Future<Output = Result<T, transom_container::ContainerError>> + Send + 'static,
    Q: FnOnce(Engine) -> Fut + Send + 'static,
    R: FnOnce(ContainerRequestId, T) -> ContainerAppEvent + Send + 'static,
{
    let Some(engine) = engine_for(state, ctx, tab_id, request_id) else {
        return;
    };
    let event_tx = ctx.event_tx.clone();
    state.container_ops.spawn(async move {
        match query(engine).await {
            Ok(value) => send_event(&event_tx, reply(request_id, value)).await,
            Err(error) => send_failed(&event_tx, request_id, error).await,
        }
    });
}

/// 一次性动作：成功与失败都收敛为 `ActionFinished`。
fn run_action<Fut, A>(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
    action: A,
) where
    Fut: Future<Output = Result<(), transom_container::ContainerError>> + Send + 'static,
    A: FnOnce(Engine) -> Fut + Send + 'static,
{
    one_shot(state, ctx, tab_id, request_id, action, |request_id, ()| {
        ContainerAppEvent::ActionFinished {
            request_id,
            error: None,
        }
    });
}

/// 长任务（文件传输）：登记取消标志，进度转成事件，结果转成 `TransferFinished`。
fn transfer<Fut, T>(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
    work: T,
) where
    Fut: Future<Output = Result<(), transom_container::ContainerError>> + Send + 'static,
    T: FnOnce(Engine, Arc<AtomicBool>, flume::Sender<(u64, Option<u64>)>) -> Fut + Send + 'static,
{
    let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
        return;
    };
    let event_tx = ctx.event_tx.clone();
    spawn_long(
        state,
        ctx,
        tab_id,
        request_id,
        Arc::clone(&cancel),
        async move {
            let (progress_tx, progress_rx) = flume::bounded(16);
            let progress_events = event_tx.clone();
            let pump = tokio::spawn(async move {
                while let Ok((transferred, total)) = progress_rx.recv_async().await {
                    let _ = progress_events.try_send(
                        ContainerAppEvent::TransferProgress {
                            request_id,
                            transferred,
                            total,
                        }
                        .into(),
                    );
                }
            });
            let error = work(engine, cancel, progress_tx)
                .await
                .err()
                .map(AppError::from);
            let _ = pump.await;
            send_event(
                &event_tx,
                ContainerAppEvent::TransferFinished { request_id, error },
            )
            .await;
        },
    );
}

/// 取引擎并登记取消标志；工作区不在时直接回报失败。
fn register_long_task(
    state: &ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
) -> Option<(Engine, Arc<AtomicBool>)> {
    let engine = engine_for(state, ctx, tab_id, request_id)?;
    Some((engine, ctx.container.register_cancel(request_id)))
}

/// 派生长任务并在结束时通知主循环回收句柄。
///
/// 被登记的任务本身就是"执行 + 回报完成"，因此 `cancel`/`abort` 作用的正是
/// 业务任务本身；若再套一层驱动任务，中止只会丢掉通知而留下业务任务在跑。
fn spawn_long(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
    cancel: Arc<AtomicBool>,
    task: impl Future<Output = ()> + Send + 'static,
) {
    let internal_tx = ctx.internal_tx.clone();
    let handle = tokio::spawn(async move {
        task.await;
        let _ = internal_tx
            .send_async(Internal::ContainerTaskFinished { tab_id, request_id })
            .await;
    });
    match state.container_workspaces.get_mut(&tab_id) {
        Some(workspace) => workspace.insert_task(request_id, cancel, handle),
        None => handle.abort(),
    }
}

/// 引擎是否跑在本机：只有本机引擎的宿主负载与本机一致。
fn is_local_engine(engine: &transom_container::Engine) -> bool {
    matches!(
        engine.info.engine.endpoint,
        transom_core::ContainerEndpoint::Unix { .. } | transom_core::ContainerEndpoint::Cli { .. }
    )
}

/// 解析 `/proc/loadavg` 的前三个字段；格式不符时返回 `None`。
fn parse_loadavg(bytes: &[u8]) -> Option<(f32, f32, f32)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut fields = text.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some((one, five, fifteen))
}

fn engine_for(
    state: &ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
) -> Option<Engine> {
    match state.container_workspaces.get(&tab_id) {
        Some(workspace) => Some(workspace.engine.clone()),
        None => {
            let error = AppError::with_detail(
                ErrorCode::ContainerUnavailable,
                "容器工作区尚未连接或已关闭",
            );
            let _ = ctx
                .event_tx
                .try_send(ContainerAppEvent::Failed { request_id, error }.into());
            None
        }
    }
}

async fn send_event(event_tx: &flume::Sender<transom_core::AppEvent>, event: ContainerAppEvent) {
    let _ = event_tx.send_async(event.into()).await;
}

async fn send_failed(
    event_tx: &flume::Sender<transom_core::AppEvent>,
    request_id: ContainerRequestId,
    error: transom_container::ContainerError,
) {
    log::debug!("容器管理操作失败，请求: {request_id}，原因: {error}");
    send_event(
        event_tx,
        ContainerAppEvent::Failed {
            request_id,
            error: error.into(),
        },
    )
    .await;
}
