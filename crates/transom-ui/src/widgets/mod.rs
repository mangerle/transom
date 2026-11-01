//! 应用级表现层组件。
//!
//! # 设计原理（Why）
//! - 组件只接收数据与回调，不依赖 `AppShell`、不直接读写 store；渲染函数可以
//!   在任意视图、测试与弹窗中复用；
//! - 视觉语义集中在这里：按钮尺寸、危险确认、设置行、空状态与菜单项的默认
//!   样式只有一份，页面不再各自拼一套；
//! - 底层优先复用 `gpui-kit` 的 `Button`、`Dialog`、`Empty` 等组件，应用层只做
//!   组合与业务语义，不重写焦点、键盘与弹层行为。
//!
//! # 代价与局限
//! - 组件为了避免依赖 `AppShell`，回调统一接收 GPUI 的 `&ClickEvent` /
//!   `&mut Window` / `&mut App`，调用方需要自己完成 `app.update` 或
//!   `dialog_host::on_event` 适配；
//! - 这里只收纳稳定、跨页面复用的结构；仅一处使用的布局仍留在页面内，避免
//!   为了“组件化”制造一次性抽象。

mod button;
mod controls;
mod dialog;
mod empty;
mod form;
mod list_row;
mod menu;
mod panel_header;
mod status;
mod toolbar;

pub(crate) use button::{
    DialogConfirmKind, IconButton, IconButtonSize, dialog_cancel_button, dialog_confirm_button,
};
pub(crate) use controls::{dropdown, number_stepper, path_value};
pub(crate) use dialog::{dialog_actions, dialog_body};
pub(crate) use empty::empty_state;
pub(crate) use form::{
    form_field, secret_field, setting_field_row, setting_frame, setting_row, setting_text_row,
};
pub(crate) use list_row::{list_row, table_header};
pub(crate) use menu::{MenuItem, menu_item};
pub(crate) use panel_header::PanelHeader;
pub(crate) use status::{status_badge, status_dot};
pub(crate) use toolbar::toolbar;
