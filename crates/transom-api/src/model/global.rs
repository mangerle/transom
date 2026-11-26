//! 项目级全局变量合并与全局参数注入。

use super::{ApiEnvironment, ApiHeader, ApiProject, ApiQueryParam, ApiRequest};
use crate::{ApiError, ApiExecutionPlan};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
        let mut index_by_name = HashMap::with_capacity(
            self.global_variables
                .len()
                .saturating_add(environment.variables.len()),
        );
        for (index, variable) in merged.variables.iter().enumerate() {
            index_by_name.entry(variable.name.clone()).or_insert(index);
        }
        for variable in &environment.variables {
            if let Some(index) = index_by_name.get(&variable.name).copied() {
                merged.variables[index] = variable.clone();
            } else {
                index_by_name.insert(variable.name.clone(), merged.variables.len());
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
        self.resolve_request_with_environment(request, &environment)
    }

    /// 生成只解析一次、可供执行器直接消费的请求计划。
    ///
    /// 与环境解析入口相比，该计划额外固定 Cookie 快照，并在进入异步网络层前完成
    /// 请求体结构校验；因此执行器无需也不应再次解释 `{{变量}}`。
    ///
    /// # Errors
    /// 请求体缺少文件或包含当前编码不支持的文件字段时返回 [`ApiError::InvalidBody`]。
    pub fn execution_plan(
        &self,
        request: &ApiRequest,
        environment: &ApiEnvironment,
    ) -> Result<ApiExecutionPlan, ApiError> {
        let environment = self.merged_environment(environment);
        let request = self.resolve_request_with_environment(request, &environment);
        ApiExecutionPlan::new(request, environment, self.cookies.clone())
    }

    fn resolve_request_with_environment(
        &self,
        request: &ApiRequest,
        environment: &ApiEnvironment,
    ) -> ApiRequest {
        let resolver = environment.resolver();
        let mut merged_request = request.clone();
        for parameter in self
            .global_parameters
            .iter()
            .filter(|parameter| parameter.enabled)
        {
            let name = resolver.resolve(parameter.name.trim());
            if name.is_empty() {
                continue;
            }
            match parameter.target {
                ApiGlobalParameterTarget::Query => {
                    if merged_request
                        .query
                        .iter()
                        .any(|existing| resolver.resolve(existing.name.trim()) == name)
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
                        resolver
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
        let mut resolved = merged_request.resolved_with_resolver(&self.base_url, &resolver);
        resolved.query.retain(|parameter| parameter.enabled);
        resolved.headers.retain(|header| header.enabled);
        resolved
    }
}
