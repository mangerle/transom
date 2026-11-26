//! API 环境变量与单次模板解析。

use super::ApiVariable;
use super::util::normalize_name;
use crate::ApiEnvironmentId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use transom_core::AppError;

/// 命名环境；变量值可以在请求、请求头和请求体中使用 `{{name}}` 引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiEnvironment {
    #[serde(default)]
    pub id: ApiEnvironmentId,
    pub name: String,
    #[serde(default)]
    pub variables: Vec<ApiVariable>,
}

impl ApiEnvironment {
    /// 新建环境。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ApiEnvironmentId::new(),
            name: normalize_name(name.into(), "未命名环境"),
            variables: Vec::new(),
        }
    }

    /// 创建没有变量的匿名环境。
    pub fn empty() -> Self {
        Self {
            id: ApiEnvironmentId::new(),
            name: "本地".to_string(),
            variables: Vec::new(),
        }
    }

    /// 解析变量模板；未知变量保留原样，便于用户看到缺失配置。
    ///
    /// 解析严格单次进行，变量值自身包含的 `{{...}}` 不会被再次展开。该语义避免
    /// 变量顺序改变最终请求，也防止变量互相引用导致无界递归或指数级膨胀。
    pub fn resolve(&self, text: &str) -> String {
        self.resolver().resolve(text)
    }

    /// 为一次请求准备可复用的变量查找表。
    pub fn resolver(&self) -> ApiEnvironmentResolver<'_> {
        ApiEnvironmentResolver::new(self)
    }

    /// 将环境变量渲染为 `NAME=value` 文本。
    pub fn variables_text(&self) -> String {
        self.variables
            .iter()
            .map(|variable| format!("{}={}", variable.name, variable.value))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 从 `NAME=value` 文本替换变量；缺少等号时返回带行号的错误。
    ///
    /// # Errors
    /// 非空行没有 `=` 或变量名为空时返回 [`AppError`]。
    pub fn set_variables_text(&mut self, text: &str) -> Result<(), AppError> {
        let mut variables = Vec::with_capacity(text.lines().count());
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((name, value)) = trimmed.split_once('=') else {
                return Err(AppError::with_detail(
                    transom_core::ErrorCode::Configuration,
                    format!("环境变量第 {} 行缺少等号: {}", index + 1, trimmed),
                ));
            };
            let name = name.trim();
            if name.is_empty() {
                return Err(AppError::with_detail(
                    transom_core::ErrorCode::Configuration,
                    format!("环境变量第 {} 行名称为空", index + 1),
                ));
            }
            variables.push(ApiVariable {
                name: name.to_string(),
                value: value.trim().to_string(),
                enabled: true,
                secret: false,
            });
        }
        self.variables = variables;
        Ok(())
    }
}

/// 一次请求内复用的环境变量查找表。
///
/// 同名变量遵循原有“先出现者生效”语义。查找表只借用环境中的字符串，不复制变量
/// 值；相比逐变量全串替换，它把解析复杂度从“文本长度乘以变量数量”降为“文本长度
/// 加变量数量”。
pub struct ApiEnvironmentResolver<'a> {
    variables: HashMap<&'a str, &'a str>,
}

impl<'a> ApiEnvironmentResolver<'a> {
    fn new(environment: &'a ApiEnvironment) -> Self {
        let mut variables = HashMap::with_capacity(environment.variables.len());
        for variable in environment
            .variables
            .iter()
            .filter(|variable| variable.enabled && !variable.name.is_empty())
        {
            variables
                .entry(variable.name.as_str())
                .or_insert(variable.value.as_str());
        }
        Self { variables }
    }

    /// 单次替换文本中的 `{{name}}`；未知名称保持原样。
    pub fn resolve(&self, text: &str) -> String {
        if !text.contains("{{") {
            return text.to_string();
        }
        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;
        while let Some(relative_open) = text[cursor..].find("{{") {
            let open = cursor.saturating_add(relative_open);
            output.push_str(&text[cursor..open]);
            let Some(relative_close) = text[open.saturating_add(2)..].find("}}") else {
                output.push_str(&text[open..]);
                return output;
            };
            let close = open.saturating_add(2).saturating_add(relative_close);
            let name = &text[open.saturating_add(2)..close];
            if let Some(value) = self.variables.get(name) {
                output.push_str(value);
            } else {
                output.push_str(&text[open..close.saturating_add(2)]);
            }
            cursor = close.saturating_add(2);
        }
        output.push_str(&text[cursor..]);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str, value: &str) -> ApiVariable {
        ApiVariable {
            name: name.to_string(),
            value: value.to_string(),
            enabled: true,
            secret: false,
        }
    }

    #[test]
    fn resolver_expands_once_without_recursive_substitution() {
        let mut environment = ApiEnvironment::empty();
        environment.variables = vec![variable("A", "{{B}}"), variable("B", "expanded")];
        assert_eq!(environment.resolve("{{A}}"), "{{B}}");
    }

    #[test]
    fn resolver_preserves_unknown_and_unclosed_templates() {
        let mut environment = ApiEnvironment::empty();
        environment.variables = vec![variable("TOKEN", "secret")];
        assert_eq!(environment.resolve("x={{MISSING}}"), "x={{MISSING}}");
        assert_eq!(environment.resolve("x={{TOKEN"), "x={{TOKEN");
    }

    #[test]
    fn duplicate_variables_keep_first_enabled_value() {
        let mut environment = ApiEnvironment::empty();
        environment.variables = vec![variable("HOST", "first"), variable("HOST", "second")];
        assert_eq!(environment.resolve("{{HOST}}"), "first");
    }
}
