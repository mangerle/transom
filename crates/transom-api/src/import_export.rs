//! API 项目导入导出：Transom、cURL、Postman 2.1 与 OpenAPI 3。

use super::{ApiProject, parse_project_import};
use transom_core::AppError;

mod common;
mod curl;
mod export;
mod openapi;
mod postman;

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
        ApiImportFormat::Curl => curl::import_curl(text),
        ApiImportFormat::Postman => postman::import_postman(text),
        ApiImportFormat::OpenApi => openapi::import_openapi(text),
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
        ApiExportFormat::Curl => export::export_curl(project),
        ApiExportFormat::Postman => export::export_postman(project),
        ApiExportFormat::OpenApi => export::export_openapi(project),
    }
    .map_err(|error| {
        AppError::with_detail(
            transom_core::ErrorCode::Configuration,
            format!("导出 API 项目失败，格式: {format:?}，原因: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::super::{ApiBodyType, ApiFormFieldKind, ApiMethod};
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
      description: 返回服务健康状态
      responses:
        '200':
          description: OK
"#;
        let projects = import_projects(text, ApiImportFormat::OpenApi).expect("OpenAPI 应可导入");
        assert_eq!(projects[0].requests.len(), 1);
        assert_eq!(projects[0].requests[0].name, "健康检查");
        assert_eq!(projects[0].requests[0].description, "返回服务健康状态");
        assert!(projects[0].requests[0].body.is_empty());
        let exported =
            export_project(&projects[0], ApiExportFormat::OpenApi).expect("OpenAPI 应可导出");
        assert!(exported.contains("返回服务健康状态"));
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
