//! API 客户端弹窗渲染分发：从当前激活的 API 工作区标签渲染对应弹窗主体。

use super::*;

impl ModalKind {
    /// 若为 API 客户端弹窗，则渲染其内容主体；否则返回 `None`。
    pub(super) fn render_api_dialog_body(
        self,
        app_state: &AppState,
        window: &Window,
        cx: &App,
        p: &Palette,
    ) -> Option<AnyElement> {
        if !matches!(self, Self::ApiNewRequest | Self::ApiConfig) {
            return None;
        }
        let Some(workspace) = app_state
            .workspace
            .active_tab
            .and_then(|tab_id| app_state.workspace.tab(tab_id))
            .and_then(|tab| tab.api_workspace())
        else {
            return Some(div().into_any_element());
        };
        let element = match self {
            Self::ApiNewRequest => crate::api_client::render_api_new_request_dialog_body(
                &workspace,
                &app_state.features.api_sidebar,
                window,
                cx,
                p,
            ),
            Self::ApiConfig => crate::api_client::render_api_config_dialog_body(
                &workspace,
                &app_state.features.api_sidebar,
                window,
                cx,
                p,
            ),
            _ => div().into_any_element(),
        };
        Some(element)
    }
}
