//! 引擎分组新建/重命名表单。

use super::*;
use crate::widgets::{DialogConfirmKind, dialog_cancel_button, dialog_confirm_button};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 引擎分组的新建/重命名弹窗体。
///
/// 设计原理（Why）：表单状态归侧栏实体，父级路径由目录快照注入；取消与保存
/// 直接 `update` 实体，保存的目录写入由外壳消费事件完成。
pub(crate) fn render_engine_folder_form_body(
    view: &Entity<ContainerSidebarView>,
    state: &ContainerSidebarViewState,
    catalog: &SessionCatalog,
    p: &Palette,
) -> AnyElement {
    let Some(form) = state.engine_folder_form.as_ref() else {
        return div().into_any_element();
    };
    let editing = form.editing.is_some();
    let parent = form.parent;
    let input = form.input.clone();
    let error = form.error.clone();
    let title = if editing {
        rust_i18n::t!("container.sidebar.folder_edit_title").to_string()
    } else {
        rust_i18n::t!("container.sidebar.folder_new_title").to_string()
    };
    let description = parent
        .and_then(|id| catalog.engine_folder_path(id))
        .map(|path| rust_i18n::t!("container.sidebar.folder_parent", path = path).to_string());
    let mut content = v_flex().gap_3().child(crate::widgets::form_field(
        rust_i18n::t!("container.sidebar.folder_name").to_string(),
        input,
        p,
    ));
    if let Some(error) = error {
        content = content.child(div().text_xs().text_color(p.danger).child(error));
    }
    let view_cancel = view.clone();
    let view_save = view.clone();
    let footer = h_flex()
        .gap_2()
        .justify_end()
        .child(dialog_cancel_button(
            "container-engine-folder-cancel",
            rust_i18n::t!("common.cancel").to_string(),
            move |_, _, cx| {
                view_cancel.update(cx, |sidebar, cx| sidebar.dismiss_folder_form(cx));
            },
        ))
        .child(dialog_confirm_button(
            "container-engine-folder-save",
            rust_i18n::t!("common.save").to_string(),
            Some(AppIcon::Ui(UiIcon::CHECK)),
            DialogConfirmKind::Primary,
            move |_, _, cx| {
                view_save.update(cx, |sidebar, cx| sidebar.submit_folder_form(cx));
            },
        ));
    crate::widgets::dialog_body(
        "container-engine-folder-body",
        title,
        description,
        content,
        Some(footer),
    )
}
