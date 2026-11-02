//! 危险操作确认条。

use super::*;

pub(crate) fn delete_button(
    id: &'static str,
    cx: &mut Context<AppShell>,
    tab_id: TabId,
    confirm: ContainerConfirm,
) -> AnyElement {
    Button::new(id)
        .danger()
        .xsmall()
        .icon(IconName::Trash)
        .label(rust_i18n::t!("common.delete").to_string())
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                workspace.confirm = Some(confirm.clone());
            }
            cx.notify();
        }))
        .into_any_element()
}

pub(crate) fn render_confirm_bar(
    tab_id: TabId,
    confirm: ContainerConfirm,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (label, recursive) = match &confirm {
        ContainerConfirm::Container(name) => (
            rust_i18n::t!("container.confirm.remove_container", name = name).to_string(),
            false,
        ),
        ContainerConfirm::Image(image) => (
            rust_i18n::t!("container.confirm.remove_image", name = short(image)).to_string(),
            false,
        ),
        ContainerConfirm::Volume(name) => (
            rust_i18n::t!("container.confirm.remove_volume", name = name).to_string(),
            false,
        ),
        ContainerConfirm::Network(name) => (
            rust_i18n::t!("container.confirm.remove_network", name = name).to_string(),
            false,
        ),
        ContainerConfirm::Compose(name) => (
            rust_i18n::t!("container.confirm.compose_down", name = name).to_string(),
            false,
        ),
        ContainerConfirm::FilePath { path, recursive } => (
            rust_i18n::t!("container.confirm.remove_path", path = path).to_string(),
            *recursive,
        ),
    };
    h_flex()
        .mx_3()
        .my_2()
        .p_2()
        .gap_3()
        .items_center()
        .justify_between()
        .rounded(p.radius.md)
        .bg(p.danger.opacity(0.12))
        .border_1()
        .border_color(p.danger.opacity(0.4))
        .child(div().text_sm().text_color(p.danger).child(label))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("container-confirm-cancel")
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_container_confirm(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("container-confirm-delete")
                        .danger()
                        .xsmall()
                        .icon(if recursive {
                            IconName::Trash
                        } else {
                            IconName::Check
                        })
                        .label(rust_i18n::t!("common.confirm_delete").to_string())
                        .on_click(cx.listener({
                            let confirm = confirm.clone();
                            move |this, _, _, cx| {
                                this.apply_container_confirm(tab_id, confirm.clone(), cx);
                            }
                        })),
                ),
        )
        .into_any_element()
}
