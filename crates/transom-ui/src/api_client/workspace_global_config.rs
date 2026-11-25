//! 环境管理、全局变量与全局参数编辑器载入。

use super::{ApiGlobalParameterTab, ApiWorkspace};
use gpui_kit::*;
use transom_api::{ApiEnvironment, ApiGlobalParameterTarget};

impl ApiWorkspace {
    pub(super) fn load_global_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editors.global_variables.update(cx, |editor, cx| {
            editor.set_variables(&self.project.global_variables, window, cx)
        });
        let query = self
            .project
            .global_parameters
            .iter()
            .filter(|parameter| parameter.target == ApiGlobalParameterTarget::Query)
            .map(global_parameter_entry)
            .collect::<Vec<_>>();
        let headers = self
            .project
            .global_parameters
            .iter()
            .filter(|parameter| parameter.target == ApiGlobalParameterTarget::Header)
            .map(global_parameter_entry)
            .collect::<Vec<_>>();
        self.editors
            .global_query
            .update(cx, |editor, cx| editor.set_entries(&query, window, cx));
        self.editors
            .global_headers
            .update(cx, |editor, cx| editor.set_entries(&headers, window, cx));
    }

    pub(crate) fn set_global_parameter_tab(
        &mut self,
        tab: ApiGlobalParameterTab,
        cx: &mut Context<Self>,
    ) {
        self.global_parameter_tab = tab;
        cx.notify();
    }

    /// 新建环境并立即切换，名称默认“新环境 N”。
    pub(crate) fn create_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = rust_i18n::t!(
            "api.request.environment_default_name",
            count = self.project.environments.len().saturating_add(1)
        )
        .to_string();
        let environment = ApiEnvironment::new(name);
        let id = environment.id;
        self.project.environments.push(environment);
        self.project.active_environment_id = Some(id);
        self.project.touch();
        self.load_environment(window, cx);
        cx.notify();
    }

    /// 删除当前环境；最后一个环境不允许删除。
    pub(crate) fn delete_current_environment(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.environments.len() <= 1 {
            return;
        }
        let Some(current) = self.environment_id.or(self.project.active_environment_id) else {
            return;
        };
        self.project
            .environments
            .retain(|environment| environment.id != current);
        let next = self
            .project
            .environments
            .first()
            .map(|environment| environment.id);
        self.project.active_environment_id = next;
        self.environment_id = next;
        self.project.touch();
        self.load_environment(window, cx);
        cx.notify();
    }
}

fn global_parameter_entry(
    parameter: &transom_api::ApiGlobalParameter,
) -> super::kv_editor::KvEntry {
    super::kv_editor::KvEntry {
        name: parameter.name.clone(),
        value: parameter.value.clone(),
        description: parameter.description.clone(),
        enabled: parameter.enabled,
    }
}
