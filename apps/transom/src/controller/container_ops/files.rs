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
                move |request_id, (bytes, truncated, size)| {
                    let binary = is_binary_preview(&bytes);
                    ContainerAppEvent::FilePreview {
                        request_id,
                        path: reply_path,
                        content: if binary {
                            String::new()
                        } else {
                            String::from_utf8_lossy(&bytes).to_string()
                        },
                        truncated,
                        size,
                        binary,
                    }
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
                move |request_id, (bytes, truncated, size)| {
                    let binary = is_binary_preview(&bytes);
                    ContainerAppEvent::FilePreview {
                        request_id,
                        path: reply_path,
                        content: if binary {
                            String::new()
                        } else {
                            String::from_utf8_lossy(&bytes).to_string()
                        },
                        truncated,
                        size,
                        binary,
                    }
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
        ContainerCommand::DownloadDirectory {
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
                transom_container::download_directory(
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
                transom_container::write_file_bytes(
                    &engine,
                    &container,
                    &path,
                    content.into_bytes(),
                )
                .await
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
                transom_container::write_volume_file_bytes(
                    &engine,
                    &volume,
                    &path,
                    content.into_bytes(),
                )
                .await
            },
            |request_id, _| ContainerAppEvent::ActionFinished {
                request_id,
                error: None,
            },
        ),
        ContainerCommand::SaveImage {
            tab_id,
            request_id,
            image,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::save_image(&engine, &image, &local_path, cancel, progress)
                    .await
                    .map(|_| ())
            },
        ),
        ContainerCommand::LoadImage {
            tab_id,
            request_id,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::load_image(&engine, &local_path, cancel, progress)
                    .await
                    .map(|_| ())
            },
        ),
        ContainerCommand::BackupVolume {
            tab_id,
            request_id,
            volume,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::backup_volume(&engine, &volume, &local_path, cancel, progress)
                    .await
                    .map(|_| ())
            },
        ),
        ContainerCommand::RestoreVolume {
            tab_id,
            request_id,
            volume,
            local_path,
        } => transfer(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine, cancel, progress| async move {
                transom_container::restore_volume(&engine, &volume, &local_path, cancel, progress)
                    .await
                    .map(|_| ())
            },
        ),
        _ => {}
    }
}

/// 判断预览内容是否为二进制：非法 UTF-8 或包含 NUL 都不允许进入文本编辑。
fn is_binary_preview(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err() || bytes.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::is_binary_preview;

    #[test]
    fn binary_preview_detection_rejects_invalid_utf8_and_nul() {
        assert!(!is_binary_preview(b"plain text"));
        assert!(!is_binary_preview("中文".as_bytes()));
        assert!(is_binary_preview(&[0xff, 0xfe]));
        assert!(is_binary_preview(b"text\0with-nul"));
    }
}
