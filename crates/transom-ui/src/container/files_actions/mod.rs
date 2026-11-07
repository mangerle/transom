//! 容器文件页的浏览、传输与就地编辑动作。
//!
//! 按用户意图拆分为 browse / transfer / prompt / edit / mutate 五个子模块，
//! 方法仍归属 AppShell，跨子模块调用保持原有 pub(crate) 可见性。

mod browse;
mod edit;
mod mutate;
mod prompt;
mod transfer;
