//! 会话配置表单模块。
//!
//! 负责聚合表单输入状态、拓扑路线展示、连接参数校验以及构建 Session 实体与渲染弹窗。

mod appearance;
mod builder;
mod init;
mod proxy;
mod serial;
mod ssh;
mod telnet;
mod types;
mod view;

pub(crate) use init::text_input;
#[allow(unused_imports)]
pub(crate) use types::BuiltSession;
pub(crate) use types::{
    AuthChoice, ConnectionChoice, IdentityOption, JumpOption, SessionForm, SessionFormInit,
};
