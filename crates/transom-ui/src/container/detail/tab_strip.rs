//! 详情页签按钮组：普通页签与带用户下拉的终端页签。
//!
//! 设计原理（Why）：页签按钮只决定当前详情页签；从 detail.rs 拆出后，该文件
//! 只负责详情内容分发，页签选择与终端用户下拉在这里单独维护。渲染只持有工作区
//! 实体句柄与状态快照，页签切换直接调用实体方法，不再依赖外壳上下文。

use super::*;
use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::theme::tokens::control;
use crate::widgets::action_button_with_icon_tint;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 详情页签按钮组：不含外边距与分隔线，由详情头部把它放在同一行的左端。
///
/// 只有一个页签的分类（镜像/卷/网络/编排）没有可切换的东西，返回 `None`，
/// 头部因此保留标题。
pub(super) fn render_detail_tab_buttons(
    view: &Entity<ContainerWorkspace>,
    workspace: &ContainerWorkspaceState,
    section: ContainerSection,
    current: ContainerDetailTab,
    p: &Palette,
) -> Option<AnyElement> {
    let tabs = section.detail_tabs();
    if tabs.len() <= 1 {
        return None;
    }
    // 当前终端会话（含正在建立）的执行用户；`None` 表示还没有会话。
    let current_user = workspace
        .terminal
        .as_ref()
        .map(|terminal| terminal.user.clone())
        .or_else(|| {
            workspace
                .terminal_request
                .as_ref()
                .map(|(_, user, _)| user.clone())
        });
    let user_tooltip = match &current_user {
        Some(Some(user)) => {
            rust_i18n::t!("container.terminal.current_user", user = user.as_str()).to_string()
        }
        Some(None) => rust_i18n::t!("container.terminal.current_default_user").to_string(),
        None => rust_i18n::t!("container.terminal.as_user").to_string(),
    };
    Some(
        h_flex()
            .gap_1()
            .items_center()
            .flex_none()
            .min_w_0()
            .children(tabs.iter().copied().map(|tab| {
                if tab != ContainerDetailTab::Terminal {
                    let tab_view = view.clone();
                    let selected = tab == current;
                    return action_button_with_icon_tint(
                        ElementId::Name(format!("container-detail-tab-{}", tab.label_key()).into()),
                        detail_icon(tab),
                        rust_i18n::t!(tab.label_key()).to_string(),
                        selected.then_some(p.list_active_border),
                    )
                    .selected(selected)
                    .on_click(move |_, _, cx| {
                        tab_view.update(cx, |workspace, cx| {
                            workspace.set_detail_tab(tab, cx);
                        });
                    })
                    .into_any_element();
                }
                // 终端页签带一个下拉：root 与镜像默认用户的会话是两回事，
                // 点主按钮仍走默认用户，避免改变原有肌肉记忆。主按钮与
                // 三角共用一个外框，视觉上是一枚分体按钮而不是两枚。
                let tab_view = view.clone();
                let menu_view = view.clone();
                let selected = tab == current;
                h_flex()
                    .items_center()
                    .flex_none()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(p.border)
                    .overflow_hidden()
                    .child(
                        action_button_with_icon_tint(
                            "container-detail-tab-terminal",
                            DomainIcon::TERMINAL,
                            rust_i18n::t!("container.detail.terminal").to_string(),
                            selected.then_some(p.list_active_border),
                        )
                        .ghost()
                        .selected(selected)
                        .tooltip(user_tooltip.clone())
                        .on_click(move |_, _, cx| {
                            tab_view.update(cx, |workspace, cx| {
                                workspace.set_detail_tab(tab, cx);
                            });
                        }),
                    )
                    .child(div().w(px(1.0)).h(px(14.0)).bg(p.border))
                    .child(
                        Button::new("container-detail-tab-terminal-menu")
                            .small()
                            .ghost()
                            .child(
                                AppIconElement::new(UiIcon::CHEVRON_DOWN)
                                    .with_rems(rems(control::COMPACT_ICON)),
                            )
                            .accessibility_label(user_tooltip.clone())
                            .tooltip(user_tooltip.clone())
                            .dropdown_menu(move |menu, _, _| {
                                terminal::populate_terminal_menu(menu, menu_view.clone())
                            }),
                    )
                    .into_any_element()
            }))
            .into_any_element(),
    )
}
