//! 容器文件浏览、预览与传输。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
        ContainerCommand::ListDir { .. }
        | ContainerCommand::ListVolumeDir { .. }
        | ContainerCommand::ReadFile { .. }
        | ContainerCommand::ReadVolumeFile { .. }
        | ContainerCommand::SaveFile { .. }
        | ContainerCommand::SaveVolumeFile { .. } => {
            handle_preview_and_save(state, ctx, command);
        }
        ContainerCommand::Download { .. }
        | ContainerCommand::DownloadVolumeFile { .. }
        | ContainerCommand::DownloadDirectory { .. }
        | ContainerCommand::Upload { .. }
        | ContainerCommand::UploadDirectory { .. } => {
            super::file_transfers::handle_transfers(state, ctx, command);
        }
        ContainerCommand::RemovePath { .. }
        | ContainerCommand::CopyPath { .. }
        | ContainerCommand::ChmodPath { .. }
        | ContainerCommand::ChownPath { .. }
        | ContainerCommand::CreateDir { .. } => {
            handle_path_mutations(state, ctx, command);
        }
        ContainerCommand::SaveImage { .. }
        | ContainerCommand::LoadImage { .. }
        | ContainerCommand::BackupVolume { .. }
        | ContainerCommand::RestoreVolume { .. } => {
            super::file_transfers::handle_archive_ops(state, ctx, command);
        }
        _ => {}
    }
}

/// 构造统一的文件预览事件，容器与存储卷预览共用。
fn build_file_preview_event(
    request_id: ContainerRequestId,
    path: String,
    bytes: Vec<u8>,
    truncated: bool,
    size: u64,
) -> ContainerAppEvent {
    let binary = is_binary_preview(&bytes);
    ContainerAppEvent::FilePreview {
        request_id,
        path,
        content: if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&bytes).to_string()
        },
        truncated,
        size,
        binary,
    }
}

/// 处理目录列举、文件预览与小文件保存命令。
fn handle_preview_and_save(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
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
            );
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
            );
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
                    build_file_preview_event(request_id, reply_path, bytes, truncated, size)
                },
            );
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
                    build_file_preview_event(request_id, reply_path, bytes, truncated, size)
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
        _ => {}
    }
}

/// 处理容器与存储卷文件的上传/下载传输命令。
fn handle_path_mutations(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
    match command {
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
        ContainerCommand::CopyPath {
            tab_id,
            request_id,
            container,
            from,
            to,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            let cancel = Arc::new(AtomicBool::new(false));
            transom_container::copy(&engine, &container, &from, &to, &cancel).await
        }),
        ContainerCommand::ChmodPath {
            tab_id,
            request_id,
            container,
            path,
            mode,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            let cancel = Arc::new(AtomicBool::new(false));
            transom_container::chmod(&engine, &container, &path, &mode, &cancel).await
        }),
        ContainerCommand::ChownPath {
            tab_id,
            request_id,
            container,
            path,
            owner,
        } => run_action(state, ctx, tab_id, request_id, move |engine| async move {
            let cancel = Arc::new(AtomicBool::new(false));
            transom_container::chown(&engine, &container, &path, &owner, &cancel).await
        }),
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
        _ => {}
    }
}

/// 处理镜像归档导出/导入与存储卷备份/恢复命令。
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
