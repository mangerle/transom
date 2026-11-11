//! 容器控制台页签。
//!
//! 设计原理（Why）：终端会话的启停请求属于工作区实体，启动身份与 Shell 选择
//! 菜单只持有实体句柄，点击后直接 `update` 实体；页签渲染只消费终端视图与
//! 当前选择快照，不再依赖 `Context<AppShell>`。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 终端页渲染参数：已建立的终端视图句柄与当前用户/Shell 选择。
pub(crate) struct ContainerTerminalView<'a> {
    pub(super) terminal: Option<Entity<transom_terminal::TerminalView>>,
    /// 外层 `None` 表示还没有会话占位；内层 `None` 表示镜像默认用户。
    pub(super) user: Option<&'a Option<String>>,
    /// 外层 `None` 表示还没有会话占位；内层 `None` 表示默认 Shell。
    pub(super) shell: Option<&'a Option<String>>,
}

/// 构造一个启动身份/Shell 组合的菜单项。
fn terminal_menu_item(
    view: &Entity<ContainerWorkspace>,
    label_key: &'static str,
    user: Option<&'static str>,
    shell: Option<&'static str>,
) -> PopupMenuItem {
    let view = view.clone();
    crate::widgets::menu_item(
        rust_i18n::t!(label_key).to_string(),
        AppIcon::Domain(DomainIcon::TERMINAL),
        move |_, _, cx| {
            view.update(cx, |workspace, cx| {
                workspace.open_container_terminal_as(
                    user.map(str::to_string),
                    shell.map(str::to_string),
                    cx,
                );
            });
        },
    )
}

/// 填充终端启动身份与 Shell 的选择菜单。
pub(crate) fn populate_terminal_menu(
    menu: PopupMenu,
    view: Entity<ContainerWorkspace>,
) -> PopupMenu {
    menu.item(terminal_menu_item(
        &view,
        "container.terminal.as_root_bash",
        Some("root"),
        Some("/bin/bash"),
    ))
    .item(terminal_menu_item(
        &view,
        "container.terminal.as_root_sh",
        Some("root"),
        Some("/bin/sh"),
    ))
    .item(PopupMenuItem::separator())
    .item(terminal_menu_item(
        &view,
        "container.terminal.as_default_bash",
        None,
        Some("/bin/bash"),
    ))
    .item(terminal_menu_item(
        &view,
        "container.terminal.as_default_sh",
        None,
        Some("/bin/sh"),
    ))
    .item(terminal_menu_item(
        &view,
        "container.terminal.as_default",
        None,
        None,
    ))
}

/// 渲染终端页签：已有会话时显示工具条与终端视图，否则显示启动入口。
pub(crate) fn render_terminal(
    view: &Entity<ContainerWorkspace>,
    params: ContainerTerminalView<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerTerminalView {
        terminal,
        user,
        shell,
    } = params;

    if let Some(terminal) = terminal {
        let default_user = rust_i18n::t!("container.terminal.default_user_name").to_string();
        let user_label = match user {
            Some(Some(user)) => user.as_str(),
            _ => default_user.as_str(),
        };
        let default_shell = rust_i18n::t!("container.terminal.default_shell_name").to_string();
        let shell_label = match shell {
            Some(Some(shell)) => shell.as_str(),
            _ => default_shell.as_str(),
        };
        let session_info = rust_i18n::t!(
            "container.terminal.session_info",
            user = user_label,
            shell = shell_label
        )
        .to_string();

        let menu_view = view.clone();
        let toolbar = h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL)).xsmall())
                    .child(session_info),
            )
            .child(div().flex_1())
            .child(
                Button::new("container-terminal-switch")
                    .ghost()
                    .xsmall()
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                    .label(rust_i18n::t!("container.terminal.switch_user").to_string())
                    .dropdown_menu(move |menu, _, _| {
                        populate_terminal_menu(menu, menu_view.clone())
                    }),
            );

        return v_flex()
            .size_full()
            .overflow_hidden()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(terminal))
            .into_any_element();
    }

    let user_tooltip = match user {
        Some(Some(user)) => {
            rust_i18n::t!("container.terminal.current_user", user = user.as_str()).to_string()
        }
        Some(None) => rust_i18n::t!("container.terminal.current_default_user").to_string(),
        None => rust_i18n::t!("container.terminal.as_user").to_string(),
    };

    let menu_view = view.clone();
    let open_view = view.clone();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.terminal.hint").to_string()),
        )
        .child(
            h_flex()
                .gap_0()
                .items_center()
                .child(
                    Button::new("container-terminal-open")
                        .primary()
                        .small()
                        .child(AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL)).small())
                        .label(rust_i18n::t!("container.terminal.open").to_string())
                        .on_click(move |_, _, cx| {
                            open_view.update(cx, |workspace, cx| {
                                workspace.open_container_terminal_as(None, None, cx);
                            });
                        }),
                )
                .child(
                    Button::new("container-terminal-open-menu")
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small())
                        .accessibility_label(user_tooltip.clone())
                        .tooltip(user_tooltip)
                        .dropdown_menu(move |menu, _, _| {
                            populate_terminal_menu(menu, menu_view.clone())
                        }),
                ),
        )
        .into_any_element()
}
