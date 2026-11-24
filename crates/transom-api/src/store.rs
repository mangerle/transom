//! API 项目本地存储契约与自有 JSON 导入。

use super::{API_SCHEMA_VERSION, ApiProject, ApiRegistry};
use transom_core::AppError;

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

/// 解析 Transom 自有导入文件：单个项目或项目登记表。
///
/// # Errors
/// JSON 无法解析、字段类型不匹配或登记表版本过高时返回 `AppError`。
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
