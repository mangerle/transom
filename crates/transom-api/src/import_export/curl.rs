//! cURL 命令导入。

use super::super::{
    ApiBodyType, ApiFormField, ApiFormFieldKind, ApiHeader, ApiProject, ApiRequest,
};
use super::common::{file_url, import_error, parse_method};
use transom_core::AppError;

pub(super) fn import_curl(text: &str) -> Result<Vec<ApiProject>, AppError> {
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
