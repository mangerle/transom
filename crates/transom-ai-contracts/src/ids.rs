//! AI 运行、对话、工具调用与审批请求的强类型标识。

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 创建标识实例；空值仍由调用方在边界处校验。
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// 返回内部字符串借用。
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// 转换为拥有型字符串。
            pub fn into_string(self) -> String {
                self.0
            }

            /// 是否为空标识。
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(RunId);
string_id!(ConversationId);
string_id!(ToolCallId);
string_id!(ApprovalRequestId);
