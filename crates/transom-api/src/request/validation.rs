//! API 请求体配置的结构性校验。

use super::{ApiBodyType, ApiError, ApiFormFieldKind, ApiRequest};

impl ApiRequest {
    /// 校验当前请求体配置能否进入发送流程。
    ///
    /// 只检查结构与必填项，不在这里访问文件系统或网络，因此可以在 UI 事件和
    /// HTTP 服务入口重复调用而不会引入阻塞 I/O。
    ///
    /// # Errors
    /// `urlencoded` 含文件字段、`multipart` 文件字段未选择路径，或 Binary
    /// 请求体未选择文件时返回 [`ApiError::InvalidBody`]。
    pub fn validate(&self) -> Result<(), ApiError> {
        match self.body_type {
            ApiBodyType::FormUrlEncoded => {
                if self
                    .form_fields
                    .iter()
                    .any(|field| field.kind == ApiFormFieldKind::File)
                {
                    return Err(ApiError::InvalidBody {
                        reason: "x-www-form-urlencoded 请求体不能包含文件字段".to_string(),
                    });
                }
            }
            ApiBodyType::Multipart => {
                for field in self
                    .form_fields
                    .iter()
                    .filter(|field| field.enabled && field.kind == ApiFormFieldKind::File)
                {
                    if !has_file_path(&field.value) {
                        return Err(ApiError::InvalidBody {
                            reason: format!("multipart 文件字段 {} 未选择文件", field.name.trim()),
                        });
                    }
                }
            }
            ApiBodyType::Binary => {
                if !has_file_path(&self.body_file) {
                    return Err(ApiError::InvalidBody {
                        reason: "Binary 请求体未选择文件".to_string(),
                    });
                }
            }
            ApiBodyType::None
            | ApiBodyType::Json
            | ApiBodyType::Text
            | ApiBodyType::Xml
            | ApiBodyType::JavaScript
            | ApiBodyType::Html => {}
        }
        Ok(())
    }
}

fn has_file_path(value: &str) -> bool {
    let path = value.trim().strip_prefix("file://").unwrap_or(value.trim());
    !path.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApiFormField;

    #[test]
    fn urlencoded_rejects_file_fields() {
        let mut request = ApiRequest::new("测试", None);
        request.body_type = ApiBodyType::FormUrlEncoded;
        request.form_fields.push(ApiFormField {
            name: "file".to_string(),
            value: "file:///tmp/demo.txt".to_string(),
            description: String::new(),
            enabled: true,
            kind: ApiFormFieldKind::File,
        });
        assert!(matches!(
            request.validate(),
            Err(ApiError::InvalidBody { .. })
        ));
    }

    #[test]
    fn multipart_requires_selected_file_path() {
        let mut request = ApiRequest::new("测试", None);
        request.body_type = ApiBodyType::Multipart;
        request.form_fields.push(ApiFormField {
            name: "file".to_string(),
            value: "file://".to_string(),
            description: String::new(),
            enabled: true,
            kind: ApiFormFieldKind::File,
        });
        assert!(matches!(
            request.validate(),
            Err(ApiError::InvalidBody { .. })
        ));
    }

    #[test]
    fn binary_requires_file_path_and_accepts_selected_path() {
        let mut request = ApiRequest::new("测试", None);
        request.body_type = ApiBodyType::Binary;
        assert!(matches!(
            request.validate(),
            Err(ApiError::InvalidBody { .. })
        ));
        request.body_file = "file:///tmp/demo.bin".to_string();
        assert!(request.validate().is_ok());
    }
}
