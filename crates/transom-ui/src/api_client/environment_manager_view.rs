//! 环境切换、命名和增删管理视图。

use super::ApiWorkspace;
use super::workspace_view::ApiWorkspaceParams;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_api::ApiEnvironmentId;

pub(super) fn render_environment_manager(params: &ApiWorkspaceParams<'_>) -> AnyElement {
    let view = params.view.clone();
    let add_view = params.view.clone();
    let delete_view = params.view.clone();
    let environment = dropdown(
        "api-environment-manager-selector",
        params.environment_id,
        params
            .project
            .environments
            .iter()
            .map(|environment| (Some(environment.id), environment.name.clone()))
            .collect(),
        |this: &mut ApiWorkspace, environment_id: Option<ApiEnvironmentId>, window, cx| {
            if let Some(environment_id) = environment_id {
                this.set_environment(environment_id, window, cx);
            }
        },
        view,
    );
    v_flex()
        .size_full()
        .min_h_0()
        .gap(rems(space::XS))
        .child(
            h_flex()
                .gap(rems(space::XS))
                .items_center()
                .child(environment)
                .child(Input::new(params.environment_name).flex_1().min_w_0())
                .child(
                    IconButton::new(
                        "api-environment-add",
                        transom_ui_icons::AppIcon::Ui(transom_ui_icons::UiIcon::ADD),
                        rust_i18n::t!("api.request.add_environment").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, window, cx| {
                        add_view.update(cx, |this, cx| this.create_environment(window, cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-environment-delete",
                        transom_ui_icons::AppIcon::Ui(transom_ui_icons::UiIcon::DELETE),
                        rust_i18n::t!("api.request.delete_environment").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .disabled(params.project.environments.len() <= 1)
                    .render()
                    .on_click(move |_, window, cx| {
                        delete_view
                            .update(cx, |this, cx| this.delete_current_environment(window, cx));
                    }),
                ),
        )
        .child(params.environment_editor.clone())
        .into_any_element()
}
