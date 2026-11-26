//! 已完成环境合并与模板解析的请求执行计划。

use super::{ApiEnvironment, ApiRequest};
use crate::{ApiCookie, ApiError};

/// 可直接交给协议执行器的请求快照。
///
/// 设计原理（Why）：原始接口定义允许包含 `{{变量}}`，而执行器只应消费一次解析后的
/// 稳定数据。把合并环境、解析请求和 Cookie 快照收进同一类型，可以从 API 契约上
/// 区分“可编辑草稿”与“可执行快照”，避免 UI 和底层重复解析同一条请求。
///
/// 代价与局限：计划会持有一份解析后的请求与环境克隆，单次发送期间会增加一次
/// 请求级内存拷贝；换取的是执行路径没有隐式变量替换，协议实现更可预测。
#[derive(Debug, Clone)]
pub struct ApiExecutionPlan {
    request: ApiRequest,
    environment: ApiEnvironment,
    cookies: Vec<ApiCookie>,
}

impl ApiExecutionPlan {
    pub(crate) fn new(
        request: ApiRequest,
        environment: ApiEnvironment,
        cookies: Vec<ApiCookie>,
    ) -> Result<Self, ApiError> {
        request.validate()?;
        Ok(Self {
            request,
            environment,
            cookies,
        })
    }

    /// 已解析、可直接发送的请求。
    pub fn request(&self) -> &ApiRequest {
        &self.request
    }

    /// 合并全局变量后的环境，仅用于解析认证所需变量。
    pub fn environment(&self) -> &ApiEnvironment {
        &self.environment
    }

    /// 本次执行开始时的 Cookie 快照。
    pub fn cookies(&self) -> &[ApiCookie] {
        &self.cookies
    }

    pub(crate) fn into_parts(self) -> (ApiRequest, ApiEnvironment, Vec<ApiCookie>) {
        (self.request, self.environment, self.cookies)
    }
}
