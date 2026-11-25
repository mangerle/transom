//! 项目级全局变量合并与全局参数注入。

use super::{ApiEnvironment, ApiHeader, ApiProject, ApiQueryParam, ApiRequest};
use serde::{Deserialize, Serialize};

/// 全局参数注入位置。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiGlobalParameterTarget {
    #[default]
    Query,
    Header,
}

/// 项目级全局参数；对项目内所有请求生效，但请求自身同名项优先。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiGlobalParameter {
    pub target: ApiGlobalParameterTarget,
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl ApiProject {
    /// 当前环境与全局变量合并后的有效环境；同名环境变量覆盖全局变量。
    pub fn merged_environment(&self, environment: &ApiEnvironment) -> ApiEnvironment {
        let mut merged = environment.clone();
        merged.variables = self.global_variables.clone();
        for variable in &environment.variables {
            if let Some(existing) = merged
                .variables
                .iter_mut()
                .find(|existing| existing.name == variable.name)
            {
                *existing = variable.clone();
            } else {
                merged.variables.push(variable.clone());
            }
        }
        merged
    }

    /// 解析请求并注入项目级全局 Query/Header 参数。
    ///
    /// 请求自身已声明的同名 Query 或 Header 优先，全局参数只补缺，不覆盖用户在
    /// 单个请求上的显式配置。
    pub fn resolved_request(
        &self,
        request: &ApiRequest,
        environment: &ApiEnvironment,
    ) -> ApiRequest {
        let environment = self.merged_environment(environment);
        let mut merged_request = request.clone();
        for parameter in self
            .global_parameters
            .iter()
            .filter(|parameter| parameter.enabled)
        {
            let name = environment.resolve(parameter.name.trim());
            if name.is_empty() {
                continue;
            }
            match parameter.target {
                ApiGlobalParameterTarget::Query => {
                    if merged_request
                        .query
                        .iter()
                        .any(|existing| environment.resolve(existing.name.trim()) == name)
                    {
                        continue;
                    }
                    merged_request.query.push(ApiQueryParam {
                        name: parameter.name.clone(),
                        value: parameter.value.clone(),
                        description: parameter.description.clone(),
                        enabled: true,
                    });
                }
                ApiGlobalParameterTarget::Header => {
                    if merged_request.headers.iter().any(|existing| {
                        environment
                            .resolve(existing.name.trim())
                            .eq_ignore_ascii_case(&name)
                    }) {
                        continue;
                    }
                    merged_request.headers.push(ApiHeader {
                        name: parameter.name.clone(),
                        value: parameter.value.clone(),
                        description: parameter.description.clone(),
                        enabled: true,
                    });
                }
            }
        }
        let mut resolved = merged_request.resolved(&self.base_url, &environment);
        resolved.query.retain(|parameter| parameter.enabled);
        resolved.headers.retain(|header| header.enabled);
        resolved
    }
}
