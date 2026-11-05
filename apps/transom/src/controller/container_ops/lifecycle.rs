//! 生命周期、编排、镜像与卷/网络变更。

use super::*;

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
            let container_service = Arc::clone(&ctx.container);
            let tid = task_id.clone();
            tokio::spawn(async move {
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
                container_service.finish_task(&tid);
                let _ = event_tx.try_send(
                    ContainerAppEvent::PullTaskFinished {
                        task_id: tid,
                        error,
                    }
                    .into(),
                );
            });
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
        ContainerCommand::BuildImage {
            tab_id,
            request_id,
            spec,
        } => {
            let Some((engine, cancel)) = register_long_task(state, ctx, tab_id, request_id) else {
                return;
            };
            let event_tx = ctx.event_tx.clone();
            spawn_long(state, ctx, tab_id, request_id, cancel, async move {
                let mut progress = |line: String| {
                    let _ = event_tx
                        .try_send(ContainerAppEvent::BuildProgress { request_id, line }.into());
                };
                let error = engine
                    .build_image(&spec, &mut progress)
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
        _ => {}
    }
}
