//! API 项目导出格式转换。

use super::super::{ApiBodyType, ApiFormFieldKind, ApiProject, ApiRequest};
use serde_json::{Map, Value, json};

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

pub(super) fn export_curl(project: &ApiProject) -> Result<String, String> {
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

pub(super) fn export_postman(project: &ApiProject) -> Result<String, String> {
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

pub(super) fn export_openapi(project: &ApiProject) -> Result<String, String> {
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
                    "description": request.description,
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
