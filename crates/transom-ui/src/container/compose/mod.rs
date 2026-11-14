//! Compose 项目管理动作：运行/创建/导入/编辑与 YAML 加载保存。
//!
//! 动作入口在 actions.rs，YAML 定位与读写集中在 yaml.rs。

mod actions;
pub(crate) use actions::{
    create_local_compose_with_content, edit_compose_yaml, import_local_compose,
};
mod yaml;
