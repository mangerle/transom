//! API 工作区查询参数与请求头的实体回写。

use super::kv_editor::KvEntry;
use super::workspace::ApiWorkspace;
use gpui_kit::*;
use transom_api::{ApiFormField, ApiHeader, ApiQueryParam};

impl ApiWorkspace {
    pub(super) fn apply_query_entries(&mut self, entries: &[KvEntry], cx: &mut Context<Self>) {
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
        request.query = entries.iter().cloned().map(query_from_entry).collect();
        self.project.touch();
        cx.notify();
    }

    pub(super) fn apply_form_fields(&mut self, fields: &[ApiFormField], cx: &mut Context<Self>) {
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
        request.form_fields = fields.to_vec();
        self.project.touch();
        cx.notify();
    }

    pub(super) fn apply_header_entries(&mut self, entries: &[KvEntry], cx: &mut Context<Self>) {
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
        request.headers = entries.iter().cloned().map(header_from_entry).collect();
        self.project.touch();
        cx.notify();
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
