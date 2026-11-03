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
