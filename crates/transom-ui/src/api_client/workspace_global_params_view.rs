//! 项目级全局 Query 参数与 Header 参数编辑视图。

use super::ApiGlobalParameterTab;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::*;

const TABS: [ApiGlobalParameterTab; 2] =
    [ApiGlobalParameterTab::Query, ApiGlobalParameterTab::Header];

pub(super) fn render_global_parameters(
    params: &ApiWorkspaceParams<'_>,
    p: &Palette,
) -> AnyElement {
    let items = TABS
        .into_iter()
        .map(|tab| {
            let view = params.view.clone();
            let (id, label) = match tab {
                ApiGlobalParameterTab::Query => (
                    "api-global-param-query",
                    rust_i18n::t!("api.request.global_query_parameters").to_string(),
                ),
                ApiGlobalParameterTab::Header => (
                    "api-global-param-header",
                    rust_i18n::t!("api.request.global_header_parameters").to_string(),
                ),
            };
            super::workspace_tabs_bar::UnderlineTabItem {
                id,
                label,
                selected: tab == params.global_parameter_tab,
                on_select: Box::new(move |_, cx| {
                    view.update(cx, |this, cx| this.set_global_parameter_tab(tab, cx));
                }),
            }
        })
        .collect::<Vec<_>>();
    let tabs = super::workspace_tabs_bar::render_underline_tabs(
        "api-global-parameter-tabs",
        items,
        None,
        p,
    );
    let editor = match params.global_parameter_tab {
        ApiGlobalParameterTab::Query => params.global_query_editor.clone().into_any_element(),
        ApiGlobalParameterTab::Header => params.global_headers_editor.clone().into_any_element(),
    };
    v_flex()
        .size_full()
        .min_h_0()
        .child(tabs)
        .child(editor)
        .into_any_element()
}
