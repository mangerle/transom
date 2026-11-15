//! 生命周期、编排、镜像与卷/网络变更。

use super::image_ops::handle_image_ops;
use super::*;

/// 拉取任务被中止时也必须清理取消登记，不能只依赖正常结束路径。
pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::RunCompose { .. }
        | ContainerCommand::Lifecycle { .. }
        | ContainerCommand::UpdateContainer { .. }
        | ContainerCommand::CommitContainer { .. }
        | ContainerCommand::Rename { .. }
        | ContainerCommand::RenameContainer { .. }
        | ContainerCommand::CreateContainer { .. }
        | ContainerCommand::RecreateContainer { .. } => {
            handle_compose_and_container(state, ctx, command);
        }
        ContainerCommand::PullImage { .. }
        | ContainerCommand::PullImageTask { .. }
        | ContainerCommand::CancelPullTask { .. }
        | ContainerCommand::RemoveImage { .. }
        | ContainerCommand::TagImage { .. }
        | ContainerCommand::PushImage { .. }
        | ContainerCommand::PushTaggedImage { .. }
        | ContainerCommand::BuildImage { .. } => {
            handle_image_ops(state, ctx, command);
        }
        ContainerCommand::CreateVolume { .. }
        | ContainerCommand::RemoveVolume { .. }
        | ContainerCommand::CreateNetwork { .. }
        | ContainerCommand::RemoveNetwork { .. }
        | ContainerCommand::ConnectNetwork { .. }
        | ContainerCommand::DisconnectNetwork { .. } => {
            handle_volume_and_network(state, ctx, command);
        }
        _ => {}
    }
}

/// 构造统一的传输进度回调，避免 Pull/Push 重复编写相同闭包。
pub(super) fn make_transfer_progress(
    event_tx: flume::Sender<transom_core::AppEvent>,
    request_id: ContainerRequestId,
) -> impl FnMut(u64, Option<u64>) {
    move |current: u64, total: Option<u64>| {
        let _ = event_tx.try_send(
            ContainerAppEvent::TransferProgress {
                request_id,
                transferred: current,
                total,
            }
            .into(),
        );
    }
}

/// 处理 Compose 编排与容器生命周期/创建/重建命令。
fn handle_compose_and_container(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
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
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            let id = engine.create_container(&spec).await?;
            // 创建后立刻启动：界面这一项的语义是「运行」，不是「创建了个停着的容器」。
            engine
                .lifecycle(&id, ContainerLifecycleAction::Start)
                .await
                .inspect(|_| {
                    log::info!("容器已创建并启动，镜像: {}，新容器: {}", spec.image, id)
                })?;
            Ok(())
        }),
        ContainerCommand::RecreateContainer {
            tab_id,
            request_id,
            old_container_id,
            spec,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            // 1. 强制移除原容器（force: true 会自动处理停止状态，并保留外部挂载卷）
            let _ = engine
                .lifecycle(
                    &old_container_id,
                    ContainerLifecycleAction::Remove { force: true },
                )
                .await;
            // 2. 以新配置创建容器
            let id = engine.create_container(&spec).await?;
            // 3. 启动新容器
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
        }),
        _ => {}
    }
}

/// 处理镜像拉取、推送、标签、删除与构建命令。
fn handle_volume_and_network(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
    match command {
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
        _ => {}
    }
}
