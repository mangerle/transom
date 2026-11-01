//! 应用设置的全局存储。
//!
//! # 设计原理（Why）
//! - 设置与进程同生命周期、与窗口无关，按"状态按生命周期归属"的规范属于 `Global`，
//!   不应挂在外壳字段上；放进 `Global` 后，面板实体与渲染路径都能直接读取，
//!   不需要外壳转手；
//! - 写入统一走 [`update_settings`]，便于在写后统一持久化；
//! - 读取一律经过 [`settings`]，调用点显式写出"这是全局配置"，避免隐式穿透状态。
//!
//! # 代价与局限
//! 读取需要 `cx`；不持有上下文的纯函数改为接收 `&AppSettings` 快照参数，这是刻意的：
//! 全局可变状态隐式可读会让"谁改了设置"难以追踪。
//!
//! `AppSettings` 定义在 `transom-core`，受孤儿规则限制不能直接为它实现 gpui 的
//! `Global`，因此用本模块的新类型包一层。

use gpui_kit::{App, BorrowAppContext, Global};
use transom_core::AppSettings;

/// 携带应用设置的全局容器。
pub(crate) struct SettingsGlobal(pub(crate) AppSettings);

impl Global for SettingsGlobal {}

/// 读取当前设置。
///
/// # Panics
/// 全局未安装时 panic；安装发生在 `AppShell::new` 引导阶段，早于任何读取。
pub(crate) fn app_settings(cx: &App) -> &AppSettings {
    &cx.global::<SettingsGlobal>().0
}

/// 就地修改设置并返回结果。
///
/// 只改内存，不负责落盘：调用方按需再走 `persist_settings_reported`。
pub(crate) fn update_app_settings<C, R>(cx: &mut C, action: impl FnOnce(&mut AppSettings) -> R) -> R
where
    C: BorrowAppContext,
{
    cx.update_global(|global: &mut SettingsGlobal, _| action(&mut global.0))
}

/// 借用可变的当前设置。
///
/// 用于就地修改方法调用（`settings_mut(cx).activity_bar.set_visible(...)`）与
/// 跨多行的赋值；单行赋值优先用 [`update_settings`]，可读性更好。
pub(crate) fn app_settings_mut(cx: &mut App) -> &mut AppSettings {
    &mut cx.global_mut::<SettingsGlobal>().0
}

/// 安装全局设置（启动引导与单元测试使用）。
pub(crate) fn install_app_settings<C: BorrowAppContext>(cx: &mut C, value: AppSettings) {
    cx.set_global(SettingsGlobal(value));
}
