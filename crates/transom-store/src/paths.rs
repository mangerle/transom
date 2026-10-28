//! 路径约定已下沉至 `transom-core::paths`（跨 crate 契约归 core），此处
//! re-export 保持既有导入路径（`transom_store::paths`）不变。
pub use transom_platform::paths::{
    config_dir, history_path, known_hosts_path, sessions_path, settings_path,
};
