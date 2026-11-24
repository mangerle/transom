//! API 导入导出共用的轻量解析函数。

use super::super::ApiMethod;
use transom_core::AppError;

pub(super) fn file_url(path: &str) -> String {
    format!("file://{}", path.trim())
}

pub(super) fn parse_method(value: &str) -> ApiMethod {
    match value.trim().to_ascii_uppercase().as_str() {
        "POST" => ApiMethod::Post,
        "PUT" => ApiMethod::Put,
        "PATCH" => ApiMethod::Patch,
        "DELETE" => ApiMethod::Delete,
        "HEAD" => ApiMethod::Head,
        "OPTIONS" => ApiMethod::Options,
        _ => ApiMethod::Get,
    }
}

pub(super) fn import_error(message: String) -> AppError {
    AppError::with_detail(transom_core::ErrorCode::Configuration, message)
}
