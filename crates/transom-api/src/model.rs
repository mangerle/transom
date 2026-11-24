//! API 项目、目录、环境、Cookie、历史和请求数据。

use super::ApiRequest;
use super::{ApiEnvironmentId, ApiFolderId, ApiProjectId, ApiRequestId};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use transom_core::AppError;

/// 当前项目文件结构版本；版本高于程序支持值时拒绝加载，避免静默丢失新字段。
pub const API_SCHEMA_VERSION: u32 = 2;
/// 单次项目导入文件上限，防止误选超大文件后在 UI 线程前产生无界内存增长。
pub const MAX_IMPORT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 120;
const MAX_HISTORY_ENTRIES: usize = 100;

/// API 项目登记表；一个文件保存全部本地项目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiRegistry {
    #[serde(default = "schema_version")]
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<ApiProject>,
}

impl Default for ApiRegistry {
    fn default() -> Self {
        Self {
            version: API_SCHEMA_VERSION,
            projects: Vec::new(),
        }
    }
}

fn schema_version() -> u32 {
    API_SCHEMA_VERSION
}

impl ApiRegistry {
    /// 查找项目。
    pub fn project(&self, id: ApiProjectId) -> Option<&ApiProject> {
        self.projects.iter().find(|project| project.id == id)
    }

    /// 插入或整体替换项目；调用方负责随后持久化。
    pub fn upsert(&mut self, project: ApiProject) {
        if let Some(existing) = self
            .projects
            .iter_mut()
            .find(|existing| existing.id == project.id)
        {
            *existing = project;
        } else {
            self.projects.push(project);
        }
    }

    /// 删除项目并返回是否确实移除了记录。
    pub fn remove(&mut self, id: ApiProjectId) -> bool {
        let before = self.projects.len();
        self.projects.retain(|project| project.id != id);
        before != self.projects.len()
    }
}

/// 一个本地 API 项目。目录只允许一级，接口可直接挂在项目根层或目录内。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiProject {
    #[serde(default)]
    pub id: ApiProjectId,
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub folders: Vec<ApiFolder>,
    #[serde(default)]
    pub requests: Vec<ApiRequest>,
    #[serde(default)]
    pub environments: Vec<ApiEnvironment>,
    #[serde(default)]
    pub active_environment_id: Option<ApiEnvironmentId>,
    #[serde(default)]
    pub cookies: Vec<ApiCookie>,
    #[serde(default)]
    pub history: Vec<ApiHistoryEntry>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl ApiProject {
    /// 创建空项目，并预置一个本地环境便于立即使用 `{{变量}}`。
    pub fn new(name: impl Into<String>) -> Self {
        let now = unix_now();
        let environment = ApiEnvironment::new("本地");
        let active_environment_id = Some(environment.id);
        Self {
            id: ApiProjectId::new(),
            name: normalize_name(name.into(), "未命名项目"),
            base_url: String::new(),
            folders: Vec::new(),
            requests: Vec::new(),
            environments: vec![environment],
            active_environment_id,
            cookies: Vec::new(),
            history: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// 项目展示名；空名称退回稳定占位，不把空字符串画到标签栏。
    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            "未命名项目"
        } else {
            name
        }
    }

    /// 接口数量，用于侧栏行摘要。
    pub fn request_count(&self) -> usize {
        self.requests.len()
    }

    /// 查找请求。
    pub fn request(&self, id: ApiRequestId) -> Option<&ApiRequest> {
        self.requests.iter().find(|request| request.id == id)
    }

    /// 当前活动环境；没有配置时返回内置空环境。
    pub fn active_environment(&self) -> ApiEnvironment {
        self.active_environment_id
            .and_then(|id| {
                self.environments
                    .iter()
                    .find(|environment| environment.id == id)
            })
            .cloned()
            .unwrap_or_else(ApiEnvironment::empty)
    }

    /// 插入或替换请求，并刷新项目更新时间。
    pub fn upsert_request(&mut self, request: ApiRequest) {
        if let Some(existing) = self
            .requests
            .iter_mut()
            .find(|existing| existing.id == request.id)
        {
            *existing = request;
        } else {
            self.requests.push(request);
        }
        self.touch();
    }

    /// 删除请求并返回是否命中。
    pub fn remove_request(&mut self, id: ApiRequestId) -> bool {
        let before = self.requests.len();
        self.requests.retain(|request| request.id != id);
        if before != self.requests.len() {
            self.touch();
            true
        } else {
            false
        }
    }

    /// 创建目录；目录内接口与根层接口使用同一套请求模型。
    pub fn add_folder(&mut self, name: impl Into<String>) -> ApiFolder {
        let folder = ApiFolder::new(name);
        self.folders.push(folder.clone());
        self.touch();
        folder
    }

    /// 删除目录并把其中的请求移回项目根层，避免删除目录时连带丢失请求。
    pub fn remove_folder(&mut self, id: ApiFolderId) -> bool {
        let before = self.folders.len();
        self.folders.retain(|folder| folder.id != id);
        if before == self.folders.len() {
            return false;
        }
        for request in &mut self.requests {
            if request.folder_id == Some(id) {
                request.folder_id = None;
            }
        }
        self.touch();
        true
    }

    /// 追加一条请求历史，超过上限时保留最近记录。
    pub fn push_history(&mut self, entry: ApiHistoryEntry) {
        self.history.push(entry);
        if self.history.len() > MAX_HISTORY_ENTRIES {
            let excess = self.history.len() - MAX_HISTORY_ENTRIES;
            self.history.drain(0..excess);
        }
        self.touch();
    }

    /// 用同名 Cookie 覆盖旧值；域名按小写比较。
    pub fn upsert_cookie(&mut self, cookie: ApiCookie) {
        if let Some(existing) = self.cookies.iter_mut().find(|existing| {
            existing.name.eq_ignore_ascii_case(&cookie.name)
                && optional_domain_eq(existing.domain.as_deref(), cookie.domain.as_deref())
                && existing.path == cookie.path
        }) {
            *existing = cookie;
        } else {
            self.cookies.push(cookie);
        }
        self.touch();
    }

    /// 更新项目元数据并刷新更新时间。
    pub fn touch(&mut self) {
        self.updated_at = unix_now();
    }
}

/// 单层接口目录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiFolder {
    #[serde(default)]
    pub id: ApiFolderId,
    pub name: String,
}

impl ApiFolder {
    /// 创建目录。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ApiFolderId::new(),
            name: normalize_name(name.into(), "未命名目录"),
        }
    }
}

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
    pub fn resolve(&self, text: &str) -> String {
        let mut resolved = text.to_string();
        for variable in self.variables.iter().filter(|variable| variable.enabled) {
            let needle = format!("{{{{{}}}}}", variable.name);
            resolved = resolved.replace(&needle, &variable.value);
        }
        resolved
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
    /// 非空行没有 `=` 或变量名为空时返回 `AppError`。
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

/// 单个环境变量；`secret` 仅用于界面遮罩，不改变模板替换行为。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiVariable {
    pub name: String,
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub secret: bool,
}

fn default_true() -> bool {
    true
}

/// 本地 Cookie；请求发送时按域名、过期时间和 Secure 规则筛选。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiCookie {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default = "default_cookie_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

fn default_cookie_path() -> String {
    "/".to_string()
}

impl ApiCookie {
    /// 是否适用于当前请求地址。
    pub fn applies_to(&self, url: &reqwest::Url) -> bool {
        let host = url.host_str().unwrap_or_default();
        let domain = self.domain.as_deref().unwrap_or(host);
        cookie_domain_matches(host, domain)
            && cookie_path_matches(url.path(), &self.path)
            && (!self.secure || url.scheme() == "https")
            && self.expires_at.is_none_or(|expires| expires > unix_now())
    }
}

fn optional_domain_eq(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left
            .trim_start_matches('.')
            .eq_ignore_ascii_case(right.trim_start_matches('.')),
        _ => false,
    }
}

fn cookie_domain_matches(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = domain
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn cookie_path_matches(request_path: &str, cookie_path: &str) -> bool {
    let cookie_path = if cookie_path.trim().is_empty() {
        "/"
    } else {
        cookie_path.trim()
    };
    if cookie_path == "/" || request_path == cookie_path {
        return true;
    }
    request_path.starts_with(cookie_path)
        && request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
}

/// 将项目 Cookie 渲染为 `name=value` 文本；域名和属性由响应自动维护。
pub fn cookies_text(cookies: &[ApiCookie]) -> String {
    cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 `name=value` 文本替换 Cookie；域名缺省时按发送目标主机生效。
///
/// # Errors
/// 非空行没有等号或 Cookie 名称为空时返回中文错误。
pub fn parse_cookies_text(text: &str) -> Result<Vec<ApiCookie>, AppError> {
    let mut cookies = Vec::with_capacity(text.lines().count());
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("Cookie 第 {} 行缺少等号: {}", index + 1, trimmed),
            ));
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("Cookie 第 {} 行名称为空", index + 1),
            ));
        }
        cookies.push(ApiCookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: None,
            path: "/".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        });
    }
    Ok(cookies)
}

/// 一次请求在项目历史中的摘要；不保存完整响应体，避免项目文件无限膨胀。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiHistoryEntry {
    pub request_id: ApiRequestId,
    pub request_name: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub elapsed_millis: u128,
    pub at: u64,
    pub assertions_passed: usize,
    pub assertions_total: usize,
    #[serde(default)]
    pub response_excerpt: String,
}

fn normalize_name(name: String, fallback: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.chars().take(MAX_NAME_CHARS).collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_project_has_stable_display_and_empty_request_list() {
        let project = ApiProject::new("  Demo API  ");
        assert_eq!(project.name, "Demo API");
        assert_eq!(project.display_name(), "Demo API");
        assert_eq!(project.request_count(), 0);
        assert!(project.active_environment_id.is_some());
        assert!(!project.id.to_string().is_empty());
    }

    #[test]
    fn environment_variables_round_trip_and_resolve() {
        let mut environment = ApiEnvironment::new("测试");
        environment
            .set_variables_text("HOST=example.com\nTOKEN=secret")
            .expect("合法环境变量");
        assert_eq!(environment.variables.len(), 2);
        assert_eq!(
            environment.resolve("https://{{HOST}}/api"),
            "https://example.com/api"
        );
        assert_eq!(
            environment.variables_text(),
            "HOST=example.com\nTOKEN=secret"
        );
    }

    #[test]
    fn broken_environment_variable_reports_line() {
        let mut environment = ApiEnvironment::new("测试");
        let error = environment
            .set_variables_text("HOST=example.com\nBROKEN")
            .expect_err("缺少等号");
        assert!(error.detail.expect("错误详情").contains("第 2 行"));
    }

    #[test]
    fn cookie_domain_matches_subdomains_with_or_without_leading_dot() {
        let url = reqwest::Url::parse("https://api.example.com/users").expect("URL");
        let cookie = ApiCookie {
            name: "session".to_string(),
            value: "abc".to_string(),
            domain: Some("example.com".to_string()),
            path: "/".to_string(),
            secure: true,
            http_only: true,
            expires_at: None,
        };
        assert!(cookie.applies_to(&url));

        let cookie = ApiCookie {
            domain: Some(".example.com".to_string()),
            ..cookie
        };
        assert!(cookie.applies_to(&url));
    }

    #[test]
    fn cookie_path_requires_a_segment_boundary() {
        let cookie = ApiCookie {
            name: "session".to_string(),
            value: "abc".to_string(),
            domain: None,
            path: "/api".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        };
        assert!(
            cookie.applies_to(&reqwest::Url::parse("https://example.com/api/users").expect("URL"))
        );
        assert!(
            !cookie.applies_to(&reqwest::Url::parse("https://example.com/apiv2").expect("URL"))
        );
    }

    #[test]
    fn cookie_upsert_treats_domain_case_and_leading_dot_equivalently() {
        let mut project = ApiProject::new("Cookie 测试");
        project.upsert_cookie(ApiCookie {
            name: "session".to_string(),
            value: "old".to_string(),
            domain: Some("Example.com".to_string()),
            path: "/".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        });
        project.upsert_cookie(ApiCookie {
            name: "session".to_string(),
            value: "new".to_string(),
            domain: Some(".example.com".to_string()),
            path: "/".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        });
        assert_eq!(project.cookies.len(), 1);
        assert_eq!(project.cookies[0].value, "new");
    }
}
