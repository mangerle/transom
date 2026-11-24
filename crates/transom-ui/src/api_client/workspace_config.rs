//! API 工作区环境变量与 Cookie 草稿回写。

use super::workspace::ApiWorkspace;
use gpui_kit::*;
use transom_api::parse_cookies_text;

impl ApiWorkspace {
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
