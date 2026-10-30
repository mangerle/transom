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
    AppError, ContainerAppEvent, ContainerCommand, ContainerEngineProfile, ContainerRequestId,
    ErrorCode, TabId,
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
        ContainerCommand::Connect { tab_id, profile } => connect(state, ctx, tab_id, profile),
        ContainerCommand::Disconnect { tab_id } => {
            if let Some(task) = state.container_connects.remove(&tab_id) {
                task.abort();
            }
            if let Some(mut workspace) = state.container_workspaces.remove(&tab_id) {
                for request_id in workspace.shutdown() {
                    ctx.container.finish(request_id);
                }
            }
            let _ = ctx
                .event_tx
                .try_send(ContainerAppEvent::Disconnected { tab_id }.into());
        }
        ContainerCommand::TestConnection {
            request_id,
            profile,
        } => {
            let service = Arc::clone(&ctx.container);
            let event_tx = ctx.event_tx.clone();
            state.container_ops.spawn(async move {
                let (info, error) = match service.connect(&profile).await {
                    Ok(engine) => (Some(Box::new(engine.info)), None),
                    Err(error) => (None, Some(AppError::from(error))),
                };
                send_event(
                    &event_tx,
                    ContainerAppEvent::TestFinished {
                        request_id,
                        info,
                        error,
                    },
                )
                .await;
            });
        }
        ContainerCommand::Cancel { tab_id, request_id } => {
            cancel_task(state, ctx, tab_id, request_id)
        }
        ContainerCommand::Discover { request_id } => {
            let event_tx = ctx.event_tx.clone();
            state.container_ops.spawn(async move {
                let (engines, failures) = discover_all().await;
                let profiles: Vec<ContainerEngineProfile> = engines
                    .iter()
                    .map(|engine| {
                        ContainerEngineProfile::new(
                            engine.info.engine.kind.to_string(),
                            engine.info.engine.clone(),
                        )
                    })
                    .collect();
                log::info!(
                    "容器引擎发现完成，可用 {} 个，不可用 {} 项",
                    profiles.len(),
                    failures.len()
                );
                send_event(
                    &event_tx,
                    ContainerAppEvent::EnginesDiscovered {
                        request_id,
                        engines: profiles,
                        failures,
                    },
                )
                .await;
            });
        }
        ContainerCommand::ListContainers { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.list_containers().await },
            |request_id, containers| ContainerAppEvent::ContainersListed {
                request_id,
                containers,
            },
        ),
        ContainerCommand::InspectContainer {
            tab_id,
            request_id,
            container,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move { engine.inspect_container(&container).await },
            |request_id, details| ContainerAppEvent::ContainerInspected {
                request_id,
                details: Box::new(details),
            },
        ),
        ContainerCommand::Lifecycle {
            tab_id,
            request_id,
            container,
            action,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.lifecycle(&container, action).await
        }),
        ContainerCommand::ExecTerminal {
            tab_id,
            request_id,
            options,
            size,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                let outcome = transom_container::open_exec(&engine, &options, size).await;
                match outcome {
                    Ok((io, exit)) => {
                        send_event(
                            &event_tx,
                            ContainerAppEvent::ExecReady {
                                tab_id,
                                request_id,
                                io: Box::new(io),
                                exit,
                            },
                        )
                        .await;
                    }
                    Err(error) => send_failed(&event_tx, request_id, error).await,
                }
            });
        }
        ContainerCommand::ListImages { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.list_images().await },
            |request_id, images| ContainerAppEvent::ImagesListed { request_id, images },
        ),
        ContainerCommand::InspectImage {
            tab_id,
            request_id,
            image,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move { engine.inspect_image(&image).await },
            |request_id, details| ContainerAppEvent::ImageInspected {
                request_id,
                details: Box::new(details),
            },
        ),
        ContainerCommand::PullImage {
            tab_id,
            request_id,
            reference,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                let mut progress = |current: u64, total: Option<u64>| {
                    let _ = event_tx.try_send(
                        ContainerAppEvent::TransferProgress {
                            request_id,
                            transferred: current,
                            total,
                        }
                        .into(),
                    );
                };
                let error = engine
                    .pull_image(&reference, &mut progress)
                    .await
                    .err()
                    .map(AppError::from);
                send_event(
                    &event_tx,
                    ContainerAppEvent::ActionFinished { request_id, error },
                )
                .await;
            });
        }
        ContainerCommand::RemoveImage {
            tab_id,
            request_id,
            image,
            force,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.remove_image(&image, force).await
        }),
        ContainerCommand::ListVolumes { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.list_volumes().await },
            |request_id, volumes| ContainerAppEvent::VolumesListed {
                request_id,
                volumes,
            },
        ),
        ContainerCommand::InspectVolume {
            tab_id,
            request_id,
            name,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move { engine.inspect_volume(&name).await },
            |request_id, details| ContainerAppEvent::VolumeInspected {
                request_id,
                details: Box::new(details),
            },
        ),
        ContainerCommand::CreateVolume {
            tab_id,
            request_id,
            name,
            driver,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.create_volume(&name, driver.as_deref()).await
        }),
        ContainerCommand::RemoveVolume {
            tab_id,
            request_id,
            name,
            force,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.remove_volume(&name, force).await
        }),
        ContainerCommand::ListNetworks { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.list_networks().await },
            |request_id, networks| ContainerAppEvent::NetworksListed {
                request_id,
                networks,
            },
        ),
        ContainerCommand::InspectNetwork {
            tab_id,
            request_id,
            network,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move { engine.inspect_network(&network).await },
            |request_id, details| ContainerAppEvent::NetworkInspected {
                request_id,
                details: Box::new(details),
            },
        ),
        ContainerCommand::CreateNetwork {
            tab_id,
            request_id,
            name,
            driver,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.create_network(&name, driver.as_deref()).await
        }),
        ContainerCommand::RemoveNetwork {
            tab_id,
            request_id,
            network,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.remove_network(&network).await
        }),
        ContainerCommand::ListComposeProjects { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.compose_projects().await },
            |request_id, projects| ContainerAppEvent::ComposeProjectsListed {
                request_id,
                projects,
            },
        ),
        ContainerCommand::Logs {
            tab_id,
            request_id,
            container,
            tail,
            follow,
            timestamps,
        } => {
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
                    let mut sink = |bytes: Vec<u8>| {
                        let _ = event_tx
                            .try_send(ContainerAppEvent::LogsChunk { request_id, bytes }.into());
                    };
                    let result = transom_container::stream_logs(
                        &engine, &container, tail, follow, timestamps, &cancel, &mut sink,
                    )
                    .await;
                    match result {
                        Ok(()) | Err(transom_container::ContainerError::Cancelled) => {}
                        Err(error) => send_failed(&event_tx, request_id, error).await,
                    }
                    send_event(&event_tx, ContainerAppEvent::LogsClosed { request_id }).await;
                },
            );
        }
        ContainerCommand::Stats {
            tab_id,
            request_id,
            container,
            enabled,
        } => {
            if !enabled {
                cancel_task(state, ctx, tab_id, request_id);
                return;
            }
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
                    let mut ticker = tokio::time::interval(Duration::from_secs(3));
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        ticker.tick().await;
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        match engine.container_stats(&container, &cancel).await {
                            Ok(stats) => {
                                let _ = event_tx.try_send(
                                    ContainerAppEvent::StatsUpdated {
                                        request_id,
                                        container: container.clone(),
                                        stats,
                                    }
                                    .into(),
                                );
                            }
                            Err(transom_container::ContainerError::Cancelled) => break,
                            Err(error) => {
                                send_failed(&event_tx, request_id, error).await;
                                break;
                            }
                        }
                    }
                },
            );
        }
        ContainerCommand::ListDir {
            tab_id,
            request_id,
            container,
            path,
        } => {
            let reply_path = path.clone();
            one_shot(
                state,
                ctx,
                tab_id,
                request_id,
                move |engine| async move {
                    transom_container::list_dir(&engine, &container, &path).await
                },
                move |request_id, entries| ContainerAppEvent::DirListed {
                    request_id,
                    path: reply_path,
                    entries,
                },
            )
        }
        ContainerCommand::ReadFile {
            tab_id,
            request_id,
            container,
            path,
            max_bytes,
        } => {
            let reply_path = path.clone();
            one_shot(
                state,
                ctx,
                tab_id,
                request_id,
                move |engine| async move {
                    transom_container::read_file(&engine, &container, &path, max_bytes).await
                },
                move |request_id, (bytes, truncated, size)| ContainerAppEvent::FilePreview {
                    request_id,
                    path: reply_path,
                    content: String::from_utf8_lossy(&bytes).to_string(),
                    truncated,
                    size,
                },
            )
        }
        ContainerCommand::Download {
            tab_id,
            request_id,
            container,
            path,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::download(
                    &engine,
                    &container,
                    &path,
                    &local_path,
                    cancel,
                    progress,
                )
                .await
                .map(|_| ())
            },
        ),
        ContainerCommand::Upload {
            tab_id,
            request_id,
            container,
            path,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::upload(&engine, &container, &path, &local_path, cancel, progress)
                    .await
                    .map(|_| ())
            },
        ),
        ContainerCommand::RemovePath {
            tab_id,
            request_id,
            container,
            path,
            recursive,
        } => {
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
                    let error = transom_container::remove_path(
                        &engine, &container, &path, recursive, &cancel,
                    )
                    .await
                    .err()
                    .map(AppError::from);
                    send_event(
                        &event_tx,
                        ContainerAppEvent::TransferFinished { request_id, error },
                    )
                    .await;
                },
            );
        }
        ContainerCommand::Rename {
            tab_id,
            request_id,
            container,
            from,
            to,
        } => {
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
                    let error =
                        transom_container::rename_path(&engine, &container, &from, &to, &cancel)
                            .await
                            .err()
                            .map(AppError::from);
                    send_event(
                        &event_tx,
                        ContainerAppEvent::TransferFinished { request_id, error },
                    )
                    .await;
                },
            );
        }
        ContainerCommand::CreateDir {
            tab_id,
            request_id,
            container,
            path,
        } => {
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
                    let error = transom_container::create_dir(&engine, &container, &path, &cancel)
                        .await
                        .err()
                        .map(AppError::from);
                    send_event(
                        &event_tx,
                        ContainerAppEvent::TransferFinished { request_id, error },
                    )
                    .await;
                },
            );
        }
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
