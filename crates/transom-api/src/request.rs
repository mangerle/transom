//! API 请求、协议、环境模板与认证配置。

use self::request_text::{normalize_request_name, parse_assertion, parse_headers};
use super::{ApiEnvironment, ApiError, ApiRequestId};
use serde::{Deserialize, Serialize};
use std::fmt;

mod request_text;
mod validation;

pub use self::request_text::method_for_request;

/// HTTP 方法；与常用 API 客户端保持同一集合。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ApiMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl ApiMethod {
    /// 方法展示文本。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    /// 下拉菜单使用的稳定顺序。
    pub const ALL: [Self; 7] = [
        Self::Get,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
        Self::Head,
        Self::Options,
    ];

    /// 该方法是否通常携带请求体，用于决定是否显示 Body 页签。
    pub const fn has_body(self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch | Self::Delete)
    }
}

impl fmt::Display for ApiMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 请求承载的协议类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProtocol {
    #[default]
    Http,
    GraphQl,
    Grpc,
    WebSocket,
}

impl ApiProtocol {
    /// 协议展示文本。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::GraphQl => "GraphQL",
            Self::Grpc => "gRPC",
            Self::WebSocket => "WebSocket",
        }
    }
}

/// 一条请求头；`enabled` 保留编辑器的启用/禁用能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiHeader {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl ApiHeader {
    /// 用名称、值与启用状态创建请求头，说明留空。
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: String::new(),
            enabled: true,
        }
    }
}

/// 一条 URL 查询参数；与请求头共享四列表格结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiQueryParam {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl ApiQueryParam {
    /// 用名称、值与启用状态创建查询参数，说明留空。
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: String::new(),
            enabled: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// HTTP 请求体的编辑与发送类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiBodyType {
    /// 不发送请求体。
    None,
    /// JSON 文本，保留用户原始格式。
    Json,
    /// 普通文本。
    #[default]
    Text,
    /// XML 文本。
    Xml,
    /// `application/x-www-form-urlencoded` 键值表单。
    FormUrlEncoded,
    /// `multipart/form-data` 键值表单，支持文件。
    Multipart,
    /// 单个二进制文件。
    Binary,
}

impl ApiBodyType {
    /// 稳定的下拉顺序。
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Json,
        Self::Text,
        Self::Xml,
        Self::FormUrlEncoded,
        Self::Multipart,
        Self::Binary,
    ];

    /// 配置文本使用的稳定值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json => "json",
            Self::Text => "text",
            Self::Xml => "xml",
            Self::FormUrlEncoded => "form_urlencoded",
            Self::Multipart => "multipart",
            Self::Binary => "binary",
        }
    }

    /// 是否使用表单字段列表发送。
    pub const fn is_form(self) -> bool {
        matches!(self, Self::FormUrlEncoded | Self::Multipart)
    }
}

/// 表单字段的值类型；文件只允许用于 multipart。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormFieldKind {
    #[default]
    Text,
    File,
}

/// 一条表单字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiFormField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ApiFormFieldKind,
}

impl ApiFormField {
    /// 创建启用的文本表单字段。
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: String::new(),
            enabled: true,
            kind: ApiFormFieldKind::Text,
        }
    }
}

/// API 请求的认证配置。
///
/// 设计原理（Why）：认证配置只保存环境变量名，不把令牌、客户端密钥或 AWS
/// Secret 直接写进项目文件；执行时再由当前环境解析出真实值。OAuth 2.0 的
/// client credentials 与 AWS SigV4 都通过显式字段建模，不把互斥信息塞进多个
/// 布尔标志。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApiAuth {
    #[default]
    None,
    Bearer {
        token_env: String,
    },
    Basic {
        username_env: String,
        password_env: String,
    },
    OAuth2ClientCredentials {
        token_url: String,
        client_id: String,
        client_secret_env: String,
        #[serde(default)]
        scope: String,
    },
    AwsSigV4 {
        access_key_env: String,
        secret_key_env: String,
        #[serde(default)]
        session_token_env: String,
        region: String,
        service: String,
    },
}

/// 一条请求断言，使用可读文本保存在项目和 UI 中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiAssertion {
    pub expression: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// 断言执行结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiAssertionResult {
    pub expression: String,
    pub passed: bool,
    pub detail: String,
}

/// 一条可保存、可发送的 API 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiRequest {
    #[serde(default)]
    pub id: ApiRequestId,
    #[serde(default)]
    pub folder_id: Option<super::ApiFolderId>,
    pub name: String,
    #[serde(default)]
    pub protocol: ApiProtocol,
    #[serde(default)]
    pub method: ApiMethod,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub query: Vec<ApiQueryParam>,
    #[serde(default)]
    pub headers: Vec<ApiHeader>,
    #[serde(default)]
    pub body_type: ApiBodyType,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub form_fields: Vec<ApiFormField>,
    #[serde(default)]
    pub body_file: String,
    #[serde(default)]
    pub graphql_variables: String,
    #[serde(default)]
    pub grpc_service: String,
    #[serde(default)]
    pub grpc_method: String,
    #[serde(default)]
    pub auth: ApiAuth,
    #[serde(default)]
    pub assertions: Vec<ApiAssertion>,
}

impl ApiRequest {
    /// 新建请求；URL 留空，由编辑器填写。
    pub fn new(name: impl Into<String>, folder_id: Option<super::ApiFolderId>) -> Self {
        Self {
            id: ApiRequestId::new(),
            folder_id,
            name: normalize_request_name(name.into()),
            protocol: ApiProtocol::Http,
            method: ApiMethod::Get,
            url: String::new(),
            description: String::new(),
            timeout_seconds: 0,
            query: Vec::new(),
            headers: Vec::new(),
            body_type: ApiBodyType::Text,
            body: String::new(),
            form_fields: Vec::new(),
            body_file: String::new(),
            graphql_variables: "{}".to_string(),
            grpc_service: String::new(),
            grpc_method: String::new(),
            auth: ApiAuth::None,
            assertions: Vec::new(),
        }
    }

    /// 请求展示名。
    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            "未命名请求"
        } else {
            name
        }
    }

    /// 复制请求内容并生成新的稳定标识；名称追加“副本”便于在接口树中区分。
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.id = ApiRequestId::new();
        copy.name = normalize_request_name(format!("{} 副本", self.display_name()));
        copy
    }

    /// 发送时将相对地址接到项目基础地址后，并合并启用中的查询参数。
    pub fn resolved_url(&self, base_url: &str, environment: &ApiEnvironment) -> String {
        let url = environment.resolve(self.url.trim());
        let base = environment
            .resolve(base_url.trim())
            .trim_end_matches('/')
            .to_string();
        let url = if url.starts_with('/') && !base.is_empty() {
            format!("{base}{url}")
        } else {
            url
        };
        let enabled = self
            .query
            .iter()
            .filter(|param| param.enabled && !param.name.trim().is_empty());
        if enabled.clone().next().is_none() {
            return url;
        }
        let Ok(mut parsed) = reqwest::Url::parse(&url) else {
            return url;
        };
        {
            let mut pairs = parsed.query_pairs_mut();
            for param in enabled {
                pairs.append_pair(
                    environment.resolve(param.name.trim()).as_str(),
                    environment.resolve(param.value.trim()).as_str(),
                );
            }
        }
        parsed.to_string()
    }

    /// 将启用的请求头渲染为一行一个 `名称: 值` 文本。
    pub fn header_text(&self) -> String {
        self.headers
            .iter()
            .filter(|header| header.enabled)
            .map(|header| format!("{}: {}", header.name, header.value))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 从文本解析请求头；空行忽略，格式错误带行号返回。
    ///
    /// # Errors
    /// 任意非空行缺少冒号或名称为空时返回带中文上下文的 `ApiError`。
    pub fn set_header_text(&mut self, text: &str) -> Result<(), ApiError> {
        self.headers = parse_headers(text)?;
        Ok(())
    }

    /// 认证配置的 JSON 编辑文本。
    pub fn auth_text(&self) -> String {
        serde_json::to_string_pretty(&self.auth)
            .unwrap_or_else(|_| "{\"kind\":\"none\"}".to_string())
    }

    /// 从 JSON 文本解析认证配置。
    ///
    /// # Errors
    /// JSON 格式错误或认证字段不符合模型时返回 `ApiError`。
    pub fn set_auth_text(&mut self, text: &str) -> Result<(), ApiError> {
        let value = text.trim();
        if value.is_empty() {
            self.auth = ApiAuth::None;
            return Ok(());
        }
        self.auth = serde_json::from_str(value).map_err(|error| ApiError::InvalidAuth {
            reason: error.to_string(),
        })?;
        Ok(())
    }

    /// 断言逐行编辑文本。
    pub fn assertions_text(&self) -> String {
        self.assertions
            .iter()
            .map(|assertion| assertion.expression.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 从逐行文本解析断言。
    ///
    /// # Errors
    /// 不支持的断言语法返回 `ApiError`，不会静默丢弃用户输入。
    pub fn set_assertions_text(&mut self, text: &str) -> Result<(), ApiError> {
        let mut assertions = Vec::with_capacity(text.lines().count());
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            parse_assertion(trimmed).map_err(|reason| ApiError::InvalidAssertion {
                line: index.saturating_add(1),
                reason,
            })?;
            assertions.push(ApiAssertion {
                expression: trimmed.to_string(),
                enabled: true,
            });
        }
        self.assertions = assertions;
        Ok(())
    }

    /// 按当前环境生成用于发送的副本，解析 URL、头、体和 GraphQL 变量。
    pub fn resolved(&self, base_url: &str, environment: &ApiEnvironment) -> Self {
        let mut resolved = self.clone();
        resolved.url = self.resolved_url(base_url, environment);
        for query in &mut resolved.query {
            query.name = environment.resolve(&query.name);
            query.value = environment.resolve(&query.value);
            query.description = environment.resolve(&query.description);
        }
        for header in &mut resolved.headers {
            header.name = environment.resolve(&header.name);
            header.value = environment.resolve(&header.value);
            header.description = environment.resolve(&header.description);
        }
        resolved.body = environment.resolve(&self.body);
        resolved.body_file = environment.resolve(&self.body_file);
        for field in &mut resolved.form_fields {
            field.name = environment.resolve(&field.name);
            field.value = environment.resolve(&field.value);
            field.description = environment.resolve(&field.description);
        }
        resolved.graphql_variables = environment.resolve(&self.graphql_variables);
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::super::ApiResponseSnapshot;
    use super::super::assertions::{assertion_summary, evaluate_assertions};
    use super::*;

    #[test]
    fn request_headers_and_environment_resolve() {
        let mut request = ApiRequest::new("测试", None);
        request.url = "https://{{HOST}}".to_string();
        request
            .set_header_text("Authorization: Bearer {{TOKEN}}\nX-Host: {{HOST}}")
            .expect("请求头应合法");
        request.query.push(ApiQueryParam::new("q", "{{TOKEN}}"));
        let mut environment = ApiEnvironment::new("测试");
        environment
            .set_variables_text("TOKEN=abc\nHOST=example.com")
            .expect("环境变量应合法");
        let resolved = request.resolved("https://{{HOST}}", &environment);
        assert_eq!(resolved.url, "https://example.com/?q=abc");
        assert_eq!(
            resolved.header_text(),
            "Authorization: Bearer abc\nX-Host: example.com"
        );
        let query = reqwest::Url::parse(&resolved.url)
            .expect("解析后的地址应可再次解析")
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        assert_eq!(query, vec![("q".to_string(), "abc".to_string())]);
    }

    #[test]
    fn assertions_are_parsed_and_evaluated() {
        let mut request = ApiRequest::new("测试", None);
        request
            .set_assertions_text("status == 200\njson ok == true\nbody contains ok")
            .expect("断言应合法");
        request.assertions.push(ApiAssertion {
            expression: "status != 500".to_string(),
            enabled: false,
        });
        let response = ApiResponseSnapshot {
            status: 200,
            status_text: "OK".to_string(),
            headers: Vec::new(),
            body: "{\"ok\":true}".to_string(),
            body_size: 11,
            elapsed_millis: 1,
            set_cookies: Vec::new(),
        };
        let results = evaluate_assertions(&request, &response);
        assert_eq!(assertion_summary(&results), (3, 3));
    }

    #[test]
    fn auth_config_round_trips_through_json() {
        let mut request = ApiRequest::new("测试", None);
        request.auth = ApiAuth::Bearer {
            token_env: "TOKEN".to_string(),
        };
        let text = request.auth_text();
        request.set_auth_text(&text).expect("认证配置应可回读");
        assert!(matches!(request.auth, ApiAuth::Bearer { .. }));
    }

    #[test]
    fn duplicate_keeps_content_and_replaces_identity() {
        let mut request = ApiRequest::new("用户列表", None);
        request.url = "https://example.com/users".to_string();
        request
            .set_header_text("Accept: application/json")
            .expect("请求头应合法");

        let duplicate = request.duplicate();

        assert_ne!(duplicate.id, request.id);
        assert_eq!(duplicate.name, "用户列表 副本");
        assert_eq!(duplicate.url, request.url);
        assert_eq!(duplicate.headers, request.headers);
    }

    #[test]
    fn removing_folder_moves_requests_to_project_root() {
        let mut project = super::super::ApiProject::new("目录测试");
        let folder = project.add_folder("用户");
        let request = ApiRequest::new("列表", Some(folder.id));
        let request_id = request.id;
        project.upsert_request(request);

        assert!(project.remove_folder(folder.id));
        assert_eq!(
            project
                .request(request_id)
                .and_then(|request| request.folder_id),
            None
        );
    }
}
