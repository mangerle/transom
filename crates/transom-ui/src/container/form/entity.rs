//! 容器引擎连接表单实体：打开状态、输入控件与表单动作都归它。
//!
//! # 设计原理（Why）
//! 表单此前由 `ContainerUiState.form: Option<ContainerFormState>` 与散落的
//! `impl AppShell` 方法共同维护：打开、保存、删除与渲染分居 `container/form.rs`、
//! 侧栏与弹窗宿主多处，读者要同时翻几个文件才能回答“谁拥有这次打开”。
//! 实体化后，一次打开期的输入控件与表单状态同生共死，外壳只持有句柄；
//! 表单内的状态变更也只重绘本实体，不再牵动整个外壳。
//!
//! 分组候选、保存落目录与删除后关闭工作区标签仍归外壳：实体不反查
//! `AppShell`，需要时通过 [`ContainerFormEvent`] 上交意图；渲染所需的分组
//! 快照由外壳在渲染时注入。测试连接由实体自己经注入的命令通道投递，
//! 回包再按请求 id 回到 [`ContainerFormView::set_test_result`]。
//!
//! # 代价与局限
//! 事件只携带结果，外壳不能回读实体补数据；保存成功即关闭，失败则保留
//! 弹窗并由实体自己写错误文案，两条路径必须各自保持一致。

use super::*;
use crate::app::command_send_error_text;
use crate::palette::localized_error;
use transom_core::{AppError, ContainerEngineInfo};

/// 容器引擎连接表单交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
pub(crate) enum ContainerFormEvent {
    /// 校验通过后的连接配置；实体已先关闭再发出本事件。
    /// 配置体积远大于删除事件，装箱避免枚举整体变大。
    Save(Box<ContainerEngineProfile>),
    /// 删除当前编辑的连接；实体已先关闭再发出本事件。
    Delete(EngineProfileId),
}

impl EventEmitter<ContainerFormEvent> for ContainerFormView {}

/// 容器引擎连接表单实体。
pub(crate) struct ContainerFormView {
    /// 未打开时为 `None`；每次打开都重建输入控件，避免残留上次编辑内容。
    state: Option<ContainerFormState>,
    /// 命令通道：测试连接直接投递，不经过外壳。
    command_tx: flume::Sender<AppCommand>,
}

impl ContainerFormView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new(command_tx: flume::Sender<AppCommand>) -> Self {
        Self {
            state: None,
            command_tx,
        }
    }

    /// 表单是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn state(&self) -> Option<&ContainerFormState> {
        self.state.as_ref()
    }

    /// 打开新建/编辑表单；每次打开都重建输入控件。
    pub(crate) fn open(
        &mut self,
        profile: Option<ContainerEngineProfile>,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state = Some(ContainerFormState::new(
            window,
            cx,
            profile.as_ref(),
            folder,
        ));
        cx.notify();
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

    /// 校验并提交表单：成功先关闭再上交 [`ContainerFormEvent::Save`]；
    /// 失败把错误写回弹窗并保持打开，等待用户就地改正。
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.build_profile(cx) else {
            return;
        };
        self.close(cx);
        cx.emit(ContainerFormEvent::Save(Box::new(profile)));
    }

    /// 测试当前配置：先写“测试中”，再经自身命令通道投递探测请求。
    ///
    /// 投递失败（队列已满或控制器退出）必须清掉在途请求并写错误，
    /// 否则状态会一直停在“测试中”。
    pub(crate) fn test(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.build_profile(cx) else {
            return;
        };
        let request_id = ContainerRequestId::new();
        if let Some(form) = self.state.as_mut() {
            form.test_request = Some(request_id);
            form.test_status = Some(rust_i18n::t!("container.form.testing").to_string());
        }
        let command = AppCommand::from(ContainerCommand::TestConnection {
            request_id,
            profile,
        });
        match self.command_tx.try_send(command) {
            Ok(()) => cx.notify(),
            Err(error) => {
                let message = command_send_error_text(error);
                if let Some(form) = self.state.as_mut() {
                    form.test_request = None;
                    form.test_ok = false;
                    form.test_status = Some(message);
                }
                cx.notify();
            }
        }
    }

    /// 写回测试连接结果；只处理与当前在途请求匹配的回包。
    pub(crate) fn set_test_result(
        &mut self,
        request_id: ContainerRequestId,
        info: Option<Box<ContainerEngineInfo>>,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.state.as_mut() else {
            return;
        };
        if form.test_request != Some(request_id) {
            return;
        }
        form.test_request = None;
        match (info, error) {
            (Some(info), _) => {
                form.test_ok = true;
                form.test_status = Some(
                    rust_i18n::t!(
                        "container.form.test_ok",
                        platform = info.platform,
                        version = info.version
                    )
                    .to_string(),
                );
            }
            (None, Some(error)) => {
                form.test_ok = false;
                form.test_status = Some(localized_error(&error));
            }
            (None, None) => {
                form.test_ok = false;
                form.test_status = Some(rust_i18n::t!("container.form.test_failed").to_string());
            }
        }
        cx.notify();
    }

    /// 关闭表单并上交删除意图；新建态没有可删除的连接，只关闭。
    pub(crate) fn delete(&mut self, cx: &mut Context<Self>) {
        let editing = self.state.as_ref().and_then(|form| form.editing);
        self.close(cx);
        if let Some(id) = editing {
            cx.emit(ContainerFormEvent::Delete(id));
        }
    }

    /// 校验并取出连接配置；失败时把错误写回弹窗并保持打开。
    fn build_profile(&mut self, cx: &mut Context<Self>) -> Option<ContainerEngineProfile> {
        let form = self.state.as_ref()?;
        match form.to_profile(cx) {
            Ok(profile) => Some(profile),
            Err(message) => {
                if let Some(form) = self.state.as_mut() {
                    form.test_ok = false;
                    form.test_status = Some(message);
                }
                cx.notify();
                None
            }
        }
    }

    /// 切换引擎种类；不改变已填端点值。
    pub(super) fn set_kind(&mut self, kind: ContainerEngineKind, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.kind = kind);
    }

    /// 选择表单图标；`Auto` 表示按引擎种类自动取图标。
    pub(super) fn set_icon(&mut self, icon: SessionIcon, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.icon = icon);
    }

    /// 选择环境色；`None` 表示不染色。
    pub(super) fn set_color(&mut self, color: SessionColor, cx: &mut Context<Self>) {
        self.edit(cx, |form| form.color = color);
    }

    /// 选择分组；切换分组后旧测试结果不再对应当前配置。
    pub(super) fn set_folder(&mut self, folder: Option<FolderId>, cx: &mut Context<Self>) {
        self.edit(cx, |form| {
            form.folder_id = folder;
            form.test_status = None;
        });
    }

    /// 切换端点档位并同步主输入框占位提示；旧测试结果失效。
    pub(super) fn set_endpoint_choice(
        &mut self,
        choice: EndpointChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.state.as_mut() {
            form.endpoint_choice = choice;
            form.test_status = None;
            form.endpoint_value.update(cx, |input, cx| {
                input.set_placeholder(
                    rust_i18n::t!(choice.placeholder_key()).to_string(),
                    window,
                    cx,
                );
            });
        }
        cx.notify();
    }

    /// 开关 TCP TLS 证书区；关闭时保留已填路径，便于再次启用。
    pub(super) fn set_tls_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.edit(cx, |form| {
            form.tls.enabled = enabled;
            form.test_status = None;
        });
    }

    /// 开关 SSH sudo 提权；切换后旧测试结果失效。
    pub(super) fn set_ssh_sudo(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.edit(cx, |form| {
            form.ssh.use_sudo = enabled;
            form.test_status = None;
        });
    }

    /// 在表单内就地修改字段并重绘；关闭状态下忽略。
    fn edit(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut ContainerFormState)) {
        if let Some(form) = self.state.as_mut() {
            update(form);
            cx.notify();
        }
    }
}
