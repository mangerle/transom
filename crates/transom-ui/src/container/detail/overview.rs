//! 基础信息总览与键值行。

use super::*;

/// 基础信息页签：结构化字段与原始 inspect 二选一。
///
/// 设计原理（Why）：原始 inspect 里就是上面这些字段的超集，两段同时铺开时
/// 用户要滚很久才能确认「下面没有别的信息」。切换按钮让同一屏只保留一种视图，
/// 想看原始 JSON 时不必离开页签。
pub(crate) fn render_container_overview(
    details: Option<&ContainerDetails>,
    notice: Option<&str>,
    raw: bool,
    p: &Palette,
) -> AnyElement {
    let Some(details) = details else {
        return centered_hint(rust_i18n::t!("container.detail.loading").to_string(), p);
    };
    let mut rows = v_flex().gap_1p5();
    if raw {
        let body: AnyElement = if details.inspect.is_empty() {
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.detail.no_inspect").to_string())
                .into_any_element()
        } else {
            div()
                .p_2()
                .rounded(p.radius.md)
                .bg(p.muted.opacity(0.3))
                .font_family(p.mono_font_family.clone())
                .text_size(p.mono_font_size)
                .child(details.inspect.clone())
                .into_any_element()
        };
        rows = rows.child(body);
        if let Some(notice) = notice {
            rows = rows.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(notice.to_string()),
            );
        }
        return scroll_body(rows, p).into_any_element();
    }
    rows = rows
        .child(kv(
            p,
            rust_i18n::t!("container.field.id"),
            short(&details.id),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.image"),
            details.image.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.state"),
            details.status.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.command"),
            details.command.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.entrypoint"),
            details.entrypoint.join(" "),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.user"),
            details.user.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.working_dir"),
            details.working_dir.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.restart"),
            details.restart_policy.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.created"),
            format_unix(details.created),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.started_at"),
            details.started_at.clone().unwrap_or_default(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.exit_code"),
            details
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_default(),
        ));
    let ports = details
        .ports
        .iter()
        .map(|port| match port.public_port {
            Some(public) => format!("{public}->{}/{}", port.private_port, port.protocol),
            None => format!("{}/{}", port.private_port, port.protocol),
        })
        .collect::<Vec<_>>()
        .join(", ");
    rows = rows.child(kv(p, rust_i18n::t!("container.field.ports"), ports));
    let mounts = details
        .mounts
        .iter()
        .map(|mount| format!("{} -> {}", mount.source, mount.destination))
        .collect::<Vec<_>>()
        .join("\n");
    rows = rows.child(kv(p, rust_i18n::t!("container.field.mounts"), mounts));
    rows = rows.child(kv(
        p,
        rust_i18n::t!("container.field.health"),
        details.health_label(),
    ));
    if !details.env.is_empty() {
        rows = rows.child(kv(
            p,
            rust_i18n::t!("container.field.env"),
            details.env.join("\n"),
        ));
    }
    if let Some(notice) = notice {
        rows = rows.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(notice.to_string()),
        );
    }
    scroll_body(rows, p).into_any_element()
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
