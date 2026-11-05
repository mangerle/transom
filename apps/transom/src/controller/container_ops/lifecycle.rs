//! 生命周期、编排、镜像与卷/网络变更。

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

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::RunCompose {
            tab_id,
            request_id,
            project,
            action,
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
                        match transom_container::compose::run(&engine, &project, action, &cancel)
                            .await
                        {
                            Ok(output) => {
                                log::info!(
                                    "编排命令完成，项目: {}，命令: {}，输出尾部: {output}",
                                    project.name,
                                    action.label()
                                );
                                None
                            }
                            Err(error) => {
                                log::error!(
                                    "编排命令失败，项目: {}，命令: {}，原因: {error}",
                                    project.name,
                                    action.label()
                                );
                                Some(AppError::from(error))
                            }
                        };
                    send_event(
                        &event_tx,
                        ContainerAppEvent::ActionFinished { request_id, error },
                    )
                    .await;
                },
            );
        }
        ContainerCommand::Lifecycle {
            tab_id,
            request_id,
            container,
            action,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.lifecycle(&container, action).await
        }),
        ContainerCommand::UpdateContainer {
            tab_id,
            request_id,
            container,
            spec,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.update_container(&container, &spec).await
        }),
        ContainerCommand::CommitContainer {
            tab_id,
            request_id,
            container,
            repo,
            tag,
            comment,
            author,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine
                .commit_container(
                    &container,
                    &repo,
                    tag.as_deref(),
                    comment.as_deref(),
                    author.as_deref(),
                )
                .await
                .map(|_| ())
        }),
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
        ContainerCommand::PullImageTask {
            tab_id,
            task_id,
            reference,
            auth,
        } => {
            let Some(engine) = state
                .container_workspaces
                .get(&tab_id)
                .map(|w| w.engine.clone())
            else {
                let _ = ctx.event_tx.try_send(
                    ContainerAppEvent::PullTaskFinished {
                        task_id,
                        error: Some(AppError::with_detail(
                            ErrorCode::ContainerUnavailable,
                            "工作区未连接",
                        )),
                    }
                    .into(),
                );
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
                let mut layers_map: std::collections::HashMap<
                    String,
                    transom_core::ImageLayerProgress,
                > = std::collections::HashMap::with_capacity(16);
                let mut progress =
                    |status: String, layer_opt: Option<transom_core::ImageLayerProgress>| {
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
                let _ = event_tx.try_send(
                    ContainerAppEvent::PullTaskFinished {
                        task_id: tid.clone(),
                        error,
                    }
                    .into(),
                );
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
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            let cancel_for_build = std::sync::Arc::clone(&cancel);
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                // 构建输出可能逐行高频到达；合并成 64KiB/100ms 的小批次再进事件总线。
                const BUILD_BATCH_BYTES: usize = 64 * 1024;
                const BUILD_BATCH_INTERVAL: std::time::Duration =
                    std::time::Duration::from_millis(100);
                let mut pending = String::new();
                let mut last_flush = tokio::time::Instant::now();
                let mut progress = |line: String| {
                    pending.push_str(&line);
                    pending.push('\n');
                    if pending.len() >= BUILD_BATCH_BYTES
                        || last_flush.elapsed() >= BUILD_BATCH_INTERVAL
                    {
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
        ContainerCommand::CreateNetwork {
            tab_id,
            request_id,
            spec,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.create_network(&spec).await
        }),
        ContainerCommand::RemoveNetwork {
            tab_id,
            request_id,
            network,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.remove_network(&network).await
        }),
        ContainerCommand::ConnectNetwork {
            tab_id,
            request_id,
            network,
            container,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.connect_network(&network, &container).await
        }),
        ContainerCommand::DisconnectNetwork {
            tab_id,
            request_id,
            network,
            container,
            force,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.disconnect_network(&network, &container, force).await
        }),
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
        ContainerCommand::RenameContainer {
            tab_id,
            request_id,
            container,
            name,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            engine.rename_container(&container, &name).await
        }),
        ContainerCommand::CreateContainer {
            tab_id,
            request_id,
            spec,
        } => {
            let action = move |engine: Engine| async move {
                let id = engine.create_container(&spec).await?;
                // 创建后立刻启动：界面这一项的语义是「运行」，不是「创建了个停着的容器」。
                engine
                    .lifecycle(&id, ContainerLifecycleAction::Start)
                    .await
                    .inspect(|_| {
                        log::info!("容器已创建并启动，镜像: {}，新容器: {}", spec.image, id)
                    })?;
                Ok(())
            };
            run_action(state, ctx, tab_id, request_id, action);
        }
        ContainerCommand::RecreateContainer {
            tab_id,
            request_id,
            old_container_id,
            spec,
        } => {
            let action = move |engine: Engine| async move {
                // 1. 强制移除原容器（force: true 会自动处理停止状态，并保留外部挂载卷）
                let _ = engine
                    .lifecycle(
                        &old_container_id,
                        ContainerLifecycleAction::Remove { force: true },
                    )
                    .await;
                // 3. 以新配置创建容器
                let id = engine.create_container(&spec).await?;
                // 4. 启动新容器
                engine
                    .lifecycle(&id, ContainerLifecycleAction::Start)
                    .await
                    .inspect(|_| {
                        log::info!(
                            "容器已就地重建并启动，原ID: {}，镜像: {}，新容器: {}",
                            old_container_id,
                            spec.image,
                            id
                        )
                    })?;
                Ok(())
            };
            run_action(state, ctx, tab_id, request_id, action);
        }
        _ => {}
    }
}
