//! 请求编辑器文本与轻量展示之间的解析适配。

use super::{ApiError, ApiHeader, ApiProtocol, ApiRequest};

/// 解析一行断言的语法，不执行断言；实际执行由断言模块负责。
pub(super) fn parse_assertion(expression: &str) -> Result<(), String> {
    let lower = expression.to_lowercase();
    if lower.starts_with("status ") {
        let rest = expression[6..].trim();
        if rest.starts_with("==") || rest.starts_with("!=") {
            return Ok(());
        }
        return Err("status 断言必须使用 == 或 !=".to_string());
    }
    for prefix in ["body ", "header ", "json ", "text "] {
        if lower.starts_with(prefix) {
            return Ok(());
        }
    }
    Err("断言必须以 status、body、header、json 或 text 开头".to_string())
}

/// 解析请求头文本。
pub(super) fn parse_headers(text: &str) -> Result<Vec<ApiHeader>, ApiError> {
    let mut headers = Vec::with_capacity(text.lines().count());
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            return Err(ApiError::InvalidHeader {
                line: index.saturating_add(1),
                reason: "缺少冒号分隔符".to_string(),
            });
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(ApiError::InvalidHeader {
                line: index.saturating_add(1),
                reason: "请求头名称为空".to_string(),
            });
        }
        headers.push(ApiHeader {
            name: name.to_string(),
            value: value.trim().to_string(),
            description: String::new(),
            enabled: true,
        });
    }
    Ok(headers)
}

pub(super) fn normalize_request_name(name: String) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "未命名请求".to_string();
    }
    trimmed.chars().take(120).collect()
}

/// 为请求生成稳定的显示方法文本。
pub fn method_for_request(request: &ApiRequest) -> String {
    match request.protocol {
        ApiProtocol::WebSocket => "WS".to_string(),
        ApiProtocol::Grpc => "GRPC".to_string(),
        ApiProtocol::GraphQl => "GRAPHQL".to_string(),
        ApiProtocol::Http => request.method.as_str().to_string(),
    }
}
