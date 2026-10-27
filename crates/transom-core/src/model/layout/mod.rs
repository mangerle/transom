//! 可定制工具条的布局契约与活动栏/状态栏条目模型。

mod activity_bar;
mod status_bar;

#[cfg(test)]
mod tests;

pub use activity_bar::{ActivityBarLayout, ActivityItemId};
pub use status_bar::{StatusBarLayout, StatusItemId};
