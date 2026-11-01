//! 跳板机与代理链表单。

use super::*;

/// 渲染跳板机级联下拉选择与拓扑链展示。
pub(super) fn render_jump_section(
    form: &SessionForm,
    app: &Entity<AppShell>,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let jump_label = form
        .jump_choice
        .and_then(|id| {
            form.jump_options
                .iter()
                .find(|option| option.id == id)
                .map(|option| option.name.clone())
        })
        .unwrap_or_else(|| rust_i18n::t!("form.proxy_jump_none").to_string());
    let target_host = form.host.read(cx).value().trim().to_string();
    let target_port = form.port.read(cx).value().trim().to_string();
    let target_label = if target_port.is_empty() || target_port == "22" {
        target_host
    } else {
        format!("{target_host}:{target_port}")
    };
    let topology = form.proxy_topology(&target_label);
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.proxy_jump").to_string()),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Button::new("proxy-jump-dropdown")
                        .outline()
                        .small()
                        .icon(IconName::Route)
                        .label(jump_label)
                        .dropdown_menu({
                            let options = form.jump_options.clone();
                            // 下拉的渲染闭包要求 'static：句柄要在构造时就克隆好，
                            // 不能等到闭包体内再借用方法参数。
                            let dropdown_app = app.clone();
                            move |menu, _window, _cx| {
                                let app = dropdown_app.clone();
                                let mut menu = menu.item(crate::widgets::menu_item(
                                    rust_i18n::t!("form.proxy_jump_none").to_string(),
                                    IconName::Ban,
                                    {
                                        let app = app.clone();
                                        move |_, _, cx| {
                                            app.update(cx, |this, cx| {
                                                if let Some(form) =
                                                    this.state.modals.session.form.as_mut()
                                                {
                                                    form.jump_choice = None;
                                                }
                                                cx.notify();
                                            });
                                        }
                                    },
                                ));
                                if !options.is_empty() {
                                    menu = menu.item(PopupMenuItem::separator());
                                }
                                for option in &options {
                                    let id = option.id;
                                    let label = format!(
                                        "{} ({}:{})",
                                        option.name, option.host, option.port
                                    );
                                    let app = app.clone();
                                    menu = menu.item(crate::widgets::menu_item(
                                        label,
                                        IconName::Route,
                                        move |_, _, cx| {
                                            app.update(cx, |this, cx| {
                                                if let Some(form) =
                                                    this.state.modals.session.form.as_mut()
                                                {
                                                    form.jump_choice = Some(id);
                                                }
                                                cx.notify();
                                            });
                                        },
                                    ));
                                }
                                menu
                            }
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(topology),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("form.proxy_jump_hint").to_string()),
        )
        .into_any_element()
}
