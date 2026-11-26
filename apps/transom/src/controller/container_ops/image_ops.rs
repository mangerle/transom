//! 容器镜像拉取、构建、导入导出与仓库操作命令处理。

use super::lifecycle::make_transfer_progress;
use super::*;

/// 拉取任务被中止时也必须清理取消登记，不能只依赖正常结束路径。
struct PullTaskGuard {
    service: Arc<transom_container::ContainerService>,
    task_id: String,
}

impl Drop for PullTaskGuard {
    fn drop(&mut self) {
        self.service.finish_task(&self.task_id);
    }
}

pub(super) fn handle_image_ops(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
    match command {
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
                let mut progress = make_transfer_progress(event_tx.clone(), request_id);
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
        ContainerCommand::PullImageTask {
            tab_id,
            task_id,
            reference,
            auth,
        } => spawn_pull_image_task(state, ctx, tab_id, task_id, reference, auth),
        ContainerCommand::CancelPullTask { tab_id: _, task_id } => {
            ctx.container.cancel_task(&task_id);
        }
        ContainerCommand::RemoveImage {
            tab_id,
            request_id,
            image,
            force,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.remove_image(&image, force).await
        }),
        ContainerCommand::TagImage {
            tab_id,
            request_id,
            image,
            repo,
            tag,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.tag_image(&image, &repo, tag.as_deref()).await
        }),
        ContainerCommand::PushImage {
            tab_id,
            request_id,
            image,
            tag,
            auth,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                let mut progress = make_transfer_progress(event_tx.clone(), request_id);
                let error = engine
                    .push_image(&image, tag.as_deref(), auth.as_ref(), &mut progress)
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
        ContainerCommand::PushTaggedImage {
            tab_id,
            request_id,
            source_image,
            repo,
            tag,
            auth,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                let mut progress = make_transfer_progress(event_tx.clone(), request_id);
                // 打标签与推送必须在同一任务里顺序执行；拆成两条命令会让 push
                // 抢在 tag 完成前到达引擎，目标标签不存在时随机失败。
                let error = match engine.tag_image(&source_image, &repo, tag.as_deref()).await {
                    Ok(()) => engine
                        .push_image(&repo, tag.as_deref(), auth.as_ref(), &mut progress)
                        .await
                        .err()
                        .map(AppError::from),
                    Err(error) => Some(AppError::from(error)),
                };
                send_event(
                    &event_tx,
                    ContainerAppEvent::ActionFinished { request_id, error },
                )
                .await;
            });
        }
        ContainerCommand::BuildImage {
            tab_id,
            request_id,
            spec,
        } => spawn_build_image(state, ctx, tab_id, request_id, spec),
        _ => {}
    }
}

/// 启动带分层进度上报的镜像拉取后台任务。
fn spawn_pull_image_task(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    task_id: String,
    reference: String,
    auth: Option<transom_core::ContainerRegistryAuth>,
) {
    let Some(engine) = state
        .container_workspaces
        .get(&tab_id)
        .map(|w| w.engine.clone())
    else {
        let event_tx = ctx.event_tx.clone();
        let error = AppError::with_detail(ErrorCode::ContainerUnavailable, "工作区未连接");
        state.container_ops.spawn(async move {
            send_event(
                &event_tx,
                ContainerAppEvent::PullTaskFinished {
                    task_id,
                    error: Some(error),
                },
            )
            .await;
        });
        return;
    };
    let cancel = ctx.container.register_task_cancel(&task_id);
    let event_tx = ctx.event_tx.clone();
    let internal_tx = ctx.internal_tx.clone();
    let container_service = Arc::clone(&ctx.container);
    let tid = task_id.clone();
    let handle_task_id = task_id.clone();
    let handle = tokio::spawn(async move {
        let _guard = PullTaskGuard {
            service: Arc::clone(&container_service),
            task_id: tid.clone(),
        };
        let mut layers_map: std::collections::HashMap<String, transom_core::ImageLayerProgress> =
            std::collections::HashMap::with_capacity(16);
        let mut progress = |status: String, layer_opt: Option<transom_core::ImageLayerProgress>| {
            if let Some(layer) = layer_opt {
                layers_map.insert(layer.id.clone(), layer);
            }
            let layers: Vec<transom_core::ImageLayerProgress> =
                layers_map.values().cloned().collect();
            let _ = event_tx.try_send(
                ContainerAppEvent::PullTaskProgress {
                    task_id: tid.clone(),
                    status,
                    layers,
                }
                .into(),
            );
        };
        let error = engine
            .pull_image_task(&reference, auth.as_ref(), cancel, &mut progress)
            .await
            .err()
            .map(AppError::from);
        send_event(
            &event_tx,
            ContainerAppEvent::PullTaskFinished {
                task_id: tid.clone(),
                error,
            },
        )
        .await;
        let _ = internal_tx
            .send_async(Internal::ContainerPullTaskFinished {
                tab_id,
                task_id: tid,
            })
            .await;
    });
    if let Some(workspace) = state.container_workspaces.get_mut(&tab_id) {
        workspace.insert_pull_task(handle_task_id, handle);
    } else {
        handle.abort();
    }
}

/// 启动镜像构建任务，并将高频日志输出按 64KiB/100ms 批量合流上报。
fn spawn_build_image(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    tab_id: TabId,
    request_id: ContainerRequestId,
    spec: transom_core::ContainerBuildSpec,
) {
    let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
        return;
    };
    let event_tx = ctx.event_tx.clone();
    let cancel_for_build = Arc::clone(&cancel);
    spawn_long(state, ctx, tab_id, request_id, cancel, async move {
        // 构建输出可能逐行高频到达；合并成 64KiB/100ms 的小批次再进事件总线。
        const BUILD_BATCH_BYTES: usize = 64 * 1024;
        const BUILD_BATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
        let mut pending = String::new();
        let mut last_flush = tokio::time::Instant::now();
        let mut progress = |line: String| {
            pending.push_str(&line);
            pending.push('\n');
            if pending.len() >= BUILD_BATCH_BYTES || last_flush.elapsed() >= BUILD_BATCH_INTERVAL {
                let batch = std::mem::take(&mut pending);
                let _ = event_tx.try_send(
                    ContainerAppEvent::BuildProgress {
                        request_id,
                        line: batch,
                    }
                    .into(),
                );
                last_flush = tokio::time::Instant::now();
            }
        };
        let error = engine
            .build_image(&spec, &cancel_for_build, &mut progress)
            .await
            .err()
            .map(AppError::from);
        drop(progress);
        if !pending.is_empty() {
            let _ = event_tx.try_send(
                ContainerAppEvent::BuildProgress {
                    request_id,
                    line: pending,
                }
                .into(),
            );
        }
        send_event(
            &event_tx,
            ContainerAppEvent::ActionFinished { request_id, error },
        )
        .await;
    });
}
