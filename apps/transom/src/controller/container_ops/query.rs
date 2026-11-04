//! 列表、详情与清理查询。

use super::*;

pub(super) fn handle(state: &mut ControllerState, ctx: &ControllerCtx, command: ContainerCommand) {
    match command {
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
        ContainerCommand::DiffContainer {
            tab_id,
            request_id,
            container,
        } => {
            let container_id = container.clone();
            one_shot(
                state,
                ctx,
                tab_id,
                request_id,
                move |engine| async move { engine.container_changes(&container).await },
                move |request_id, changes| ContainerAppEvent::ContainerDiffed {
                    request_id,
                    container: container_id,
                    changes,
                },
            )
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
        ContainerCommand::PruneVolumes { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.prune_volumes().await },
            |request_id, report| ContainerAppEvent::VolumesPruned { request_id, report },
        ),
        ContainerCommand::PruneImages { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.prune_images().await },
            |request_id, report| ContainerAppEvent::ImagesPruned { request_id, report },
        ),
        ContainerCommand::PruneNetworks { tab_id, request_id } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            |engine| async move { engine.prune_networks().await },
            |request_id, report| ContainerAppEvent::NetworksPruned { request_id, report },
        ),
        ContainerCommand::SearchImages {
            tab_id,
            request_id,
            term,
            limit,
        } => one_shot(
            state,
            ctx,
            tab_id,
            request_id,
            move |engine| async move { engine.search_images(&term, limit).await },
            |request_id, results| ContainerAppEvent::ImagesSearched { request_id, results },
        ),
        _ => {}
    }
}
