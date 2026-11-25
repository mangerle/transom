//! 项目级全局 Query 参数与 Header 参数编辑视图。

use super::ApiGlobalParameterTab;
use super::workspace_view::ApiWorkspaceParams;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::*;

const TABS: [ApiGlobalParameterTab; 2] =
    [ApiGlobalParameterTab::Query, ApiGlobalParameterTab::Header];

pub(super) fn render_global_parameters(params: &ApiWorkspaceParams<'_>) -> AnyElement {
    let view = params.view.clone();
    let selected = TABS
        .iter()
        .position(|tab| *tab == params.global_parameter_tab)
        .unwrap_or(0);
    let tabs = TabBar::new("api-global-parameter-tabs")
        .underline()
        .small()
        .w_full()
        .selected_index(selected)
        .children(TABS.into_iter().map(|tab| {
            let label = match tab {
                ApiGlobalParameterTab::Query => {
                    rust_i18n::t!("api.request.global_query_parameters").to_string()
                }
                ApiGlobalParameterTab::Header => {
                    rust_i18n::t!("api.request.global_header_parameters").to_string()
                }
            };
            Tab::new().label(label)
        }))
        .on_click(move |index, _, cx| {
            if let Some(tab) = TABS.get(*index).copied() {
                view.update(cx, |this, cx| this.set_global_parameter_tab(tab, cx));
            }
        });
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
