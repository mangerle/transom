//! 基础信息总览与键值行。

use super::*;
use crate::container::ContainerWorkspace;
use crate::theme::tokens::typography;
use crate::widgets::action_button;
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 基础信息页签：结构化字段与原始 inspect 二选一。
///
/// 设计原理（Why）：原始 inspect 里就是上面这些字段的超集，两段同时铺开时
/// 用户要滚很久才能确认「下面没有别的信息」。切换按钮让同一屏只保留一种视图，
/// 想看原始 JSON 时不必离开页签。
///
/// 渲染只持有工作区实体句柄与只读快照：复制提示写回实体的 `notice`，原始视图
/// 切换与端口转发分别走实体状态和工作区事件，不再依赖 `Context<AppShell>`。
pub(crate) fn render_container_overview(
    view: &Entity<ContainerWorkspace>,
    details: Option<&ContainerDetails>,
    raw: bool,
    inspect: InspectSearch<'_>,
    browser_host: &str,
    p: &Palette,
) -> AnyElement {
    let Some(details) = details else {
        return centered_hint(rust_i18n::t!("container.detail.loading").to_string(), p);
    };
    if raw {
        return render_raw_inspect(view, &details.inspect, inspect, p);
    }
    let cards = v_flex()
        .gap_3()
        .child(render_identity_card(view, details, p))
        .child(render_ports_card(view, details, browser_host, p))
        .child(render_mounts_overview_card(view, details, p));
    let cards = render_optional_overview_cards(cards, view, details, p);
    scroll_body(cards, p).into_any_element()
}

fn render_identity_card(
    view: &Entity<ContainerWorkspace>,
    details: &ContainerDetails,
    p: &Palette,
) -> AnyElement {
    use super::overview_cards::*;
    let mut fields = v_flex()
        .gap_1p5()
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.id"),
            short(&details.id),
            true,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.image"),
            details.image.clone(),
            true,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.state"),
            details.status.clone(),
            false,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.command"),
            details.command.clone(),
            true,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.entrypoint"),
            details.entrypoint.join(" "),
            true,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.user"),
            details.user.clone().unwrap_or_default(),
            false,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.working_dir"),
            details.working_dir.clone().unwrap_or_default(),
            true,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.restart"),
            details.restart_policy.clone().unwrap_or_default(),
            false,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.created"),
            format_unix(details.created),
            false,
            p,
        ))
        .child(copyable_field(
            view,
            rust_i18n::t!("container.field.started_at"),
            details.started_at.clone().unwrap_or_default(),
            false,
            p,
        ));
    if let Some(code) = details.exit_code {
        fields = fields.child(copyable_field(
            view,
            rust_i18n::t!("container.field.exit_code"),
            code.to_string(),
            true,
            p,
        ));
    }
    overview_card(
        rust_i18n::t!("container.detail.category_identity").to_string(),
        AppIcon::Custom(CustomIcon::INFO),
        p,
        Some(render_raw_inspect_button(view)),
        fields,
    )
}

fn render_raw_inspect_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let view = view.clone();
    action_button(
        "container-overview-raw-inspect",
        UiIcon::EDITOR_ONLY,
        rust_i18n::t!("container.field.inspect").to_string(),
    )
    .ghost()
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.state.overview_raw = true;
            cx.notify();
        });
    })
    .into_any_element()
}

fn render_ports_card(
    view: &Entity<ContainerWorkspace>,
    details: &ContainerDetails,
    browser_host: &str,
    p: &Palette,
) -> AnyElement {
    use super::overview_cards::*;
    let name = details.name.trim_start_matches('/');
    let ports = render_port_chips(view, &details.ports, browser_host, name, p);
    overview_card(
        rust_i18n::t!("container.field.ports").to_string(),
        AppIcon::Custom(CustomIcon::PORT),
        p,
        None,
        ports,
    )
}

fn render_mounts_overview_card(
    view: &Entity<ContainerWorkspace>,
    details: &ContainerDetails,
    p: &Palette,
) -> AnyElement {
    use super::overview_cards::*;
    overview_card(
        rust_i18n::t!("container.field.mounts").to_string(),
        AppIcon::Custom(CustomIcon::MOUNT),
        p,
        None,
        render_mounts_card(view, &details.mounts, p),
    )
}

fn render_optional_overview_cards(
    mut cards: Div,
    view: &Entity<ContainerWorkspace>,
    details: &ContainerDetails,
    p: &Palette,
) -> Div {
    use super::overview_cards::*;
    if let Some(health) = &details.health_details {
        cards = cards.child(render_health_timeline(view, health, p));
    }
    if !details.env.is_empty() {
        cards = cards.child(render_env_card(view, &details.env, p));
    }
    cards
}

/// 关联容器行：卷、网络与编排详情里点名字直接跳到那个容器。
///
/// 设计原理（Why）：这三类详情天然带「谁在用我」的关系，原先只把名字拼成一段
/// 文本，用户看到也只能再回列表用搜索找；跳转由工作区实体的 `reveal_container`
/// 负责切分类与延迟选中，因此这里只需要一个按钮。渲染只持有实体句柄，点击后
/// 在实体内完成选择与刷新，不再依赖外壳上下文。
pub(crate) fn linked_containers(
    view: &Entity<ContainerWorkspace>,
    title: String,
    items: Vec<(String, String)>,
    p: &Palette,
) -> AnyElement {
    let mut block = v_flex().gap_1().child(
        div()
            .text_size(rems(typography::CAPTION))
            .text_color(p.muted_foreground)
            .child(title),
    );
    if items.is_empty() {
        block = block.child(
            div()
                .text_size(rems(typography::BODY))
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.linked.none").to_string()),
        );
    }
    for (name, hint) in items {
        let reveal_name = name.clone();
        let link_view = view.clone();
        let mut row = h_flex().gap_2().items_center().child(
            action_button(
                ElementId::Name(format!("container-link-{name}").into()),
                DomainIcon::CONTAINER,
                name,
            )
            .ghost()
            .on_click(move |_, _, cx| {
                link_view.update(cx, |workspace, cx| {
                    workspace.reveal_container(reveal_name.clone(), cx);
                });
            }),
        );
        if !hint.is_empty() {
            row = row.child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.muted_foreground)
                    .child(hint),
            );
        }
        block = block.child(row);
    }
    block.into_any_element()
}

pub(crate) fn kv(p: &Palette, key: impl Into<String>, value: impl Into<String>) -> AnyElement {
    let key = key.into();
    let value = value.into();
    let value = if value.trim().is_empty() {
        "—".to_string()
    } else {
        value
    };
    h_flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(key),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(rems(typography::BODY))
                .child(value),
        )
        .into_any_element()
}

pub(crate) fn scroll_body(content: impl IntoElement, _p: &Palette) -> Div {
    v_flex().child(
        div()
            .id("container-detail-scroll")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .child(content),
    )
}

pub(crate) fn centered_hint(message: String, p: &Palette) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(p.muted_foreground)
        .child(message)
        .into_any_element()
}

pub(crate) fn format_unix(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub(crate) fn short(id: &str) -> String {
    transom_core::short_id(id)
}
