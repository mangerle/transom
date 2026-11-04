//! 容器文件浏览、预览与传输。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
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
        ContainerCommand::ListVolumeDir {
            tab_id,
            request_id,
            volume,
            path,
        } => {
            let reply_path = path.clone();
            one_shot(
                state,
                ctx,
                tab_id,
                request_id,
                move |engine| async move {
                    transom_container::list_volume_dir(&engine, &volume, &path).await
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
        ContainerCommand::ReadVolumeFile {
            tab_id,
            request_id,
            volume,
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
                    transom_container::read_volume_file(&engine, &volume, &path, max_bytes).await
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
        ContainerCommand::DownloadVolumeFile {
            tab_id,
            request_id,
            volume,
            path,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::download_volume_file(
                    &engine,
                    &volume,
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
        ContainerCommand::SaveFile {
            tab_id,
            request_id,
            container,
            path,
            content,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move {
                transom_container::write_file_bytes(&engine, &container, &path, content.into_bytes()).await
            },
            |request_id, _| ContainerAppEvent::ActionFinished {
                request_id,
                error: None,
            },
        ),
        ContainerCommand::SaveVolumeFile {
            tab_id,
            request_id,
            volume,
            path,
            content,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move {
                transom_container::write_volume_file_bytes(&engine, &volume, &path, content.into_bytes()).await
            },
            |request_id, _| ContainerAppEvent::ActionFinished {
                request_id,
                error: None,
            },
        ),
        _ => {}
    }
}
