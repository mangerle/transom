//! 无活动 API 项目时的主区域启动页。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::action_button;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

impl AppShell {
    pub(crate) fn render_api_project_launcher(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let sidebar = self.state.features.api_sidebar.clone();
        let projects = sidebar.read(cx).state.registry.projects.clone();
        let management = {
            let state = sidebar.read(cx);
            let mut content = v_flex().w(rems(30.0)).gap(rems(space::XS));
            let mut has_content = false;
            if let Some(editor) = &state.state.editor {
                content = content.child(super::sidebar_view::render_editor(&sidebar, editor, p));
                has_content = true;
            }
            if let Some(import) = &state.state.pending_import {
                content = content.child(super::sidebar_view::render_import_state(
                    &sidebar, import, p,
                ));
                has_content = true;
            }
            if let Some(error) = &state.state.load_error {
                content = content.child(
                    div()
                        .p(rems(space::SM))
                        .rounded(p.radius.md)
                        .bg(p.surface.subtle)
                        .text_color(p.danger)
                        .child(error.clone()),
                );
                has_content = true;
            }
            has_content.then(|| content.into_any_element())
        };
        let create_view = sidebar.clone();
        let import_view = sidebar.clone();
        let projects_grid =
            h_flex()
                .w_full()
                .gap(rems(space::SM))
                .flex_wrap()
                .children(projects.into_iter().map(|project| {
                    let project_id = project.id;
                    let title = project.display_name().to_string();
                    let count =
                        rust_i18n::t!("api.project.request_count", count = project.request_count())
                            .to_string();
                    let open_view = sidebar.clone();
                    v_flex()
                        .id(ElementId::Name(
                            format!("api-launcher-project-{project_id}").into(),
                        ))
                        .w(rems(15.0))
                        .p(rems(space::SM))
                        .gap(rems(space::XS))
                        .rounded(p.radius.lg)
                        .border_1()
                        .border_color(p.divider(p.border))
                        .bg(p.surface.subtle)
                        .hover(|style| style.bg(p.list_hover))
                        .on_click(move |_, _, cx| {
                            open_view.update(cx, |view, cx| view.open_project(project_id, cx));
                        })
                        .child(
                            h_flex()
                                .gap(rems(space::XS))
                                .items_center()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::WEB_SERVER)).small())
                                .child(div().flex_1().min_w_0().truncate().child(title)),
                        )
                        .child(div().text_color(p.muted_foreground).child(count))
                        .into_any_element()
                }));

        v_flex()
            .id("api-project-launcher")
            .size_full()
            .p(rems(space::XL))
            .gap(rems(space::LG))
            .items_center()
            .justify_center()
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::WEB_SERVER)).with_rems(rems(2.0)))
            .child(
                div()
                    .text_size(rems(crate::theme::tokens::typography::TITLE))
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("api.sidebar.new_project").to_string()),
            )
            .child(
                div()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("api.sidebar.empty_hint").to_string()),
            )
            .child(
                h_flex()
                    .gap(rems(space::SM))
                    .child(
                        action_button(
                            "api-launcher-create",
                            AppIcon::Ui(UiIcon::ADD),
                            rust_i18n::t!("api.sidebar.new_project").to_string(),
                        )
                        .primary()
                        .on_click(move |_, window, cx| {
                            create_view.update(cx, |view, cx| view.start_create(window, cx));
                        }),
                    )
                    .child(
                        action_button(
                            "api-launcher-import",
                            AppIcon::Ui(UiIcon::DOWNLOAD),
                            rust_i18n::t!("api.sidebar.import_project").to_string(),
                        )
                        .on_click(move |_, _, cx| {
                            import_view.update(cx, |_, cx| {
                                cx.emit(super::ApiSidebarEvent::ImportProject);
                                cx.notify();
                            });
                        }),
                    ),
            )
            .children(management)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .child(projects_grid),
            )
            .into_any_element()
    }
}
