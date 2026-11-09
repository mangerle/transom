//! 镜像构建弹窗：上下文/参数表单与构建进度展示。
//!
//! 动作与校验放在 actions.rs，渲染放在 render.rs。

mod actions;
mod render;
#[cfg(test)]
mod tests;

pub(crate) use render::render_container_build_dialog_body;
