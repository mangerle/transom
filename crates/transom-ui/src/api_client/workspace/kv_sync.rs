//! API 工作区查询参数与请求头的实体回写。

use super::ApiWorkspace;
use crate::api_client::editors::kv::KvEntry;
use gpui_kit::*;
use transom_api::{
    ApiAssertion, ApiAuth, ApiCookie, ApiFormField, ApiGlobalParameter, ApiGlobalParameterTarget,
    ApiHeader, ApiQueryParam, ApiRequest, ApiVariable,
};

impl ApiWorkspace {
    /// 就地更新当前选中请求并刷新项目时间戳；仅在回调返回 `true` 时通知外层重绘。
    fn update_selected_request(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut ApiRequest) -> bool,
    ) {
        let Some(request_id) = self.selected_request else {
            return;
        };
        let Some(request) = self
            .project
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        else {
            return;
        };
        let should_notify = update(request);
        self.project.touch();
        if should_notify {
            cx.notify();
        }
    }

    pub(super) fn apply_environment_name(&mut self, cx: &mut Context<Self>) {
        let Some(environment_id) = self.active_environment_id() else {
            return;
        };
        let name = self
            .editors
            .environment_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }
        let Some(environment) = self
            .project
            .environments
            .iter_mut()
            .find(|environment| environment.id == environment_id)
        else {
            return;
        };
        environment.name = name;
        self.project.touch();
        cx.notify();
    }

    pub(super) fn apply_request_name(&mut self, cx: &mut Context<Self>) {
        let name = self.editors.request_name.read(cx).value().to_string();
        self.update_selected_request(cx, |request| {
            let changed = request.name != name;
            request.name = name;
            changed
        });
    }

    pub(super) fn apply_query_entries(&mut self, entries: &[KvEntry], cx: &mut Context<Self>) {
        self.update_selected_request(cx, |request| {
            let prev_len = request.query.len();
            request.query = entries.iter().cloned().map(query_from_entry).collect();
            request.query.len() != prev_len
        });
    }

    pub(super) fn apply_cookies(&mut self, cookies: Vec<ApiCookie>, _cx: &mut Context<Self>) {
        self.project.cookies = cookies;
        self.project.touch();
    }

    pub(super) fn apply_environment_variables(
        &mut self,
        variables: Vec<ApiVariable>,
        _cx: &mut Context<Self>,
    ) {
        let Some(environment_id) = self.active_environment_id() else {
            return;
        };
        let Some(environment) = self
            .project
            .environments
            .iter_mut()
            .find(|environment| environment.id == environment_id)
        else {
            return;
        };
        environment.variables = variables;
        self.project.active_environment_id = Some(environment_id);
        self.project.touch();
    }

    pub(super) fn apply_global_variables(
        &mut self,
        variables: Vec<ApiVariable>,
        _cx: &mut Context<Self>,
    ) {
        self.project.global_variables = variables;
        self.project.touch();
    }

    pub(super) fn apply_global_parameters(
        &mut self,
        target: ApiGlobalParameterTarget,
        entries: &[KvEntry],
        _cx: &mut Context<Self>,
    ) {
        let mut parameters = Vec::with_capacity(
            self.project
                .global_parameters
                .len()
                .saturating_add(entries.len()),
        );
        parameters.extend(
            self.project
                .global_parameters
                .iter()
                .filter(|parameter| parameter.target != target)
                .cloned(),
        );
        parameters.extend(
            entries
                .iter()
                .cloned()
                .map(|entry| global_parameter_from_entry(target, entry)),
        );
        self.project.global_parameters = parameters;
        self.project.touch();
    }

    pub(super) fn apply_assertions(
        &mut self,
        assertions: Vec<ApiAssertion>,
        cx: &mut Context<Self>,
    ) {
        self.update_selected_request(cx, |request| {
            let prev_len = request.assertions.len();
            request.assertions = assertions;
            request.assertions.len() != prev_len
        });
    }

    pub(super) fn apply_auth(&mut self, auth: ApiAuth, _cx: &mut Context<Self>) {
        self.update_selected_request(_cx, |request| {
            request.auth = auth;
            false
        });
    }

    pub(super) fn apply_form_fields(&mut self, fields: &[ApiFormField], cx: &mut Context<Self>) {
        self.update_selected_request(cx, |request| {
            let prev_len = request.form_fields.len();
            request.form_fields = fields.to_vec();
            request.form_fields.len() != prev_len
        });
    }

    pub(super) fn apply_header_entries(&mut self, entries: &[KvEntry], cx: &mut Context<Self>) {
        self.update_selected_request(cx, |request| {
            let prev_len = request.headers.len();
            request.headers = entries.iter().cloned().map(header_from_entry).collect();
            request.headers.len() != prev_len
        });
    }
}

pub(super) fn query_entry(param: &ApiQueryParam) -> KvEntry {
    KvEntry {
        name: param.name.clone(),
        value: param.value.clone(),
        description: param.description.clone(),
        enabled: param.enabled,
    }
}

pub(super) fn header_entry(header: &ApiHeader) -> KvEntry {
    KvEntry {
        name: header.name.clone(),
        value: header.value.clone(),
        description: header.description.clone(),
        enabled: header.enabled,
    }
}

pub(super) fn query_from_entry(entry: KvEntry) -> ApiQueryParam {
    ApiQueryParam {
        name: entry.name,
        value: entry.value,
        description: entry.description,
        enabled: entry.enabled,
    }
}

pub(super) fn header_from_entry(entry: KvEntry) -> ApiHeader {
    ApiHeader {
        name: entry.name,
        value: entry.value,
        description: entry.description,
        enabled: entry.enabled,
    }
}

fn global_parameter_from_entry(
    target: ApiGlobalParameterTarget,
    entry: KvEntry,
) -> ApiGlobalParameter {
    ApiGlobalParameter {
        target,
        name: entry.name,
        value: entry.value,
        description: entry.description,
        enabled: entry.enabled,
    }
}
