//! API 项目、目录、环境、Cookie、历史和请求数据。

use super::{ApiEnvironmentId, ApiFolderId, ApiProjectId, ApiRequestId};
use super::{ApiHeader, ApiQueryParam, ApiRequest};
use serde::{Deserialize, Serialize};

mod cookie;
mod environment;
mod global;
mod util;

use self::cookie::optional_domain_eq;
pub use self::cookie::{ApiCookie, cookies_text, parse_cookies_text};
pub(crate) use self::cookie::{
    cookie_domain_matches, cookie_header_value, is_valid_cookie_name, is_valid_cookie_value,
};
pub use self::environment::{ApiEnvironment, ApiEnvironmentResolver};
pub use self::global::{ApiGlobalParameter, ApiGlobalParameterTarget};
use self::util::{normalize_name, unix_now};

/// 当前项目文件结构版本；版本高于程序支持值时拒绝加载，避免静默丢失新字段。
pub const API_SCHEMA_VERSION: u32 = 3;
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
    pub global_variables: Vec<ApiVariable>,
    #[serde(default)]
    pub global_parameters: Vec<ApiGlobalParameter>,
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
            global_variables: Vec::new(),
            global_parameters: Vec::new(),
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
    fn environment_variables_override_global_variables() {
        let mut project = ApiProject::new("变量优先级");
        project.global_variables = vec![
            ApiVariable {
                name: "HOST".to_string(),
                value: "global.example.com".to_string(),
                enabled: true,
                secret: false,
            },
            ApiVariable {
                name: "GLOBAL_ONLY".to_string(),
                value: "global".to_string(),
                enabled: true,
                secret: false,
            },
        ];
        let mut environment = ApiEnvironment::new("开发");
        environment.variables.push(ApiVariable {
            name: "HOST".to_string(),
            value: "env.example.com".to_string(),
            enabled: true,
            secret: false,
        });
        let merged = project.merged_environment(&environment);
        assert_eq!(merged.resolve("{{HOST}}"), "env.example.com");
        assert_eq!(merged.resolve("{{GLOBAL_ONLY}}"), "global");
    }

    #[test]
    fn execution_plan_resolves_request_once_and_captures_cookie_snapshot() {
        let mut project = ApiProject::new("执行计划");
        project.cookies.push(ApiCookie {
            name: "session".to_string(),
            value: "abc".to_string(),
            domain: None,
            path: "/".to_string(),
            secure: false,
            http_only: false,
            expires_at: None,
        });
        let mut request = ApiRequest::new("单次解析", None);
        request.url = "https://example.com/{{A}}".to_string();
        request.body = "{{A}}".to_string();
        let mut environment = ApiEnvironment::empty();
        environment.variables = vec![
            ApiVariable {
                name: "A".to_string(),
                value: "{{B}}".to_string(),
                enabled: true,
                secret: false,
            },
            ApiVariable {
                name: "B".to_string(),
                value: "expanded".to_string(),
                enabled: true,
                secret: false,
            },
        ];
        let plan = project
            .execution_plan(&request, &environment)
            .expect("执行计划应合法");
        assert!(
            plan.request().url.contains("%7B%7BB%7D%7D") || plan.request().url.ends_with("{{B}}")
        );
        assert_eq!(plan.request().body, "{{B}}");
        assert_eq!(plan.cookies().len(), 1);
    }

    #[test]
    fn request_values_override_global_query_and_header_parameters() {
        let mut project = ApiProject::new("全局参数");
        project.global_parameters = vec![
            ApiGlobalParameter {
                target: ApiGlobalParameterTarget::Query,
                name: "token".to_string(),
                value: "global".to_string(),
                description: String::new(),
                enabled: true,
            },
            ApiGlobalParameter {
                target: ApiGlobalParameterTarget::Query,
                name: "global_only".to_string(),
                value: "1".to_string(),
                description: String::new(),
                enabled: true,
            },
            ApiGlobalParameter {
                target: ApiGlobalParameterTarget::Header,
                name: "X-Scope".to_string(),
                value: "global".to_string(),
                description: String::new(),
                enabled: true,
            },
        ];
        let mut request = ApiRequest::new("全局参数", None);
        request.url = "https://example.com".to_string();
        request.query.push(ApiQueryParam::new("token", "request"));
        request.headers.push(ApiHeader::new("x-scope", "request"));
        let resolved = project.resolved_request(&request, &ApiEnvironment::empty());
        assert_eq!(
            resolved
                .query
                .iter()
                .find(|parameter| parameter.name == "token")
                .map(|parameter| parameter.value.as_str()),
            Some("request")
        );
        assert!(
            resolved
                .query
                .iter()
                .any(|parameter| parameter.name == "global_only")
        );
        assert!(resolved.url.contains("global_only=1"));
        assert_eq!(
            resolved
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case("x-scope"))
                .map(|header| header.value.as_str()),
            Some("request")
        );
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
