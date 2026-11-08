//! 会话新建/编辑表单实体：打开状态、输入控件与表单动作都归它。
//!
//! # 设计原理（Why）
//! 表单此前由 `ModalHub.session.form: Option<SessionForm>` 加散落的
//! `impl AppShell` 方法维护：打开、保存、删除与渲染分居 `workspace`、`sidebar`
//! 与 `session_form` 多处，读者要同时翻几个文件才能回答“谁拥有这次打开”。
//! 实体化后，一次打开期的输入控件与表单状态同生共死，外壳只持有句柄；
//! 表单内的状态变更也只重绘本实体，不再牵动整个外壳。
//!
//! 目录、钥匙串与串口枚举仍归外壳：实体不反查 `AppShell`，需要时通过
//! [`SessionFormEvent`] 上交意图，外壳处理后以 `view.update` 注入数据。
//!
//! # 代价与局限
//! 事件只携带结果，外壳不能回读实体补数据；保存成功即关闭，失败则保留
//! 弹窗并由实体自己写错误文案，两条路径必须各自保持一致。

use super::types::{BuiltSession, SessionForm, SessionFormInit};
use crate::palette::localized_error;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use transom_core::{AppError, FolderInheritedDefaults, SerialPortDescriptor};

/// 会话新建/编辑表单实体。
pub(crate) struct SessionFormView {
    /// 未打开时为 `None`；每次打开都重建输入控件，避免残留上次编辑内容。
    state: Option<SessionForm>,
}

impl SessionFormView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new() -> Self {
        Self { state: None }
    }

    /// 表单是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn state(&self) -> Option<&SessionForm> {
        self.state.as_ref()
    }

    /// 当前打开期的可变状态；仅供本模块内的动作与迁移期外壳更新使用。
    pub(crate) fn form_mut(&mut self) -> Option<&mut SessionForm> {
        self.state.as_mut()
    }

    /// 打开表单并安装文件夹输入联动。
    pub(crate) fn open(
        &mut self,
        init: SessionFormInit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = SessionForm::new(window, cx, init);
        self.install(form, cx);
    }

    /// 关闭表单并丢弃本次打开期的全部输入控件。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.state = None;
        cx.notify();
    }

    /// 表单全部文本项的当前值，转发给弹窗脏判定。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        self.state
            .as_ref()
            .map(|form| form.text_values(cx))
            .unwrap_or_default()
    }

    /// 校验并取出构建结果；成功即关闭表单，失败把错误写回弹窗并保持打开。
    pub(crate) fn take_built(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(BuiltSession, bool, bool)> {
        let (built, save_password, save_passphrase) = {
            let form = self.state.as_mut()?;
            let save_password = form.save_password;
            let save_passphrase = form.save_passphrase;
            match form.build_session(cx) {
                Ok(built) => (built, save_password, save_passphrase),
                Err(error) => {
                    form.error = Some(error);
                    cx.notify();
                    return None;
                }
            }
        };
        self.state = None;
        cx.notify();
        Some((built, save_password, save_passphrase))
    }

    /// 在表单内回填文件夹路径并应用分组默认值。
    pub(crate) fn apply_folder_selection(
        &mut self,
        path: &str,
        defaults: &FolderInheritedDefaults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.state.as_mut() {
            form.apply_folder_selection(path, defaults, window, cx);
        }
        cx.notify();
    }

    /// 清空文件夹路径（选择根层级）。
    pub(crate) fn clear_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.state.as_ref() {
            form.folder_path
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    /// 选中一个串口设备并写回输入框。
    pub(crate) fn select_serial_port(
        &mut self,
        port_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.state.as_ref() {
            form.serial_port
                .update(cx, |state, cx| state.set_value(port_name, window, cx));
        }
        cx.notify();
    }

    /// 写回串口枚举结果；设备路径为空时自动选中第一个可用端口。
    ///
    /// 枚举失败时保留手动输入能力，仅记录错误提示。
    pub(crate) fn apply_serial_ports(
        &mut self,
        result: Result<Vec<SerialPortDescriptor>, AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.state.as_mut() else {
            return;
        };
        match result {
            Ok(ports) => {
                form.serial_ports_error = None;
                form.serial_ports = ports;
                let current = form.serial_port.read(cx).value().trim().to_string();
                if current.is_empty()
                    && let Some(first) = form.serial_ports.first()
                {
                    let port_name = first.port_name.clone();
                    form.serial_port.update(cx, |state, cx| {
                        state.set_value(&port_name, window, cx);
                    });
                }
            }
            Err(error) => form.serial_ports_error = Some(localized_error(&error)),
        }
        cx.notify();
    }

    /// 装载表单并安装文件夹输入与候选弹层的关系。
    ///
    /// 文件夹字段是"输入或选择"：候选不随输入过滤。展开后一旦开始键入
    /// 路径，就把候选收起，避免一个不动的列表挡住表单；选中与清空仍由
    /// 字段自身收起。
    fn install(&mut self, form: SessionForm, cx: &mut Context<Self>) {
        cx.subscribe(&form.folder_path, |this, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let Some(form) = this.state.as_mut() else {
                return;
            };
            if form.folder_open {
                form.folder_open = false;
                cx.notify();
            }
        })
        .detach();
        self.state = Some(form);
        cx.notify();
    }
}
