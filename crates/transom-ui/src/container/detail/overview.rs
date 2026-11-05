//! 基础信息总览与键值行。

use super::*;

/// 基础信息页签：结构化字段与原始 inspect 二选一。
///
/// 设计原理（Why）：原始 inspect 里就是上面这些字段的超集，两段同时铺开时
/// 用户要滚很久才能确认「下面没有别的信息」。切换按钮让同一屏只保留一种视图，
/// 想看原始 JSON 时不必离开页签。
pub(crate) fn render_container_overview(
    tab_id: TabId,
    details: Option<&ContainerDetails>,
    raw: bool,
    search_input: &Entity<InputState>,
    browser_host: &str,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(details) = details else {
        return centered_hint(rust_i18n::t!("container.detail.loading").to_string(), p);
    };
    if raw {
        return render_raw_inspect(tab_id, &details.inspect, search_input, p, cx);
    }

    use super::overview_cards::*;

    let mut cards = v_flex().gap_3();

    // 1. 运行身份与运行时
    let mut runtime_fields = v_flex().gap_1p5();
    runtime_fields = runtime_fields
        .child(copyable_field(rust_i18n::t!("container.field.id"), short(&details.id), true, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.image"), details.image.clone(), true, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.state"), details.status.clone(), false, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.command"), details.command.clone(), true, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.entrypoint"), details.entrypoint.join(" "), true, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.user"), details.user.clone().unwrap_or_default(), false, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.working_dir"), details.working_dir.clone().unwrap_or_default(), true, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.restart"), details.restart_policy.clone().unwrap_or_default(), false, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.created"), format_unix(details.created), false, p, cx))
        .child(copyable_field(rust_i18n::t!("container.field.started_at"), details.started_at.clone().unwrap_or_default(), false, p, cx));

    if let Some(code) = details.exit_code {
        runtime_fields = runtime_fields.child(copyable_field(rust_i18n::t!("container.field.exit_code"), code.to_string(), true, p, cx));
    }

    cards = cards.child(overview_card(
        rust_i18n::t!("container.detail.category_identity").to_string(),
        IconName::Info,
        p,
        None,
        runtime_fields,
    ));

    // 2. 网络与端口映射
    let container_name = details.name.trim_start_matches('/');
    let port_chips = render_port_chips(tab_id, &details.ports, browser_host, container_name, p, cx);
    cards = cards.child(overview_card(
        rust_i18n::t!("container.field.ports").to_string(),
        IconName::Network,
        p,
        None,
        port_chips,
    ));

    // 3. 存储挂载
    let mounts_el = render_mounts_card(&details.mounts, p);
    cards = cards.child(overview_card(
        rust_i18n::t!("container.field.mounts").to_string(),
        IconName::HardDrive,
        p,
        None,
        mounts_el,
    ));

    // 4. 健康检查时间线（如果有）
    if let Some(health_diag) = &details.health_details {
        cards = cards.child(render_health_timeline(health_diag, p));
    }

    // 5. 环境变量
    if !details.env.is_empty() {
        cards = cards.child(render_env_card(&details.env, p, cx));
    }

    scroll_body(cards, p).into_any_element()
}

/// 关联容器行：卷、网络与编排详情里点名字直接跳到那个容器。
///
/// 设计原理（Why）：这三类详情天然带「谁在用我」的关系，原先只把名字拼成一段
/// 文本，用户看到也只能再回列表用搜索找；跳转由 `reveal_container` 负责切分类与
/// 延迟选中，因此这里只需要一个按钮。
pub(crate) fn linked_containers(
    tab_id: TabId,
    title: String,
    items: Vec<(String, String)>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut block = v_flex()
        .gap_0p5()
        .child(div().text_xs().text_color(p.muted_foreground).child(title));
    if items.is_empty() {
        block = block.child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.linked.none").to_string()),
        );
    }
    for (name, hint) in items {
        let reveal_name = name.clone();
        let mut row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Icon::new(IconName::Container)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(
                Button::new(ElementId::Name(
                    format!("container-link-{name}-{tab_id}").into(),
                ))
                .ghost()
                .xsmall()
                .label(name)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reveal_container(tab_id, reveal_name.clone(), cx);
                })),
            );
        if !hint.is_empty() {
            row = row.child(div().text_xs().text_color(p.muted_foreground).child(hint));
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
                .text_xs()
                .text_color(p.muted_foreground)
                .child(key),
        )
        .child(div().flex_1().min_w_0().text_sm().child(value))
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

