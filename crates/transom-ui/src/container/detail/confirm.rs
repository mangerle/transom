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
