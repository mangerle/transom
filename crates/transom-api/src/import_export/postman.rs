//! Postman 2.1 集合导入。

use super::super::{
    ApiBodyType, ApiFolderId, ApiFormField, ApiFormFieldKind, ApiHeader, ApiProject, ApiRequest,
};
use super::common::{file_url, import_error, parse_method};
use serde_json::Value;
use transom_core::AppError;

pub(super) fn import_postman(text: &str) -> Result<Vec<ApiProject>, AppError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| import_error(format!("解析 Postman 文件失败，原因: {error}")))?;
    let name = value
        .pointer("/info/name")
        .and_then(Value::as_str)
        .unwrap_or("Postman 导入");
    let mut project = ApiProject::new(name);
    let items = value
        .get("item")
        .and_then(Value::as_array)
        .ok_or_else(|| import_error("Postman 文件缺少 item 数组".to_string()))?;
    collect_postman_items(items, &mut project, None, 0)?;
    Ok(vec![project])
}

fn collect_postman_items(
    items: &[Value],
    project: &mut ApiProject,
    folder_id: Option<ApiFolderId>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > 32 {
        return Err(import_error("Postman 目录嵌套超过上限".to_string()));
    }
    for item in items {
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("目录");
            let folder = project.add_folder(name);
            collect_postman_items(children, project, Some(folder.id), depth.saturating_add(1))?;
            continue;
        }
        let request = parse_postman_request(item, folder_id)?;
        project.requests.push(request);
    }
    Ok(())
}

fn parse_postman_request(
    item: &Value,
    folder_id: Option<ApiFolderId>,
) -> Result<ApiRequest, AppError> {
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Postman 请求");
    let request = item
        .get("request")
        .ok_or_else(|| import_error(format!("Postman 项缺少 request: {name}")))?;
    let mut result = ApiRequest::new(name, folder_id);
    result.method = parse_method(
        request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or(result.method.as_str()),
    );
    result.url = postman_url(request.get("url"));
    if let Some(headers) = request.get("header").and_then(Value::as_array) {
        for header in headers {
            let name = header
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            result.headers.push(ApiHeader {
                name: name.to_string(),
                value: header
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                description: String::new(),
                enabled: !header
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
    }
    if let Some(body) = request.get("body") {
        parse_postman_body(body, &mut result)?;
    }
    Ok(result)
}

fn parse_postman_body(body: &Value, request: &mut ApiRequest) -> Result<(), AppError> {
    let mode = body.get("mode").and_then(Value::as_str).unwrap_or("raw");
    match mode {
        "raw" => {
            request.body_type = match body
                .pointer("/options/raw/language")
                .and_then(Value::as_str)
            {
                Some("json") => ApiBodyType::Json,
                Some("xml") => ApiBodyType::Xml,
                _ => ApiBodyType::Text,
            };
            request.body = body
                .get("raw")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        "urlencoded" | "formdata" => {
            request.body_type = if mode == "formdata" {
                ApiBodyType::Multipart
            } else {
                ApiBodyType::FormUrlEncoded
            };
            if let Some(fields) = body.get(mode).and_then(Value::as_array) {
                for field in fields {
                    let Some(name) = field.get("key").and_then(Value::as_str) else {
                        continue;
                    };
                    let kind = if field.get("type").and_then(Value::as_str) == Some("file") {
                        ApiFormFieldKind::File
                    } else {
                        ApiFormFieldKind::Text
                    };
                    let value = if kind == ApiFormFieldKind::File {
                        postman_file_source(field)
                    } else {
                        field
                            .get("value")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    };
                    request.form_fields.push(ApiFormField {
                        name: name.to_string(),
                        value,
                        description: field
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        enabled: !field
                            .get("disabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        kind,
                    });
                }
            }
        }
        "file" => {
            request.body_type = ApiBodyType::Binary;
            request.body_file = body
                .pointer("/file/src")
                .and_then(Value::as_str)
                .map(file_url)
                .unwrap_or_default();
        }
        _ => {}
    }
    Ok(())
}

fn postman_file_source(field: &Value) -> String {
    field
        .get("src")
        .and_then(|src| {
            src.as_str().map(str::to_string).or_else(|| {
                src.as_array()
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .map(|path| file_url(&path))
        .unwrap_or_default()
}

fn postman_url(url: Option<&Value>) -> String {
    let Some(url) = url else {
        return String::new();
    };
    if let Some(raw) = url.as_str() {
        return raw.to_string();
    }
    url.get("raw")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
