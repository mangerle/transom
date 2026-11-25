//! API 项目窗格渲染。

use super::*;

impl AppShell {
    pub(super) fn render_api_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Option<AnyElement> {
        let workspace = self.state.workspace.tabs[index].api_workspace()?;
        let api_sidebar = self.state.features.api_sidebar.clone();
        let api_sidebar_state = api_sidebar.read(cx);
        let state = workspace.read(cx);
        let params = state.render_params(
            &workspace,
            &api_sidebar,
            &api_sidebar_state.state,
            window.rem_size(),
        );
        Some(crate::api_client::render_api_workspace(params, p))
    }
}
