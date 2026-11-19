//! 右侧详情区渲染：空态、头部与按页签分发的内容。
//!
//! 设计原理（Why）：详情区涉及容器/镜像/卷/网络/编排五类对象与六个页签，
//! 原先单个函数同时承担数据快照、标题、头部与内容分发。这里按阶段拆开，
//! 每个函数只做一件事，数据从 `ContainerWorkspaceState` 就地读取，避免在
//! 一个函数里堆满几十个局部快照。

use super::*;

/// 渲染容器详情区。
pub(crate) fn render_container_detail(
    params: ContainerDetailParams<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerDetailParams {
        tab_id,
        view,
        workspace,
        compose_yaml_text,
        compose_yaml_line_numbers,
        logs_filter,
        changes_search_value,
        inspect_keyword,
    } = params;
    let section = workspace.section;
    if workspace.selected.is_none() {
        return render_empty_detail(section, p);
    }
    let ctx = DetailContentCtx {
        tab_id,
        view,
        workspace,
        section,
        compose_yaml_text,
        compose_yaml_line_numbers,
        logs_filter,
        changes_search_value,
        inspect_keyword,
    };
    let header = render_detail_header_block(&ctx, p);
    let content = render_detail_content(&ctx, p);
    v_flex()
        .size_full()
        .overflow_hidden()
        .child(header)
        .child(div().flex_1().min_h_0().overflow_hidden().child(content))
        .into_any_element()
}

/// 内容分发所需的只读上下文。
struct DetailContentCtx<'a> {
    tab_id: TabId,
    view: &'a Entity<ContainerWorkspace>,
    workspace: &'a ContainerWorkspaceState,
    section: ContainerSection,
    compose_yaml_text: &'a str,
    compose_yaml_line_numbers: compose_yaml::LineNumberMetrics,
    logs_filter: &'a str,
    changes_search_value: &'a str,
    inspect_keyword: &'a str,
}

fn render_empty_detail(section: ContainerSection, p: &Palette) -> AnyElement {
    let (icon, title_key, desc_key) = match section {
        ContainerSection::Containers => (
            AppIcon::Domain(DomainIcon::CONTAINER),
            "container.empty.containers_title",
            "container.empty.containers_desc",
        ),
        ContainerSection::Images => (
            AppIcon::Custom(CustomIcon::IMAGE),
            "container.empty.images_title",
            "container.empty.images_desc",
        ),
        ContainerSection::Volumes => (
            AppIcon::Custom(CustomIcon::VOLUME),
            "container.empty.volumes_title",
            "container.empty.volumes_desc",
        ),
        ContainerSection::Networks => (
            AppIcon::Custom(CustomIcon::NETWORK),
            "container.empty.networks_title",
            "container.empty.networks_desc",
        ),
        ContainerSection::Compose => (
            AppIcon::Domain(DomainIcon::CONTAINER),
            "container.empty.compose_title",
            "container.empty.compose_desc",
        ),
    };
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .p_6()
        .child(crate::widgets::empty_state(
            icon,
            rust_i18n::t!(title_key).to_string(),
            rust_i18n::t!(desc_key).to_string(),
            p,
        ))
        .into_any_element()
}

fn render_detail_header_block(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let selected = ctx.workspace.selected.clone();
    let selected_container = ctx
        .workspace
        .containers
        .iter()
        .find(|item| selected.as_deref() == Some(item.id.as_str()))
        .cloned();
    let selected_image = ctx.workspace.selected_image().cloned();
    let selected_volume = ctx.workspace.selected_volume().cloned();
    let selected_network = ctx.workspace.selected_network().cloned();
    let selected_compose = ctx.workspace.selected_compose().cloned();
    let title = detail_title(ctx);
    let tabs = render_detail_tab_buttons(
        ctx.view,
        ctx.workspace,
        ctx.section,
        ctx.workspace.detail_tab,
        p,
    );
    render_detail_header(
        ContainerHeaderView {
            view: ctx.view,
            section: ctx.section,
            title,
            tabs,
            detail_tab: ctx.workspace.detail_tab,
            container: &selected_container,
            details: &ctx.workspace.details,
            image: &selected_image,
            volume: &selected_volume,
            network: &selected_network,
            compose: &selected_compose,
            all_containers: &ctx.workspace.containers,
            timeline: if ctx.section == ContainerSection::Containers {
                selected_container.as_ref().map(|c| {
                    timeline::StatusTimeline::infer(ctx.workspace.details.as_ref(), c.state)
                })
            } else {
                None
            },
        },
        p,
    )
}

fn detail_title(ctx: &DetailContentCtx<'_>) -> String {
    let workspace = ctx.workspace;
    let title = match ctx.section {
        ContainerSection::Containers => workspace
            .containers
            .iter()
            .find(|item| workspace.selected.as_deref() == Some(item.id.as_str()))
            .map(|item| item.name.clone()),
        ContainerSection::Images => workspace.selected_image().map(|i| i.display_name()),
        ContainerSection::Volumes => workspace.selected_volume().map(|i| i.name.clone()),
        ContainerSection::Networks => workspace.selected_network().map(|i| i.name.clone()),
        ContainerSection::Compose => workspace.selected_compose().map(|i| i.name.clone()),
    };
    title.unwrap_or_else(|| rust_i18n::t!("container.detail.no_selection").to_string())
}

fn render_detail_content(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    match ctx.section {
        ContainerSection::Containers => render_containers_content(ctx, p),
        ContainerSection::Compose => render_compose_content(ctx, p),
        ContainerSection::Images => render_image_content(ctx, p),
        ContainerSection::Volumes => render_volume_content(ctx, p),
        ContainerSection::Networks => render_network_content(ctx, p),
    }
}

fn render_containers_content(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    match ctx.workspace.detail_tab {
        ContainerDetailTab::Logs => render_container_logs(ctx, p),
        ContainerDetailTab::Terminal => render_container_terminal(ctx, p),
        ContainerDetailTab::Stats => render_container_stats_tab(ctx, p),
        ContainerDetailTab::Files => render_container_files(ctx, p),
        ContainerDetailTab::Changes => render_container_changes_tab(ctx, p),
        _ => render_container_overview_tab(ctx, p),
    }
}

fn render_container_overview_tab(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let host = ctx.workspace.browser_host();
    render_container_overview(
        ctx.view,
        ctx.workspace.details.as_ref(),
        ctx.workspace.overview_raw,
        InspectSearch {
            input: &ctx.workspace.inspect_search,
            keyword: ctx.inspect_keyword,
        },
        &host,
        p,
    )
}

fn render_container_logs(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    render_logs(
        ctx.view,
        ContainerLogsView {
            logs: &workspace.logs,
            follow: workspace.logs_follow,
            tail: workspace.logs_tail,
            timestamps: workspace.logs_timestamps,
            reversed: workspace.logs_reversed,
            soft_wrap: workspace.logs_soft_wrap,
            scroll: &workspace.logs_scroll,
            search: &workspace.logs_search,
            filter: ctx.logs_filter,
        },
        p,
    )
}

fn render_container_terminal(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    let terminal = workspace
        .terminal
        .as_ref()
        .map(|terminal| terminal.view.clone());
    let user = workspace
        .terminal
        .as_ref()
        .map(|terminal| terminal.user.clone())
        .or_else(|| {
            workspace
                .terminal_request
                .as_ref()
                .map(|(_, user, _)| user.clone())
        });
    let shell = workspace
        .terminal
        .as_ref()
        .map(|terminal| terminal.shell.clone())
        .or_else(|| {
            workspace
                .terminal_request
                .as_ref()
                .map(|(_, _, shell)| shell.clone())
        });
    render_terminal(
        ctx.view,
        terminal::ContainerTerminalView {
            terminal,
            user: user.as_ref(),
            shell: shell.as_ref(),
        },
        p,
    )
}

fn render_container_stats_tab(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    let host = workspace.browser_host();
    let history: Vec<ContainerStats> = workspace.stats_history.iter().cloned().collect();
    let selected_container = workspace
        .containers
        .iter()
        .find(|item| workspace.selected.as_deref() == Some(item.id.as_str()));
    let is_running = selected_container
        .map(|c| c.state.is_running())
        .unwrap_or(false);
    crate::container::stats::render_container_stats(
        ctx.view,
        crate::container::stats::ContainerStatsView {
            stats: workspace.stats.as_ref(),
            history: &history,
            chart: workspace.stats_chart,
            interval: workspace.stats_interval,
            error: workspace.stats_error.as_deref(),
            details: workspace.details.as_ref(),
            browser_host: &host,
            processes: &workspace.stats_processes,
            processes_loading: workspace.stats_processes_loading,
            processes_error: workspace.stats_processes_error.as_deref(),
            is_running,
            has_selection: workspace.selected.is_some(),
            ports_collapsed: workspace.stats_ports_collapsed,
            processes_collapsed: workspace.stats_processes_collapsed,
        },
        p,
    )
}

fn render_container_files(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    render_files(
        ctx.view,
        ContainerFilesView {
            path: &workspace.files.path,
            entries: &workspace.files.entries,
            preview: workspace.files.preview.as_ref(),
            preview_editor: workspace.files.preview_editor.as_ref(),
            preview_saving: workspace.files.preview_saving,
            preview_binary: workspace.files.preview_binary,
            loading: workspace.files.loading,
            scroll: &workspace.files.scroll,
            readonly: ctx.section == ContainerSection::Volumes,
        },
        p,
    )
}

fn render_container_changes_tab(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    changes::render_container_changes(
        ctx.view,
        changes::ContainerChangesView {
            tab_id: ctx.tab_id,
            container_id: workspace.selected.as_deref(),
            changes_data: workspace.changes.as_ref(),
            loading: workspace.changes_loading,
            search_input: &workspace.changes_search,
            search_keyword: ctx.changes_search_value,
        },
        p,
    )
}

fn render_compose_content(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    render_compose_detail(
        ctx.view,
        compose::ComposeDetailParams {
            project: workspace.selected_compose(),
            containers: &workspace.containers,
            detail_tab: workspace.detail_tab,
            compose_yaml_editor: workspace.compose_yaml_editor.as_ref(),
            compose_yaml_text: ctx.compose_yaml_text,
            compose_yaml_line_numbers: ctx.compose_yaml_line_numbers,
            dependencies: &workspace.compose_dependencies,
            hovered_service: workspace.compose_hovered_service.as_deref(),
        },
        p,
    )
}

fn render_image_content(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    render_image_detail(
        ctx.view,
        image::ImageDetailView {
            summary: workspace.selected_image(),
            details: workspace.image_details.as_ref(),
            overview_raw: workspace.overview_raw,
            detail_tab: workspace.detail_tab,
            inspect: InspectSearch {
                input: &workspace.inspect_search,
                keyword: ctx.inspect_keyword,
            },
        },
        p,
    )
}

fn render_volume_content(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    render_volume_detail(
        ctx.view,
        ContainerVolumeView {
            summary: workspace.selected_volume(),
            details: workspace.volume_details.as_ref(),
            overview_raw: workspace.overview_raw,
            inspect: InspectSearch {
                input: &workspace.inspect_search,
                keyword: ctx.inspect_keyword,
            },
        },
        p,
    )
}

fn render_network_content(ctx: &DetailContentCtx<'_>, p: &Palette) -> AnyElement {
    let workspace = ctx.workspace;
    render_network_detail(
        ctx.view,
        ContainerNetworkView {
            summary: workspace.selected_network(),
            details: workspace.network_details.as_ref(),
            overview_raw: workspace.overview_raw,
            inspect: InspectSearch {
                input: &workspace.inspect_search,
                keyword: ctx.inspect_keyword,
            },
        },
        p,
    )
}
