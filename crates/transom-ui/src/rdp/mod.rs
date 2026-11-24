//! RDP 远程桌面 UI 集成模块。

pub mod canvas;
mod frame_cache;
pub mod input;
pub mod toolbar;
pub mod view;

pub use view::RdpView;

#[cfg(test)]
mod tests;
