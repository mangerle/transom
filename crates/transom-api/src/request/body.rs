//! HTTP 请求体、raw 文本语言与表单字段模型。

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// HTTP 请求体的编辑与发送类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiBodyType {
    /// 不发送请求体。
    None,
    /// JSON 文本，保留用户原始格式。
    Json,
    /// 普通文本。
    #[default]
    Text,
    /// XML 文本。
    Xml,
    /// JavaScript 文本。
    JavaScript,
    /// HTML 文本。
    Html,
    /// `application/x-www-form-urlencoded` 键值表单。
    FormUrlEncoded,
    /// `multipart/form-data` 键值表单，支持文件。
    Multipart,
    /// 单个二进制文件。
    Binary,
}

impl ApiBodyType {
    /// 稳定的下拉顺序。
    pub const ALL: [Self; 9] = [
        Self::None,
        Self::FormUrlEncoded,
        Self::Multipart,
        Self::Binary,
        Self::Json,
        Self::Text,
        Self::Xml,
        Self::JavaScript,
        Self::Html,
    ];

    /// 配置文本使用的稳定值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json => "json",
            Self::Text => "text",
            Self::Xml => "xml",
            Self::JavaScript => "javascript",
            Self::Html => "html",
            Self::FormUrlEncoded => "form_urlencoded",
            Self::Multipart => "multipart",
            Self::Binary => "binary",
        }
    }

    /// 是否使用表单字段列表发送。
    pub const fn is_form(self) -> bool {
        matches!(self, Self::FormUrlEncoded | Self::Multipart)
    }

    /// 是否为 raw 文本请求体。
    pub const fn is_raw(self) -> bool {
        self.raw_language().is_some()
    }

    /// raw 请求体对应的语言；表单、文件和空请求体返回 `None`。
    pub const fn raw_language(self) -> Option<ApiRawLanguage> {
        match self {
            Self::Json => Some(ApiRawLanguage::Json),
            Self::Text => Some(ApiRawLanguage::Text),
            Self::Xml => Some(ApiRawLanguage::Xml),
            Self::JavaScript => Some(ApiRawLanguage::JavaScript),
            Self::Html => Some(ApiRawLanguage::Html),
            Self::None | Self::FormUrlEncoded | Self::Multipart | Self::Binary => None,
        }
    }
}

/// raw 请求体的文本语言。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiRawLanguage {
    #[default]
    Json,
    Xml,
    JavaScript,
    Text,
    Html,
}

impl ApiRawLanguage {
    pub const ALL: [Self; 5] = [
        Self::Json,
        Self::Xml,
        Self::JavaScript,
        Self::Text,
        Self::Html,
    ];

    pub const fn body_type(self) -> ApiBodyType {
        match self {
            Self::Json => ApiBodyType::Json,
            Self::Xml => ApiBodyType::Xml,
            Self::JavaScript => ApiBodyType::JavaScript,
            Self::Text => ApiBodyType::Text,
            Self::Html => ApiBodyType::Html,
        }
    }
}

/// 表单字段的值类型；文件只允许用于 multipart。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormFieldKind {
    #[default]
    Text,
    File,
}

/// 一条表单字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiFormField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ApiFormFieldKind,
}

impl ApiFormField {
    /// 创建启用的文本表单字段。
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: String::new(),
            enabled: true,
            kind: ApiFormFieldKind::Text,
        }
    }
}
