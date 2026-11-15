//! 容器文件上传、下载与归档传输命令处理。

//! 容器文件浏览、预览与传输。

use super::*;

pub(super) fn handle_transfers(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
    match command {
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
        ContainerCommand::UploadDirectory {
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
                transom_container::upload_directory(
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
        _ => {}
    }
}

/// 处理容器内路径删除、复制、改权与建目录命令。
pub(super) fn handle_archive_ops(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: ContainerCommand,
) {
    match command {
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
