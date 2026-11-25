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
        Some(crate::api_client::render_api_workspace(
            crate::api_client::ApiWorkspaceParams {
                view: &workspace,
                api_sidebar: &api_sidebar,
                api_sidebar_state: &api_sidebar_state.state,
                project: &state.project,
                request: state.selected_request(),
                selected_request: state.selected_request,
                method: state.method,
                protocol: state.protocol,
                environment_id: state.environment_id,
                request_tab: state.request_tab,
                response_tab: state.response_tab,
                selected_history: state.selected_history,
                save_destination_open: state.save_destination_open,
                auxiliary_section: state.auxiliary_section,
                config_modal: state.config_modal,
                new_request_modal_open: state.new_request_modal_open,
                global_parameter_tab: state.global_parameter_tab,
                folder_editor: state.folder_editor.as_ref(),
                pending_folder_delete: state.pending_folder_delete,
                codegen_target: state.codegen_target,
                ws_connected: state.ws_connected,
                notice: state.notice.as_deref(),
                response: state.response.as_ref(),
                assertion_results: &state.assertion_results,
                sending: state.sending,
                error: state.error.as_deref(),
                request_name: &state.editors.request_name,
                request_url: &state.editors.request_url,
                query_editor: &state.editors.query,
                header_editor: &state.editors.headers,
                request_body: &state.editors.request_body,
                form_editor: &state.editors.form,
                grpc_service: &state.editors.grpc_service,
                grpc_method: &state.editors.grpc_method,
                graphql_variables: &state.editors.graphql_variables,
                auth: &state.editors.auth,
                assertions: &state.editors.assertions,
                timeout: &state.editors.timeout,
                docs: &state.editors.docs,
                environment_editor: &state.editors.environment,
                global_variables_editor: &state.editors.global_variables,
                global_query_editor: &state.editors.global_query,
                global_headers_editor: &state.editors.global_headers,
                environment_name: &state.editors.environment_name,
                cookies_editor: &state.editors.cookies,
                websocket_input: &state.editors.websocket_input,
                websocket_log: &state.editors.websocket_log,
                response_body: &state.editors.response_body,
                code_preview: &state.editors.code_preview,
                save_destination_search: &state.editors.save_destination_search,
                tree_editor_split: &state.tree_editor_split,
                request_response_split: &state.request_response_split,
                rem: window.rem_size(),
            },
            p,
        ))
    }
}
