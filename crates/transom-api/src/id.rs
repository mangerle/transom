use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! api_id {
    ($name:ident) => {
        /// API 领域稳定标识；随机 UUID 避免项目、目录与请求在不同项目间串号。
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// 生成新的稳定标识。
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

api_id!(ApiProjectId);
api_id!(ApiFolderId);
api_id!(ApiRequestId);
