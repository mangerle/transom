//! API 工作区环境变量与 Cookie 草稿回写。

use super::workspace::ApiWorkspace;
use gpui_kit::*;
use transom_api::{ApiEnvironment, parse_cookies_text};

impl ApiWorkspace {
    /// 将环境变量编辑区写回当前环境；格式错误时保留编辑器文本并展示提示。
    pub(super) fn capture_environment(&mut self, cx: &App) {
        let text = self.editors.environment.read(cx).value().to_string();
        let environment_id = self
            .environment_id
            .or(self.project.active_environment_id)
            .and_then(|id| {
                self.project
                    .environments
                    .iter()
                    .any(|environment| environment.id == id)
                    .then_some(id)
            });
        let environment_id = environment_id.unwrap_or_else(|| {
            let environment = ApiEnvironment::empty();
            let id = environment.id;
            self.project.environments.push(environment);
            id
        });
        if let Some(environment) = self
            .project
            .environments
            .iter_mut()
            .find(|environment| environment.id == environment_id)
        {
            if let Err(error) = environment.set_variables_text(&text) {
                self.error =
                    Some(error.detail.unwrap_or_else(|| {
                        rust_i18n::t!("api.error.environment_format").to_string()
                    }));
            }
            self.project.active_environment_id = Some(environment_id);
        }
    }

    /// 将 Cookie 编辑区写回项目，并保留响应自动维护的域名 Cookie。
    pub(super) fn capture_cookies(&mut self, cx: &App) {
        let text = self.editors.cookies.read(cx).value().to_string();
        match parse_cookies_text(&text) {
            Ok(cookies) => {
                let mut merged = cookies;
                for existing in self
                    .project
                    .cookies
                    .iter()
                    .filter(|cookie| cookie.domain.is_some())
                {
                    merged.push(existing.clone());
                }
                self.project.cookies = merged;
            }
            Err(error) => {
                self.error = Some(
                    error
                        .detail
                        .unwrap_or_else(|| rust_i18n::t!("api.error.cookie_format").to_string()),
                );
            }
        }
    }
}
