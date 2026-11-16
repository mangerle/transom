//! 日志页签右侧工具条：显示开关、滚动控制与复制/导出/清空。

use super::*;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Selectable, Side};
use transom_ui_icons::UiIcon;

/// 右侧竖排工具条：按“显示 / 滚动 / 数据操作”三组排列。
pub(super) fn render_logs_sidebar(
    view: &Entity<ContainerWorkspace>,
    params: &ContainerLogsView<'_>,
    p: &Palette,
) -> AnyElement {
    let children: Vec<AnyElement> = vec![
        log_timestamps_button(view, params.timestamps),
        log_reverse_button(view, params.reversed),
        log_tail_button(view, params.tail),
        log_divider(p),
        log_scroll_top_button(params.scroll),
        log_scroll_bottom_button(params.scroll),
        log_soft_wrap_button(view, params.soft_wrap),
        log_follow_button(view, params.follow),
        log_divider(p),
        log_copy_button(view),
        log_export_button(view),
        log_clear_button(view),
    ];
    v_flex()
        .w_8()
        .h_full()
        .flex_shrink_0()
        .py_2()
        .gap_1()
        .items_center()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Left, p.divider_width)
        .children(children)
        .into_any_element()
}

fn log_divider(p: &Palette) -> AnyElement {
    div()
        .w_4()
        .my_1()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Top, p.divider_width)
        .into_any_element()
}

fn log_timestamps_button(view: &Entity<ContainerWorkspace>, timestamps: bool) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-timestamps",
        UiIcon::TOGGLE_VISIBILITY,
        rust_i18n::t!("container.logs.timestamps").to_string(),
    )
    .render()
    .selected(timestamps)
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.toggle_container_logs_timestamps();
            cx.notify();
        });
    })
    .into_any_element()
}

fn log_reverse_button(view: &Entity<ContainerWorkspace>, reversed: bool) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-reverse",
        UiIcon::SORT_BY,
        rust_i18n::t!("container.logs.reverse").to_string(),
    )
    .render()
    .selected(reversed)
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.state.logs_reversed = !workspace.state.logs_reversed;
            cx.notify();
        });
    })
    .into_any_element()
}

fn log_tail_button(view: &Entity<ContainerWorkspace>, tail: usize) -> AnyElement {
    let tail_view = view.clone();
    IconButton::new(
        "container-logs-tail-dropdown",
        UiIcon::LIST_FILES,
        rust_i18n::t!("container.logs.tail_lines").to_string(),
    )
    .render()
    .dropdown_menu(move |menu, _, _| {
        let mut menu = menu.check_side(Side::Right);
        for choice in LOG_TAIL_CHOICES {
            let label = if choice == 0 {
                rust_i18n::t!("container.logs.tail_all").to_string()
            } else {
                rust_i18n::t!("container.logs.tail_format", count = choice).to_string()
            };
            menu = menu.item(PopupMenuItem::new(label).checked(choice == tail).on_click({
                let view = tail_view.clone();
                move |_, _, cx| {
                    view.update(cx, |workspace, cx| {
                        workspace.set_container_logs_tail(choice);
                        cx.notify();
                    });
                }
            }));
        }
        menu
    })
    .into_any_element()
}

fn log_scroll_top_button(scroll: &ScrollHandle) -> AnyElement {
    let scroll = scroll.clone();
    IconButton::new(
        "container-logs-scroll-top",
        UiIcon::UP,
        rust_i18n::t!("container.logs.scroll_top").to_string(),
    )
    .render()
    .on_click(move |_, _, _| {
        scroll.set_offset(point(px(0.0), px(0.0)));
    })
    .into_any_element()
}

fn log_scroll_bottom_button(scroll: &ScrollHandle) -> AnyElement {
    let scroll = scroll.clone();
    IconButton::new(
        "container-logs-scroll-bottom",
        UiIcon::DOWN,
        rust_i18n::t!("container.logs.scroll_bottom").to_string(),
    )
    .render()
    .on_click(move |_, _, _| {
        scroll.scroll_to_bottom();
    })
    .into_any_element()
}

fn log_soft_wrap_button(view: &Entity<ContainerWorkspace>, soft_wrap: bool) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-soft-wrap",
        UiIcon::SOFT_WRAP,
        rust_i18n::t!("container.logs.soft_wrap").to_string(),
    )
    .render()
    .selected(soft_wrap)
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.state.logs_soft_wrap = !workspace.state.logs_soft_wrap;
            cx.notify();
        });
    })
    .into_any_element()
}

fn log_follow_button(view: &Entity<ContainerWorkspace>, follow: bool) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-follow",
        UiIcon::SCROLL_DOWN,
        rust_i18n::t!("container.logs.follow").to_string(),
    )
    .render()
    .selected(follow)
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.toggle_container_logs_follow(!follow);
            cx.notify();
        });
    })
    .into_any_element()
}

fn log_copy_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-copy",
        UiIcon::COPY,
        rust_i18n::t!("container.logs.copy").to_string(),
    )
    .render()
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.copy_container_logs(cx);
        });
    })
    .into_any_element()
}

fn log_export_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-export",
        UiIcon::DOWNLOAD,
        rust_i18n::t!("container.logs.export").to_string(),
    )
    .render()
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.export_container_logs(cx);
        });
    })
    .into_any_element()
}

fn log_clear_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let view = view.clone();
    IconButton::new(
        "container-logs-clear",
        UiIcon::DELETE,
        rust_i18n::t!("container.logs.clear").to_string(),
    )
    .render()
    .on_click(move |_, _, cx| {
        view.update(cx, |workspace, cx| {
            workspace.state.logs.clear();
            cx.notify();
        });
    })
    .into_any_element()
}
