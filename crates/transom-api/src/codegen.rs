//! API 请求代码生成。

use super::{ApiBodyType, ApiFormFieldKind, ApiRequest, ApiResponseSnapshot};

/// 代码生成目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeTarget {
    Curl,
    JavaScriptFetch,
    PythonRequests,
    RustReqwest,
}

impl CodeTarget {
    /// 下拉和预览标题使用的稳定名称。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Curl => "cURL",
            Self::JavaScriptFetch => "JavaScript fetch",
            Self::PythonRequests => "Python requests",
            Self::RustReqwest => "Rust reqwest",
        }
    }
}

/// 生成当前请求的可复制代码文本。
pub fn generate_code(target: CodeTarget, request: &ApiRequest) -> String {
    match target {
        CodeTarget::Curl => curl(request),
        CodeTarget::JavaScriptFetch => javascript_fetch(request),
        CodeTarget::PythonRequests => python_requests(request),
        CodeTarget::RustReqwest => rust_reqwest(request),
    }
}

fn curl(request: &ApiRequest) -> String {
    let mut line = format!(
        "curl -X {} '{}'",
        request.method.as_str(),
        shell_quote(&request.url)
    );
    for header in request.headers.iter().filter(|header| header.enabled) {
        line.push_str(&format!(
            " -H '{}: {}'",
            shell_quote(&header.name),
            shell_quote(&header.value)
        ));
    }
    match request.body_type {
        ApiBodyType::None => {}
        ApiBodyType::FormUrlEncoded => {
            for field in request.form_fields.iter().filter(|field| field.enabled) {
                line.push_str(&format!(
                    " --data-urlencode '{}={}'",
                    shell_quote(&field.name),
                    shell_quote(&field.value)
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
                    shell_quote(&field.name),
                    shell_quote(&value)
                ));
            }
        }
        ApiBodyType::Binary => {
            line.push_str(&format!(
                " --upload-file '{}'",
                shell_quote(request.body_file.trim_start_matches("file://"))
            ));
        }
        ApiBodyType::Json | ApiBodyType::Text | ApiBodyType::Xml => {
            if !request.body.is_empty() {
                line.push_str(&format!(" -d '{}'", shell_quote(&request.body)));
            }
        }
    }
    line
}

fn javascript_fetch(request: &ApiRequest) -> String {
    let headers = request
        .headers
        .iter()
        .filter(|header| header.enabled)
        .map(|header| format!("  {:?}: {:?},\n", header.name, header.value))
        .collect::<String>();
    let body = if request.body.is_empty() {
        "".to_string()
    } else {
        format!("  body: {},\n", rust_debug_string(&request.body))
    };
    format!(
        "const response = await fetch({url:?}, {{\n  method: {method:?},\n  headers: {{\n{headers}  }},\n{body}}});\nconst data = await response.text();",
        url = request.url,
        method = request.method.as_str(),
        headers = headers,
        body = body
    )
}

fn python_requests(request: &ApiRequest) -> String {
    let headers = request
        .headers
        .iter()
        .filter(|header| header.enabled)
        .map(|header| format!("    {:?}: {:?},\n", header.name, header.value))
        .collect::<String>();
    let body = if request.body.is_empty() {
        String::new()
    } else {
        format!(", data={:?}", request.body)
    };
    format!(
        "import requests\n\nresponse = requests.request(\n    {method:?},\n    {url:?},\n    headers={{\n{headers}    }}{body}\n)\nprint(response.status_code)\nprint(response.text)",
        method = request.method.as_str(),
        url = request.url,
        headers = headers,
        body = body
    )
}

fn rust_reqwest(request: &ApiRequest) -> String {
    let mut builder = format!(
        "reqwest::Client::new()\n    .request(reqwest::Method::{}, {:?})",
        request.method.as_str(),
        request.url
    );
    for header in request.headers.iter().filter(|header| header.enabled) {
        builder.push_str(&format!(
            "\n    .header({:?}, {:?})",
            header.name, header.value
        ));
    }
    if !request.body.is_empty() {
        builder.push_str(&format!("\n    .body({:?})", request.body));
    }
    format!("{builder}\n    .send()\n    .await?\n    .text()\n    .await?;")
}

fn shell_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn rust_debug_string(value: &str) -> String {
    format!("{:?}", value)
}

/// 响应是否适合在代码生成或历史摘要中直接展示。
pub fn response_excerpt(response: &ApiResponseSnapshot, limit: usize) -> String {
    let mut excerpt = String::with_capacity(limit.min(response.body.len()));
    excerpt.extend(response.body.chars().take(limit));
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApiMethod;

    #[test]
    fn curl_codegen_includes_multipart_file() {
        let mut request = ApiRequest::new("上传", None);
        request.method = ApiMethod::Post;
        request.url = "https://example.com/upload".to_string();
        request.body_type = ApiBodyType::Multipart;
        request.form_fields = vec![crate::ApiFormField {
            name: "file".to_string(),
            value: "file:///tmp/demo.txt".to_string(),
            description: String::new(),
            enabled: true,
            kind: ApiFormFieldKind::File,
        }];
        let code = generate_code(CodeTarget::Curl, &request);
        assert!(code.contains("-F 'file=@/tmp/demo.txt'"), "{code}");
    }

    #[test]
    fn generates_supported_targets() {
        let mut request = ApiRequest::new("测试", None);
        request.method = ApiMethod::Post;
        request.url = "https://api.example.com/users".to_string();
        request.body = "{\"name\":\"A\"}".to_string();
        for target in [
            CodeTarget::Curl,
            CodeTarget::JavaScriptFetch,
            CodeTarget::PythonRequests,
            CodeTarget::RustReqwest,
        ] {
            assert!(!generate_code(target, &request).is_empty());
        }
    }
}
