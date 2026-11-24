//! API 项目导入导出：Transom、cURL、Postman 2.1 与 OpenAPI 3。

use super::{
    ApiBodyType, ApiFormField, ApiFormFieldKind, ApiHeader, ApiMethod, ApiProject, ApiRequest,
    parse_project_import,
};
use serde_json::{Map, Value, json};
use transom_core::AppError;

/// 导入格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiImportFormat {
    Transom,
    Curl,
    Postman,
    OpenApi,
}

/// 导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiExportFormat {
    Transom,
    Curl,
    Postman,
    OpenApi,
}

/// 按格式导入项目。
///
/// # Errors
/// 文件解析失败、格式缺少必要字段或 API 项目版本过高时返回中文错误。
pub fn import_projects(text: &str, format: ApiImportFormat) -> Result<Vec<ApiProject>, AppError> {
    match format {
        ApiImportFormat::Transom => parse_project_import(text),
        ApiImportFormat::Curl => import_curl(text),
        ApiImportFormat::Postman => import_postman(text),
        ApiImportFormat::OpenApi => import_openapi(text),
    }
}

/// 按格式导出项目。
///
/// # Errors
/// 序列化失败或格式不支持项目结构时返回中文错误。
pub fn export_project(project: &ApiProject, format: ApiExportFormat) -> Result<String, AppError> {
    match format {
        ApiExportFormat::Transom => {
            serde_json::to_string_pretty(project).map_err(|error| error.to_string())
        }
        ApiExportFormat::Curl => export_curl(project),
        ApiExportFormat::Postman => export_postman(project),
        ApiExportFormat::OpenApi => export_openapi(project),
    }
    .map_err(|error| {
        AppError::with_detail(
            transom_core::ErrorCode::Configuration,
            format!("导出 API 项目失败，格式: {format:?}，原因: {error}"),
        )
    })
}

fn import_curl(text: &str) -> Result<Vec<ApiProject>, AppError> {
    let tokens = shell_tokens(text)?;
    let mut request = ApiRequest::new("cURL 导入", None);
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "-X" | "--request" => {
                index = index.saturating_add(1);
                request.method =
                    parse_method(tokens.get(index).map(String::as_str).unwrap_or("GET"));
            }
            "--url" => {
                index = index.saturating_add(1);
                request.url = tokens.get(index).cloned().unwrap_or_default();
            }
            "-H" | "--header" => {
                index = index.saturating_add(1);
                if let Some(value) = tokens.get(index) {
                    push_curl_header(&mut request, value)?;
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                index = index.saturating_add(1);
                request.body = tokens.get(index).cloned().unwrap_or_default();
            }
            "--data-urlencode" => {
                index = index.saturating_add(1);
                if let Some(value) = tokens.get(index) {
                    request.body_type = ApiBodyType::FormUrlEncoded;
                    request.form_fields.push(parse_curl_form_value(value)?);
                }
            }
            "-F" | "--form" => {
                index = index.saturating_add(1);
                if let Some(value) = tokens.get(index) {
                    request.body_type = ApiBodyType::Multipart;
                    request.form_fields.push(parse_curl_form_value(value)?);
                }
            }
            "-T" | "--upload-file" => {
                index = index.saturating_add(1);
                if let Some(value) = tokens.get(index) {
                    request.body_type = ApiBodyType::Binary;
                    request.body_file = file_url(value);
                }
            }
            token if token.starts_with("http://") || token.starts_with("https://") => {
                request.url = token.to_string();
            }
            _ => {}
        }
        index = index.saturating_add(1);
    }
    if request.url.trim().is_empty() {
        return Err(import_error("cURL 命令缺少 URL".to_string()));
    }
    let mut project = ApiProject::new("cURL 导入");
    project.requests.push(request);
    Ok(vec![project])
}

fn push_curl_header(request: &mut ApiRequest, text: &str) -> Result<(), AppError> {
    let Some((name, value)) = text.split_once(':') else {
        return Err(import_error(format!("cURL 请求头缺少冒号: {text}")));
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(import_error(format!("cURL 请求头名称为空: {text}")));
    }
    request.headers.push(ApiHeader {
        name: name.to_string(),
        value: value.trim().to_string(),
        description: String::new(),
        enabled: true,
    });
    Ok(())
}

fn parse_curl_form_value(text: &str) -> Result<ApiFormField, AppError> {
    let Some((name, value)) = text.split_once('=') else {
        return Err(import_error(format!("cURL 表单字段缺少等号: {text}")));
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(import_error(format!("cURL 表单字段名称为空: {text}")));
    }
    let value = value.split(';').next().unwrap_or_default().trim();
    let (kind, value) = if let Some(path) = value.strip_prefix('@') {
        (ApiFormFieldKind::File, file_url(path))
    } else {
        (ApiFormFieldKind::Text, value.to_string())
    };
    Ok(ApiFormField {
        name: name.to_string(),
        value,
        description: String::new(),
        enabled: true,
        kind,
    })
}

fn file_url(path: &str) -> String {
    format!("file://{}", path.trim())
}

fn import_postman(text: &str) -> Result<Vec<ApiProject>, AppError> {
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
    folder_id: Option<super::ApiFolderId>,
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
    folder_id: Option<super::ApiFolderId>,
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
            result.headers.push(super::ApiHeader {
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

fn import_openapi(text: &str) -> Result<Vec<ApiProject>, AppError> {
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
                request.body = description.to_string();
            }
            project.requests.push(request);
        }
    }
    Ok(vec![project])
}

fn postman_body(request: &ApiRequest) -> Value {
    match request.body_type {
        ApiBodyType::FormUrlEncoded => json!({
            "mode": "urlencoded",
            "urlencoded": request.form_fields.iter().map(|field| json!({
                "key": field.name,
                "value": field.value,
                "description": field.description,
                "disabled": !field.enabled,
            })).collect::<Vec<_>>()
        }),
        ApiBodyType::Multipart => json!({
            "mode": "formdata",
            "formdata": request.form_fields.iter().map(|field| json!({
                "key": field.name,
                "type": if field.kind == ApiFormFieldKind::File { "file" } else { "text" },
                "src": if field.kind == ApiFormFieldKind::File { field.value.trim_start_matches("file://").to_string() } else { String::new() },
                "value": if field.kind == ApiFormFieldKind::Text { field.value.clone() } else { String::new() },
                "description": field.description,
                "disabled": !field.enabled,
            })).collect::<Vec<_>>()
        }),
        ApiBodyType::Binary => json!({
            "mode": "file",
            "file": {"src": request.body_file.trim_start_matches("file://")}
        }),
        ApiBodyType::Json => json!({
            "mode": "raw",
            "raw": request.body,
            "options": {"raw": {"language": "json"}}
        }),
        ApiBodyType::Xml => json!({
            "mode": "raw",
            "raw": request.body,
            "options": {"raw": {"language": "xml"}}
        }),
        ApiBodyType::Text | ApiBodyType::None => json!({"mode": "raw", "raw": request.body}),
    }
}

fn export_curl(project: &ApiProject) -> Result<String, String> {
    let mut lines = Vec::with_capacity(project.requests.len());
    for request in &project.requests {
        let method = request.method.as_str();
        let url = if request.url.starts_with('/') && !project.base_url.is_empty() {
            format!("{}{}", project.base_url.trim_end_matches('/'), request.url)
        } else {
            request.url.clone()
        };
        let mut line = format!("curl -X {method} '{}'", url.replace('\'', "'\\''"));
        for header in request.headers.iter().filter(|header| header.enabled) {
            line.push_str(&format!(
                " -H '{}: {}'",
                header.name.replace('\'', "'\\''"),
                header.value.replace('\'', "'\\''")
            ));
        }
        match request.body_type {
            ApiBodyType::None => {}
            ApiBodyType::FormUrlEncoded => {
                for field in request.form_fields.iter().filter(|field| field.enabled) {
                    line.push_str(&format!(
                        " --data-urlencode '{}={}'",
                        field.name.replace('\'', "'\\''"),
                        field.value.replace('\'', "'\\''")
                    ));
                }
            }
            ApiBodyType::Multipart => {
                for field in request.form_fields.iter().filter(|field| field.enabled) {
                    let value = match field.kind {
                        ApiFormFieldKind::File => {
                            format!("@{}", field.value.trim_start_matches("file://"))
                        }
                        ApiFormFieldKind::Text => field.value.clone(),
                    };
                    line.push_str(&format!(
                        " -F '{}={}'",
                        field.name.replace('\'', "'\\''"),
                        value.replace('\'', "'\\''")
                    ));
                }
            }
            ApiBodyType::Binary => {
                line.push_str(&format!(
                    " -T '{}'",
                    request
                        .body_file
                        .trim_start_matches("file://")
                        .replace('\'', "'\\''")
                ));
            }
            ApiBodyType::Json | ApiBodyType::Text | ApiBodyType::Xml => {
                if !request.body.is_empty() {
                    line.push_str(&format!(" -d '{}'", request.body.replace('\'', "'\\''")));
                }
            }
        }
        lines.push(line);
    }
    Ok(lines.join("\n"))
}

fn export_postman(project: &ApiProject) -> Result<String, String> {
    let items = project
        .requests
        .iter()
        .map(|request| {
            json!({
                "name": request.display_name(),
                "request": {
                    "method": request.method.as_str(),
                    "header": request.headers.iter().filter(|h| h.enabled).map(|h| json!({"key": h.name, "value": h.value})).collect::<Vec<_>>(),
                    "body": postman_body(request),
                    "url": {"raw": request.url}
                }
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({
        "info": {"name": project.display_name(), "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
        "item": items
    }))
    .map_err(|error| error.to_string())
}

fn export_openapi(project: &ApiProject) -> Result<String, String> {
    let mut paths = Map::with_capacity(project.requests.len());
    for request in &project.requests {
        let method = request.method.as_str().to_lowercase();
        let entry = paths
            .entry(request.url.clone())
            .or_insert_with(|| json!({}));
        if let Some(object) = entry.as_object_mut() {
            object.insert(
                method,
                json!({
                    "summary": request.display_name(),
                    "description": request.body,
                    "responses": {"200": {"description": "实际响应"}}
                }),
            );
        }
    }
    serde_json::to_string_pretty(&json!({
        "openapi": "3.0.3",
        "info": {"title": project.display_name(), "version": "1.0.0"},
        "servers": if project.base_url.is_empty() { Vec::<Value>::new() } else { vec![json!({"url": project.base_url})] },
        "paths": paths
    }))
    .map_err(|error| error.to_string())
}

fn parse_yaml_or_json(text: &str) -> Result<Value, AppError> {
    serde_json::from_str(text)
        .or_else(|_| serde_yaml_ng::from_str::<Value>(text))
        .map_err(|error| import_error(format!("解析 OpenAPI 文件失败，原因: {error}")))
}

fn shell_tokens(text: &str) -> Result<Vec<String>, AppError> {
    let mut tokens = Vec::with_capacity(16);
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in text.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && quote == Some('"') {
            escaped = true;
        } else if Some(character) == quote {
            quote = None;
        } else if quote.is_none() && (character == '\'' || character == '"') {
            quote = Some(character);
        } else if quote.is_none() && character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quote.is_some() {
        return Err(import_error("cURL 命令引号未闭合".to_string()));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn parse_method(value: &str) -> ApiMethod {
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

fn is_http_method(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
    )
}

fn import_error(message: String) -> AppError {
    AppError::with_detail(transom_core::ErrorCode::Configuration, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_import_and_export_round_trip() {
        let projects = import_projects(
            "curl -X POST 'https://example.com/users' -H 'Content-Type: application/json' -d '{\"name\":\"A\"}'",
            ApiImportFormat::Curl,
        )
        .expect("cURL 应可导入");
        let project = &projects[0];
        assert_eq!(project.requests[0].method, ApiMethod::Post);
        assert_eq!(project.requests[0].headers.len(), 1);
        assert!(
            export_project(project, ApiExportFormat::Curl)
                .expect("cURL 应可导出")
                .contains("curl")
        );
    }

    #[test]
    fn curl_import_keeps_multiple_headers() {
        let projects = import_projects(
            "curl 'https://example.com/users' -H 'Accept: application/json' -H 'X-Trace: demo'",
            ApiImportFormat::Curl,
        )
        .expect("cURL 应可导入");
        assert_eq!(projects[0].requests[0].headers.len(), 2);
        assert_eq!(projects[0].requests[0].headers[0].name, "Accept");
        assert_eq!(projects[0].requests[0].headers[1].name, "X-Trace");
    }

    #[test]
    fn openapi_import_reads_paths() {
        let text = r#"openapi: 3.0.3
info:
  title: 示例接口
  version: 1.0.0
servers:
  - url: https://api.example.com
paths:
  /health:
    get:
      summary: 健康检查
      responses:
        '200':
          description: OK
"#;
        let projects = import_projects(text, ApiImportFormat::OpenApi).expect("OpenAPI 应可导入");
        assert_eq!(projects[0].requests.len(), 1);
        assert_eq!(projects[0].requests[0].name, "健康检查");
    }

    #[test]
    fn curl_imports_multipart_file_and_exports_form() {
        let projects = import_projects(
            "curl -X POST 'https://example.com/upload' -F 'note=hello' -F 'file=@/tmp/demo.txt'",
            ApiImportFormat::Curl,
        )
        .expect("cURL multipart 应可导入");
        let request = &projects[0].requests[0];
        assert_eq!(request.body_type, ApiBodyType::Multipart);
        assert_eq!(request.form_fields.len(), 2);
        assert_eq!(request.form_fields[1].kind, ApiFormFieldKind::File);
        assert!(request.form_fields[1].value.ends_with("/tmp/demo.txt"));
        let exported =
            export_project(&projects[0], ApiExportFormat::Curl).expect("cURL multipart 应可导出");
        assert!(exported.contains("-F 'file=@/tmp/demo.txt'"), "{exported}");
    }

    #[test]
    fn postman_formdata_round_trips_file_fields() {
        let text = r#"{
  "info": {"name": "上传"},
  "item": [{
    "name": "上传文件",
    "request": {
      "method": "POST",
      "url": {"raw": "https://example.com/upload"},
      "body": {
        "mode": "formdata",
        "formdata": [
          {"key": "note", "type": "text", "value": "hello"},
          {"key": "file", "type": "file", "src": "/tmp/demo.txt"}
        ]
      }
    }
  }]
}"#;
        let projects =
            import_projects(text, ApiImportFormat::Postman).expect("Postman formdata 应可导入");
        let request = &projects[0].requests[0];
        assert_eq!(request.body_type, ApiBodyType::Multipart);
        assert_eq!(request.form_fields[1].kind, ApiFormFieldKind::File);
        let exported =
            export_project(&projects[0], ApiExportFormat::Postman).expect("Postman 应可导出");
        assert!(exported.contains("\"mode\": \"formdata\""));
        assert!(exported.contains("\"src\": \"/tmp/demo.txt\""));
    }

    #[test]
    fn postman_import_reads_nested_items() {
        let text = r#"{
  "info": {"name": "Postman 示例"},
  "item": [
    {
      "name": "用户",
      "item": [
        {
          "name": "用户列表",
          "request": {
            "method": "GET",
            "url": {"raw": "https://example.com/users"},
            "header": [{"key": "Accept", "value": "application/json"}]
          }
        }
      ]
    }
  ]
}"#;
        let projects = import_projects(text, ApiImportFormat::Postman).expect("Postman 应可导入");
        assert_eq!(projects[0].requests.len(), 1);
        assert_eq!(projects[0].requests[0].name, "用户列表");
        assert!(projects[0].requests[0].folder_id.is_some());
    }
}
