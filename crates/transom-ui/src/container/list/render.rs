//! 列表行渲染与底部状态条。

use super::*;
use crate::widgets::status_dot_bare;

/// 渲染列表；空结果给出可行动的提示而不是白屏。
pub(crate) fn render_rows(
    tab_id: TabId,
    rows: &[ListRow],
    selected: &Option<String>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if rows.is_empty() {
        return v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .p_4()
            .child(crate::widgets::empty_state(
                IconName::Search,
                rust_i18n::t!("container.list.none").to_string(),
                rust_i18n::t!("container.list.none_hint").to_string(),
                p,
            ))
            .into_any_element();
    }
    let app = cx.entity().clone();
    let mut list = v_flex().gap_0p5().w_full();
    for row in rows {
        let is_selected = selected.as_deref() == Some(row_key(&row.target));
        list = list.child(list_row(&app, tab_id, row, is_selected, p, cx));
    }
    div()
        .id("container-list-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(list)
        .into_any_element()
}

/// 单行渲染：状态点、标题副标题、快捷按钮与右键菜单。
pub(crate) fn list_row(
    app: &Entity<AppShell>,
    tab_id: TabId,
    row: &ListRow,
    selected: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let group = format!("container-list-row-{}", row.key);
    let title = row.title.clone();
    let mut element = h_flex()
        .id(ElementId::Name(row.key.clone().into()))
        .group(group.as_str())
        .w_full()
        .h(px(40.0))
        .relative()
        .gap_2()
        .px_2p5()
        .rounded(p.radius.sm)
        .items_center()
        .cursor_pointer();

    if selected {
        element = element
            .bg(p.primary.opacity(0.06))
            .text_color(p.foreground)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(6.0))
                    .bottom(px(6.0))
                    .w(px(2.0))
                    .rounded_full()
                    .bg(p.primary),
            );
    } else {
        element = element
            .text_color(p.foreground)
            .hover(|style| style.bg(p.list_hover));
    }

    // 运行态指示点
    if row.running.is_some() {
        element = element.child(render_status_dot(row, p));
    }

    let title_el = div()
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .truncate()
        .child(title);

    let names = v_flex().flex_1().min_w_0().gap_0p5().child(title_el);

    element = element.child(match &row.subtitle {
        Some(subtitle) => names
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(p.muted_foreground)
                    .font_family(p.mono_font_family.clone())
                    .truncate()
                    .child(subtitle.clone()),
            )
            .into_any_element(),
        None => names.into_any_element(),
    });

    if let Some(badge) = row.badge.clone() {
        element = element.child(
            div()
                .text_size(px(11.0))
                .flex_none()
                .text_color(p.muted_foreground)
                .child(badge),
        );
    }

    // 行内快捷动作按钮：仅在 hover 或选中时显现
    let actions = quick_actions(row);
    if !actions.is_empty() {
        let mut actions_flex = h_flex().gap_0p5().items_center().flex_none();
        for action in actions {
            let target = row.target.clone();
            actions_flex = actions_flex.child(
                Button::new(ElementId::Name(
                    format!("{}-action-{:?}", row.key, action).into(),
                ))
                .ghost()
                .xsmall()
                .icon(action.icon())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.run_row_action(tab_id, target.clone(), action, window, cx);
                })),
            );
        }

        let actions_container = if selected {
            actions_flex.into_any_element()
        } else {
            actions_flex
                .opacity(0.0)
                .group_hover(group.as_str(), |style| style.opacity(1.0))
                .into_any_element()
        };
        element = element.child(actions_container);
    }

    let menu_target = row.target.clone();
    let menu_actions = row_actions(row);
    let menu_app = app.clone();
    let click_target = row.target.clone();
    let element = element.on_click(cx.listener(move |this, _, _, cx| {
        this.select_row(tab_id, &click_target, cx);
    }));

    element
        .context_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for action in menu_actions.clone() {
                if action.is_danger() {
                    menu = menu.item(PopupMenuItem::separator());
                }
                let target = menu_target.clone();
                let entity = menu_app.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!(action.label_key()).to_string(),
                    action.icon(),
                    move |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.run_row_action(tab_id, target.clone(), action, window, cx);
                        });
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 状态色点：接入设计系统状态圆点。
pub(crate) fn render_status_dot(row: &ListRow, p: &Palette) -> AnyElement {
    let color = match row.running {
        Some(true) => p.success,
        Some(false) => p.muted_foreground,
        None => p.border,
    };
    status_dot_bare(color).into_any_element()
}
