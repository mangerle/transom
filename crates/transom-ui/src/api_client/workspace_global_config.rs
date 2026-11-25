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

    /// 打开环境变量与全局配置弹窗。
    pub(crate) fn open_config_modal(&mut self, cx: &mut Context<Self>) {
        let initial = self
            .environment_id
            .or(self.project.active_environment_id)
            .or_else(|| self.project.environments.first().map(|env| env.id))
            .map_or(
                super::ApiConfigModalSection::GlobalParameters,
                super::ApiConfigModalSection::Environment,
            );
        self.config_modal = Some(initial);
        cx.notify();
    }

    /// 切换配置弹窗左侧选中的导航项；选中环境时同步切换工作区当前环境。
    pub(crate) fn select_config_modal_section(
        &mut self,
        section: super::ApiConfigModalSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let super::ApiConfigModalSection::Environment(env_id) = section {
            self.set_environment(env_id, window, cx);
        }
        self.config_modal = Some(section);
        cx.notify();
    }

    /// 在配置弹窗中新建环境并自动选中。
    pub(crate) fn create_modal_environment(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_environment(window, cx);
        if let Some(id) = self.project.active_environment_id {
            self.environment_id = Some(id);
            self.config_modal = Some(super::ApiConfigModalSection::Environment(id));
        }
        cx.notify();
    }

    /// 在配置弹窗中删除指定环境；最后一个环境不允许删除。
    pub(crate) fn delete_modal_environment(
        &mut self,
        env_id: transom_api::ApiEnvironmentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project.environments.len() <= 1 {
            return;
        }
        self.project
            .environments
            .retain(|environment| environment.id != env_id);
        let next = self
            .project
            .environments
            .first()
            .map(|environment| environment.id);
        if self.environment_id == Some(env_id) || self.project.active_environment_id == Some(env_id) {
            self.project.active_environment_id = next;
            self.environment_id = next;
            self.load_environment(window, cx);
        }
        if self.config_modal == Some(super::ApiConfigModalSection::Environment(env_id)) {
            self.config_modal = next.map(super::ApiConfigModalSection::Environment);
        }
        self.project.touch();
        cx.notify();
    }

    /// 关闭环境变量与全局配置弹窗。
    pub(crate) fn close_config_modal(&mut self, cx: &mut Context<Self>) {
        self.config_modal = None;
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
