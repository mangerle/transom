//! RDP 远程桌面 UI 集成模块。

pub mod canvas;
mod frame_cache;
pub mod input;
pub mod toolbar;
pub(crate) mod tunnel;
pub mod view;
mod view_render;

pub use view::{RdpView, RdpViewEvent};

#[cfg(test)]
mod tests;
