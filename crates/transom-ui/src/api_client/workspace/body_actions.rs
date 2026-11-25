//! HTTP 请求体模式、raw 语言和 Binary 文件动作。

use super::{ApiBodyMode, ApiWorkspace};
use gpui_kit::*;
use transom_api::{ApiBodyType, ApiRawLanguage};

impl ApiWorkspace {
    /// 切换 HTTP Body 类型，并同步表单编辑器的文件能力。
    pub(crate) fn set_body_type(
        &mut self,
        body_type: ApiBodyType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_editor(cx);
        let Some(request_id) = self.selected_request else {
            return;
        };
        if let Some(request) = self
            .project
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        {
            request.body_type = body_type;
            if let Some(language) = body_type.raw_language() {
                request.raw_language = language;
            }
            self.project.touch();
        }
        self.editors.form.update(cx, |editor, cx| {
            editor.set_allow_files(body_type == transom_api::ApiBodyType::Multipart, window, cx);
        });
        cx.notify();
    }

    /// 选择 Binary 请求体文件，路径只在发送时读取。
    pub(crate) fn pick_binary_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self.binary_file_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(request_id) = this.selected_request else {
                    return;
                };
                if let Some(request) = this
                    .project
                    .requests
                    .iter_mut()
                    .find(|request| request.id == request_id)
                {
                    request.body_type = transom_api::ApiBodyType::Binary;
                    request.body_file = format!("file://{}", path.display());
                    this.project.touch();
                    this.editors.form.update(cx, |editor, cx| {
                        editor.set_allow_files(false, window, cx);
                    });
                    cx.notify();
                }
            });
        }));
    }

    /// 切换顶层请求体模式；raw 模式复用请求上次选择的语言。
    pub(crate) fn set_body_mode(
        &mut self,
        mode: ApiBodyMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw_language = self
            .selected_request()
            .map_or(ApiRawLanguage::Json, |request| {
                request
                    .body_type
                    .raw_language()
                    .unwrap_or(request.raw_language)
            });
        self.set_body_type(mode.body_type(raw_language), window, cx);
    }

    /// 切换 raw 文本语言，并同步实际发送的 Body 类型。
    pub(crate) fn set_raw_language(
        &mut self,
        language: ApiRawLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_editor(cx);
        let Some(request_id) = self.selected_request else {
            return;
        };
        if let Some(request) = self
            .project
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        {
            request.raw_language = language;
            request.body_type = language.body_type();
            self.project.touch();
        }
        self.editors.form.update(cx, |editor, cx| {
            editor.set_allow_files(false, window, cx);
        });
        let token = raw_language_token(language);
        self.editors.request_body.update(cx, |state, cx| {
            state.set_highlighter(token, cx);
        });
        cx.notify();
    }

    /// 格式化当前请求体内容（当前支持 JSON 美化缩进）。
    pub(crate) fn format_request_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.editors.request_body.read(cx).value().to_string();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) => {
                if let Ok(formatted) = serde_json::to_string_pretty(&value) {
                    self.editors.request_body.update(cx, |state, cx| {
                        state.set_value(formatted, window, cx);
                    });
                    self.capture_editor(cx);
                    self.error = None;
                    cx.notify();
                }
            }
            Err(_) => {
                self.error =
                    Some(rust_i18n::t!("api.request.format_body_invalid_json").to_string());
                cx.notify();
            }
        }
    }
}

/// 将 raw 语言枚举映射为 syntect 语法名称。
pub(super) fn raw_language_token(language: ApiRawLanguage) -> &'static str {
    match language {
        ApiRawLanguage::Json => "json",
        ApiRawLanguage::Xml => "xml",
        ApiRawLanguage::JavaScript => "javascript",
        ApiRawLanguage::Html => "html",
        ApiRawLanguage::Text => "text",
    }
}
