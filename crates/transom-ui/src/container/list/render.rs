//! 列表行渲染与底部状态条。
//!
//! 行渲染只持有工作区实体句柄：点击直接 `select_row`，快捷按钮与右键菜单把
//! 行动作交给 [`ContainerWorkspace::run_row_action`] 上报外壳统一分发，不再经
//! `AppShell` 的渲染方法。

use super::*;
use crate::app::AppShell;
use crate::widgets::status_dot_bare;
use gpui_kit::base::{ScrollBounce, v_virtual_list};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::scroll::Scrollbar;
use std::rc::Rc;
use transom_ui_icons::AppIconElement;

/// 容器列表虚拟渲染参数。
pub(crate) struct ContainerRowsParams<'a> {
    pub(crate) app_entity: Entity<AppShell>,
    pub(crate) view: &'a Entity<ContainerWorkspace>,
    pub(crate) rows: Vec<ListRow>,
    pub(crate) selected: Option<String>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) scroll: VirtualListScrollHandle,
    pub(crate) rem: Pixels,
    pub(crate) palette: &'a Palette,
}

/// 渲染列表；空结果给出可行动的提示而不是白屏。
pub(crate) fn render_rows(params: ContainerRowsParams<'_>) -> AnyElement {
    let ContainerRowsParams {
        app_entity,
        view,
        rows,
        selected,
        focus_handle,
        scroll,
        rem,
        palette: p,
    } = params;
    if rows.is_empty() {
        return v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .p_4()
            .child(crate::widgets::empty_state(
                AppIcon::Ui(UiIcon::SEARCH),
                rust_i18n::t!("container.list.none").to_string(),
                rust_i18n::t!("container.list.none_hint").to_string(),
                p,
            ))
            .into_any_element();
    }
    let row_count = rows.len();
    let rows = Rc::new(rows);
    let row_height = rems(2.5).to_pixels(rem);
    let item_sizes = Rc::new(vec![size(px(0.0), row_height); row_count]);
    let p = p.clone();
    let view_for_rows = view.clone();
    let list = v_virtual_list(
        app_entity,
        ElementId::Name("container-list-virtual-list".into()),
        item_sizes,
        move |_app, range, _window, _cx| {
            range
                .filter_map(|index| {
                    let row = rows.get(index)?;
                    let is_selected = selected.as_deref() == Some(row_key(&row.target));
                    Some(list_row(
                        &view_for_rows,
                        row,
                        is_selected,
                        &focus_handle,
                        &p,
                    ))
                })
                .collect()
        },
    )
    .track_scroll(&scroll)
    .w_full()
    .h_full();
    div()
        .id("container-list-scroll")
        .relative()
        .flex_1()
        .min_h_0()
        .child(ScrollBounce::new("container-list-scroll-bounce", &scroll, list).enabled(false))
        .child(Scrollbar::vertical(&scroll).viewport_from_layout())
        .into_any_element()
}

/// 单行渲染：状态点、标题副标题、快捷按钮与右键菜单。
pub(crate) fn list_row(
    view: &Entity<ContainerWorkspace>,
    row: &ListRow,
    selected: bool,
    focus_handle: &FocusHandle,
    p: &Palette,
) -> AnyElement {
    let group = format!("container-list-row-{}", row.key);
    let title = row.title.clone();
    let mut element = h_flex()
        .id(ElementId::Name(row.key.clone().into()))
        .group(group.as_str())
        .w_full()
        .h(rems(2.5))
        .relative()
        .gap_2()
        .px_2p5()
        .rounded(p.radius.sm)
        .items_center()
        .cursor_pointer();

    if selected {
        element = element.bg(p.list_active).text_color(p.foreground);
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
        .text_size(rems(0.8125))
        .font_weight(FontWeight::MEDIUM)
        .truncate()
        .child(title);

    let names = v_flex().flex_1().min_w_0().gap_0p5().child(title_el);

    element = element.child(match &row.subtitle {
        Some(subtitle) => names
            .child(
                div()
                    .text_size(rems(0.6875))
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
                .text_size(rems(0.6875))
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
            let view = view.clone();
            actions_flex = actions_flex.child(
                Button::new(ElementId::Name(
                    format!("{}-action-{:?}", row.key, action).into(),
                ))
                .ghost()
                .xsmall()
                .child(AppIconElement::new(action.icon()).small())
                .accessibility_label(rust_i18n::t!(action.label_key()).to_string())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    view.update(cx, |workspace, cx| {
                        workspace.run_row_action(target.clone(), action, cx);
                    });
                }),
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
    let menu_view = view.clone();
    let click_target = row.target.clone();
    let click_view = view.clone();
    let click_focus = focus_handle.clone();
    let element = element.on_click(move |_, window, cx| {
        click_view.update(cx, |workspace, cx| workspace.select_row(&click_target, cx));
        window.focus(&click_focus, cx);
    });

    element
        .context_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            for action in menu_actions.clone() {
                if action.is_danger() {
                    menu = menu.item(PopupMenuItem::separator());
                }
                let target = menu_target.clone();
                let view = menu_view.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!(action.label_key()).to_string(),
                    action.icon(),
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.run_row_action(target.clone(), action, cx);
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
