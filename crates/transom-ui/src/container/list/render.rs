//! 列表行渲染与底部状态条。

use super::*;

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
            .px_2()
            .gap_1()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(Icon::new(IconName::Search).xsmall())
            .child(rust_i18n::t!("container.list.none").to_string())
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
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(p.radius.md)
        .items_center()
        .cursor_pointer()
        .hover(|style| style.bg(p.muted.opacity(0.5)));
    // 行首不再放类型图标：中栏已经按分类分页，每一行的类型是确定的，图标只是噪声。
    // 状态点也只给有运行态的条目，静态资源那枚灰点是纯装饰。
    if row.running.is_some() {
        element = element.child(status_dot(row, p));
    }
    let names = v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(div().text_sm().truncate().child(title));
    element = element.child(match &row.subtitle {
        Some(subtitle) => names
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .truncate()
                    .child(subtitle.clone()),
            )
            .into_any_element(),
        None => names.into_any_element(),
    });
    if let Some(badge) = row.badge.clone() {
        element = element.child(
            div()
                .text_xs()
                .flex_none()
                .text_color(p.muted_foreground)
                .child(badge),
        );
    }
    for action in quick_actions(row) {
        let target = row.target.clone();
        element = element.child(
            Button::new(ElementId::Name(
                format!("{}-action-{:?}", row.key, action).into(),
            ))
            .ghost()
            .xsmall()
            .icon(action.icon())
            // 同理：行内动作不该因为冒泡再触发一次「选中该行」。
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.run_row_action(tab_id, target.clone(), action, window, cx);
            })),
        );
    }
    // 勾选放在动作之后：列表扫描时先看到「这个容器能做什么」，多选是次要操作。
    if let Some(is_checked) = row.checked {
        let check_id = row_target_key_id(&row.target);
        element = element.child(
            Checkbox::new(ElementId::Name(format!("{}-check", row.key).into()))
                .small()
                .checked(is_checked)
                // 勾选只改勾选集合：不吞掉冒泡的话，行自身的点击会顺手把右侧详情
                // 切到这个容器，正在看的日志与统计就被清了。
                .on_click(cx.listener(move |this, &_, _, cx| {
                    cx.stop_propagation();
                    this.toggle_container_check(tab_id, check_id.clone(), cx);
                })),
        );
    }
    if selected {
        element = element
            .bg(p.sidebar_accent)
            .text_color(p.sidebar_accent_foreground);
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
                // 破坏性动作与常规动作之间加分隔线，降低误点删除的概率。
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

/// 状态色点：运行用主色、停止用弱色，静态资源用边框色。
pub(crate) fn status_dot(row: &ListRow, p: &Palette) -> AnyElement {
    let color = match row.running {
        Some(true) => p.success,
        Some(false) => p.muted_foreground,
        None => p.border,
    };
    div()
        .size_2()
        .flex_none()
        .rounded_full()
        .bg(color)
        .into_any_element()
}

/// 勾选行需要的主键：容器用 ID，与 `selected` 的取法一致。
pub(crate) fn row_target_key_id(target: &RowTarget) -> String {
    row_key(target).to_string()
}

/// 批量条：高度固定常驻，没有勾选时只占位不绘制内容。
///
/// 设计原理（Why）：条件渲染会让中栏在勾选/取消时上下跳一截，肉眼就是闪烁；
/// 固定高度后只有条内的文字与按钮切换，列表位置不变。
pub(crate) fn render_status_bar(
    tab_id: TabId,
    section: ContainerSection,
    count: usize,
    armed: bool,
    notice: Option<&str>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut bar = h_flex()
        .gap_1()
        .items_center()
        .h(px(28.0))
        .px_2()
        .rounded(p.radius.md);
    if section != ContainerSection::Containers || count == 0 {
        // 没有批量选择时，这同一行用来放操作状态，不再额外加一条提示带。
        let Some(notice) = notice else {
            return bar.into_any_element();
        };
        return bar
            .bg(p.muted.opacity(0.35))
            .child(
                Icon::new(IconName::Info)
                    .xsmall()
                    .text_color(p.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .truncate()
                    .child(notice.to_string()),
            )
            .child(
                Button::new("container-notice-dismiss")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(rust_i18n::t!("container.notice.dismiss").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismiss_container_notice(tab_id, cx);
                    })),
            )
            .into_any_element();
    }
    bar = bar.bg(p.muted.opacity(0.35)).child(
        div()
            .text_xs()
            .child(rust_i18n::t!("container.batch.selected", count = count).to_string()),
    );
    for action in [
        ContainerBatchAction::Start,
        ContainerBatchAction::Stop,
        ContainerBatchAction::Remove,
    ] {
        let label = if action.is_dangerous() && armed {
            rust_i18n::t!("container.batch.confirm", count = count).to_string()
        } else {
            rust_i18n::t!(action.label_key()).to_string()
        };
        bar = bar.child(
            Button::new(ElementId::Name(
                format!("container-batch-{}", action.label_key()).into(),
            ))
            .xsmall()
            .ghost()
            .icon(action.icon())
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_container_batch(tab_id, action, cx);
            })),
        );
    }
    bar = bar.child(div().flex_1()).child(
        Button::new("container-batch-clear")
            .xsmall()
            .ghost()
            .icon(IconName::Close)
            .label(rust_i18n::t!("container.batch.clear").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                    workspace.checked.clear();
                    workspace.batch_armed = false;
                }
                cx.notify();
            })),
    );
    bar.into_any_element()
}
