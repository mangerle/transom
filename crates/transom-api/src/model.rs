//! API 项目、接口目录、请求和本地存储契约。

use super::{ApiFolderId, ApiProjectId, ApiRequestId};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use transom_core::AppError;

/// 当前项目文件结构版本；版本高于程序支持值时拒绝加载，避免静默丢失新字段。
pub const API_SCHEMA_VERSION: u32 = 1;
/// 单次项目导入文件上限，防止误选超大文件后在 UI 线程前产生无界内存增长。
pub const MAX_IMPORT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 120;

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
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl ApiProject {
    /// 创建空项目；首次请求由工作区内的“新建请求”动作生成。
    pub fn new(name: impl Into<String>) -> Self {
        let now = unix_now();
        Self {
            id: ApiProjectId::new(),
            name: normalize_name(name.into(), "未命名项目"),
            base_url: String::new(),
            folders: Vec::new(),
            requests: Vec::new(),
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

/// HTTP 方法；与 CyberAPI 保持相同的常用集合，但独立定义，避免跨项目类型耦合。
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
}

/// 一条请求头；`enabled` 保留编辑器的启用/禁用能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiHeader {
    pub name: String,
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 一条可保存、可发送的 HTTP 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiRequest {
    #[serde(default)]
    pub id: ApiRequestId,
    #[serde(default)]
    pub folder_id: Option<ApiFolderId>,
    pub name: String,
    #[serde(default)]
    pub method: ApiMethod,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<ApiHeader>,
    #[serde(default)]
    pub body: String,
}

impl ApiRequest {
    /// 新建请求；URL 留空，由编辑器填写。
    pub fn new(name: impl Into<String>, folder_id: Option<ApiFolderId>) -> Self {
        Self {
            id: ApiRequestId::new(),
            folder_id,
            name: normalize_name(name.into(), "未命名请求"),
            method: ApiMethod::Get,
            url: String::new(),
            headers: Vec::new(),
            body: String::new(),
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

    /// 发送时将 `/` 开头的相对地址接到项目基础地址后；绝对 URL 原样返回。
    pub fn resolved_url(&self, base_url: &str) -> String {
        let url = self.url.trim();
        let base = base_url.trim().trim_end_matches('/');
        if url.starts_with('/') && !base.is_empty() {
            format!("{base}{url}")
        } else {
            url.to_string()
        }
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
    pub fn set_header_text(&mut self, text: &str) -> Result<(), super::ApiError> {
        self.headers = parse_headers(text)?;
        Ok(())
    }
}

/// 本地项目存储契约；UI 只依赖契约，JSON 实现位于 `transom-store`。
pub trait ApiProjectStore: Send + Sync {
    /// 加载全部项目；文件不存在时返回空登记表。
    ///
    /// # Errors
    /// 文件损坏、版本过高或 I/O 失败时返回 `AppError`。
    fn load(&self) -> Result<ApiRegistry, AppError>;

    /// 原子保存全部项目。
    ///
    /// # Errors
    /// 序列化、创建目录或原子替换失败时返回 `AppError`。
    fn save(&self, registry: &ApiRegistry) -> Result<(), AppError>;
}

/// 导入文件解析结果：兼容单个项目与应用自己的项目登记表。
pub fn parse_project_import(input: &str) -> Result<Vec<ApiProject>, AppError> {
    let value: serde_json::Value = serde_json::from_str(input).map_err(|error| {
        AppError::with_detail(
            transom_core::ErrorCode::Configuration,
            format!("解析 API 项目导入文件失败，原因: {error}"),
        )
    })?;
    if value.get("projects").is_some() {
        let registry: ApiRegistry = serde_json::from_value(value).map_err(|error| {
            AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("解析 API 项目登记表失败，原因: {error}"),
            )
        })?;
        if registry.version > API_SCHEMA_VERSION {
            return Err(AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!(
                    "API 项目文件版本 {} 高于当前支持版本 {}，请升级 Transom",
                    registry.version, API_SCHEMA_VERSION
                ),
            ));
        }
        Ok(registry.projects)
    } else {
        let project: ApiProject = serde_json::from_value(value).map_err(|error| {
            AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("解析单个 API 项目失败，原因: {error}"),
            )
        })?;
        Ok(vec![project])
    }
}

fn parse_headers(text: &str) -> Result<Vec<ApiHeader>, super::ApiError> {
    let mut headers = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            return Err(super::ApiError::InvalidHeader {
                line: index.saturating_add(1),
                reason: "缺少冒号分隔符".to_string(),
            });
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(super::ApiError::InvalidHeader {
                line: index.saturating_add(1),
                reason: "请求头名称为空".to_string(),
            });
        }
        headers.push(ApiHeader {
            name: name.to_string(),
            value: value.trim().to_string(),
            enabled: true,
        });
    }
    Ok(headers)
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
        assert!(!project.id.to_string().is_empty());
    }

    #[test]
    fn header_text_round_trips_and_reports_bad_lines() {
        let mut request = ApiRequest::new("创建用户", None);
        request
            .set_header_text("Content-Type: application/json\nX-Trace: demo")
            .expect("合法请求头");
        assert_eq!(request.headers.len(), 2);
        assert_eq!(
            request.header_text(),
            "Content-Type: application/json\nX-Trace: demo"
        );
        assert!(request.set_header_text("broken header").is_err());
    }

    #[test]
    fn relative_url_uses_project_base_url() {
        let mut request = ApiRequest::new("健康检查", None);
        request.url = "/health".to_string();
        assert_eq!(
            request.resolved_url("https://api.example.com/"),
            "https://api.example.com/health"
        );
        request.url = "https://other.example.com/ping".to_string();
        assert_eq!(
            request.resolved_url("https://api.example.com"),
            "https://other.example.com/ping"
        );
    }

    #[test]
    fn import_accepts_single_project_and_registry() {
        let project = ApiProject::new("导入项目");
        let registry = ApiRegistry {
            version: API_SCHEMA_VERSION,
            projects: vec![project.clone()],
        };
        let single = parse_project_import(&serde_json::to_string(&project).expect("序列化"))
            .expect("单个项目");
        let many = parse_project_import(&serde_json::to_string(&registry).expect("序列化"))
            .expect("登记表");
        assert_eq!(single, vec![project]);
        assert_eq!(many.len(), 1);
    }
}
