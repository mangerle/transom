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

use super::init::text_input;
use super::types::{
    AuthChoice, BuiltSession, ConnectionChoice, IdentityOption, SessionForm, SessionFormInit,
    SessionFormSection,
};
use crate::identity::{BuiltIdentity, IdentityEditState, build_identity_from_edit};
use crate::palette::localized_error;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use transom_core::{
    AppError, AuthMethod, FolderInheritedDefaults, IdentityId, SecretKind, SerialPortDescriptor,
    SessionId,
};

/// 会话表单交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
pub(crate) enum SessionFormEvent {
    /// 校验通过后的构建结果；实体已先关闭再发出本事件。
    Save {
        /// 构建结果体积远大于删除事件，装箱避免枚举整体变大。
        built: Box<BuiltSession>,
        save_password: bool,
        save_passphrase: bool,
    },
    /// 删除当前编辑的会话；实体已先关闭再发出本事件。
    Delete { session_id: SessionId },
    /// 文件夹候选被选中；目录默认值由外壳解析后注入。
    SelectFolder { path: String },
    /// 重新枚举系统串口；数据由外壳注入，实体不直接访问系统。
    RefreshSerialPorts,
    /// 内联新建或编辑账户（凭据模板）完成；由外壳写入目录与钥匙串。
    SaveIdentity { built: Box<BuiltIdentity> },
    /// 打开内联账户编辑时请求外壳后台回填已保存的凭据口令。
    PrefillIdentitySecrets {
        id: IdentityId,
        fields: Vec<(SecretKind, Entity<InputState>)>,
    },
}

impl EventEmitter<SessionFormEvent> for SessionFormView {}

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

    /// 在表单内就地修改字段并重绘；关闭状态下忽略。
    pub(super) fn edit(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut SessionForm)) {
        if let Some(form) = self.state.as_mut() {
            update(form);
            cx.notify();
        }
    }

    /// 供同模块子视图读写表单状态（转发规则编辑需要同时访问窗口与上下文）。
    pub(super) fn state_mut(&mut self) -> Option<&mut SessionForm> {
        self.state.as_mut()
    }

    /// 回写文件夹候选弹层的展开状态。
    pub(crate) fn set_folder_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.folder_open = open);
    }

    /// 回写图标与颜色选择弹层的展开状态。
    pub(crate) fn set_icon_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.icon_open = open);
    }

    /// 切换左侧配置分组。
    pub(crate) fn set_section(&mut self, section: SessionFormSection, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.section = section);
    }

    /// 打开内联账户（凭据模板）编辑器。
    pub(crate) fn open_identity_editor(
        &mut self,
        id: Option<IdentityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.state.as_mut() else {
            return;
        };
        let existing = id.and_then(|target| {
            form.identities
                .iter()
                .find(|option| option.id == target)
                .cloned()
        });
        let edit = new_identity_edit_state(existing.as_ref(), window, cx);
        let edit_id = edit.base;
        let fields = vec![
            (SecretKind::Password, edit.password.clone()),
            (SecretKind::KeyPassphrase, edit.passphrase.clone()),
        ];
        form.identity_edit = Some(edit);
        if let Some(id) = edit_id {
            cx.emit(SessionFormEvent::PrefillIdentitySecrets { id, fields });
        }
        cx.notify();
    }

    /// 取消内联账户编辑器。
    pub(crate) fn cancel_identity_editor(&mut self, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.identity_edit = None);
    }

    /// 校验并提交内联账户编辑器。
    pub(crate) fn save_identity_editor(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.as_mut() else {
            return;
        };
        let Some(edit) = form.identity_edit.as_mut() else {
            return;
        };
        edit.save_password = !edit.password.read(cx).value().is_empty();
        edit.save_passphrase = !edit.passphrase.read(cx).value().is_empty();
        match build_identity_from_edit(edit, cx) {
            Ok(built) => {
                let option = IdentityOption {
                    id: built.identity.id,
                    name: built.identity.name.clone(),
                    username: built.identity.username.clone(),
                    auth: built.identity.auth.clone(),
                };
                if let Some(existing) = form.identities.iter_mut().find(|item| item.id == option.id)
                {
                    *existing = option;
                } else {
                    form.identities.push(option);
                }
                form.identity_choice = Some(built.identity.id);
                form.identity_edit = None;
                cx.emit(SessionFormEvent::SaveIdentity {
                    built: Box::new(built),
                });
                cx.notify();
            }
            Err(error) => {
                if let Some(edit) = form.identity_edit.as_mut() {
                    edit.error = Some(error);
                }
                cx.notify();
            }
        }
    }

    /// 切换连接类型；切到串口且尚无设备清单时请求外壳重新枚举。
    pub(crate) fn set_connection(&mut self, choice: ConnectionChoice, cx: &mut Context<Self>) {
        let Some(form) = self.state.as_mut() else {
            return;
        };
        form.connection = choice;
        let needs_ports = choice == ConnectionChoice::Serial && form.serial_ports.is_empty();
        cx.notify();
        if needs_ports {
            cx.emit(SessionFormEvent::RefreshSerialPorts);
        }
    }

    /// 表单全部文本项的当前值，转发给弹窗脏判定。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        self.state
            .as_ref()
            .map(|form| form.text_values(cx))
            .unwrap_or_default()
    }

    /// 校验并提交表单：成功先关闭再上交 [`SessionFormEvent::Save`]；
    /// 失败把错误写回弹窗并保持打开，等待用户就地改正。
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        let Some((built, save_password, save_passphrase)) = self.build(cx) else {
            return;
        };
        self.state = None;
        cx.emit(SessionFormEvent::Save {
            built: Box::new(built),
            save_password,
            save_passphrase,
        });
        cx.notify();
    }

    /// 关闭表单并上交删除意图；新建态没有可删除的会话，只关闭。
    pub(crate) fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.take() else {
            return;
        };
        if let Some(base) = form.base {
            cx.emit(SessionFormEvent::Delete {
                session_id: base.id,
            });
        }
        cx.notify();
    }

    /// 校验并取出构建结果；失败时把错误写回弹窗。
    fn build(&mut self, cx: &mut Context<Self>) -> Option<(BuiltSession, bool, bool)> {
        let (built, save_password, save_passphrase) = {
            let form = self.state.as_mut()?;
            let save_password = !form.password.read(cx).value().is_empty();
            let save_passphrase = !form.passphrase.read(cx).value().is_empty();
            match form.build_session(cx) {
                Ok(built) => (built, save_password, save_passphrase),
                Err(error) => {
                    form.error = Some(error);
                    cx.notify();
                    return None;
                }
            }
        };
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

/// 根据已有账户候选（或空模板）初始化内联账户编辑器状态。
fn new_identity_edit_state(
    existing: Option<&IdentityOption>,
    window: &mut Window,
    cx: &mut Context<SessionFormView>,
) -> IdentityEditState {
    let (auth, key_path, save_password, save_passphrase) = match existing.map(|item| &item.auth) {
        Some(AuthMethod::PrivateKey {
            path,
            save_passphrase,
            ..
        }) => (
            AuthChoice::PrivateKey,
            path.display().to_string(),
            false,
            *save_passphrase,
        ),
        Some(AuthMethod::Password { save }) => (AuthChoice::Password, String::new(), *save, false),
        Some(AuthMethod::Agent) => (AuthChoice::Agent, String::new(), false, false),
        _ => (AuthChoice::Password, String::new(), true, false),
    };
    IdentityEditState {
        base: existing.map(|item| item.id),
        name: text_input(
            window,
            cx,
            existing.map(|item| item.name.as_str()).unwrap_or(""),
            "",
        ),
        username: text_input(
            window,
            cx,
            existing.map(|item| item.username.as_str()).unwrap_or(""),
            "root",
        ),
        auth,
        key_path: text_input(window, cx, &key_path, "~/.ssh/id_ed25519"),
        password: text_input(
            window,
            cx,
            "",
            &rust_i18n::t!("form.password_optional_placeholder"),
        ),
        passphrase: text_input(
            window,
            cx,
            "",
            &rust_i18n::t!("form.passphrase_optional_placeholder"),
        ),
        save_password,
        save_passphrase,
        error: None,
    }
}
