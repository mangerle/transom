//! 编排项目详情。

use super::*;

pub(crate) fn render_compose_detail(
    tab_id: TabId,
    project: Option<&ComposeProject>,
    containers: &[ContainerSummary],
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(project) = project else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };
    let members: Vec<&ContainerSummary> = containers
        .iter()
        .filter(|item| item.compose_project.as_deref() == Some(project.name.as_str()))
        .collect();
    let log_bar = h_flex().pb_1().child(
        Button::new("compose-project-logs")
            .primary()
            .small()
            .icon(IconName::FileText)
            .label(rust_i18n::t!("container.compose.logs").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(w) = this.state.container_workspace_mut(tab_id) {
                    w.detail_tab = ContainerDetailTab::Logs;
                }
                this.ensure_container_logs(tab_id);
                cx.notify();
            })),
    );
    let mut rows = v_flex().gap_1p5();
    rows = rows
        .child(log_bar)
        .child(kv(
            p,
            rust_i18n::t!("container.field.state"),
            project.state_label(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.working_dir"),
            project.working_dir.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.config_files"),
            project.config_files.join("\n"),
        ));
    if members.is_empty() {
        rows = rows.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.compose.no_containers").to_string()),
        );
    }
    let member_items = members
        .iter()
        .map(|member| {
            (
                member.name.clone(),
                format!("{} · {}", member.state.key(), member.status),
            )
        })
        .collect();
    rows = rows.child(linked_containers(
        tab_id,
        rust_i18n::t!("container.field.containers").to_string(),
        member_items,
        p,
        cx,
    ));
    scroll_body(rows, p).into_any_element()
}
