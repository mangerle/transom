//! OpenAPI 3 JSON/YAML 文档导入。

use super::super::{ApiProject, ApiRequest};
use super::common::{import_error, parse_method};
use serde_json::Value;
use transom_core::AppError;

pub(super) fn import_openapi(text: &str) -> Result<Vec<ApiProject>, AppError> {
    let value = parse_yaml_or_json(text)?;
    let name = value
        .pointer("/info/title")
        .and_then(Value::as_str)
        .unwrap_or("OpenAPI 导入");
    let base_url = value
        .pointer("/servers/0/url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let paths = value
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| import_error("OpenAPI 文件缺少 paths 对象".to_string()))?;
    let mut project = ApiProject::new(name);
    project.base_url = base_url.to_string();
    for (path, path_item) in paths {
        let Some(operations) = path_item.as_object() else {
            continue;
        };
        for (method, operation) in operations {
            if !is_http_method(method) {
                continue;
            }
            let request_name = operation
                .get("summary")
                .and_then(Value::as_str)
                .or_else(|| operation.get("operationId").and_then(Value::as_str))
                .unwrap_or(path);
            let mut request = ApiRequest::new(request_name, None);
            request.method = parse_method(method);
            request.url = path.to_string();
            if let Some(description) = operation.get("description").and_then(Value::as_str) {
                request.description = description.to_string();
            }
            project.requests.push(request);
        }
    }
    Ok(vec![project])
}

fn parse_yaml_or_json(text: &str) -> Result<Value, AppError> {
    serde_json::from_str(text)
        .or_else(|_| serde_yaml_ng::from_str::<Value>(text))
        .map_err(|error| import_error(format!("解析 OpenAPI 文件失败，原因: {error}")))
}

fn is_http_method(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
    )
}
