//! Kitty 图形协议（APC `ESC _ G ... ESC \`）解码与图像存储子模块。

mod parser;
mod store;
pub mod types;

#[cfg(test)]
mod tests;

pub(crate) use parser::KittyParser;
pub(crate) use store::GraphicsStore;
pub use types::{GraphicsPlacement, KittyEvent, PlaceRequest, TerminalImage};
