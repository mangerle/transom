//! 运行容器表单实体：打开状态、输入控件与提交动作都归它。
//!
//! # 设计原理（Why）
//! 表单此前由 `ContainerUiState.run_form: Option<RunContainerFormState>` 与
//! 散落的 `impl AppShell` 方法共同维护，打开、提交与渲染分居三处，读者要同时
//! 翻几个文件才能回答“谁拥有这次打开”。实体化后，一次打开期的输入控件与
//! 表单状态同生共死，外壳只持有句柄；提交成功即关闭并上交
//! [`ContainerRunFormEvent::Submit`]，由外壳定位目标工作区后投递命令。
//!
//! 表单不持有命令通道：创建与重建都要先确认目标标签页仍然存活，命令由外壳
//! 在事件处理时投递，实体只表达“这次提交要创建什么”。
//!
//! # 代价与局限
//! 事件只携带结果，外壳处理时禁止回读实体，否则会在实体派发期间触发 GPUI 的
//! 双重租借崩溃。本阶段渲染仍留在 `form/run/*` 的 `impl AppShell` 上，字段
//! 修改暂经 [`ContainerRunFormView::edit_state`] 桥接；渲染迁移完成后删除该
//! 入口，改为具名状态方法。

use crate::container::types::RunContainerFormState;
use crate::container::types::run_form::{EnvVarItem, PortMappingItem, VolumeMappingItem};
use gpui_kit::{App, Context, EventEmitter, Window};
use transom_core::{ContainerCreateSpec, ContainerDetails, TabId};

/// 运行容器表单交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
pub(crate) enum ContainerRunFormEvent {
    /// 校验通过的创建参数；实体已先关闭再发出本事件。
    Submit {
        /// 目标容器工作区标签。
        tab_id: TabId,
        /// 解析并校验后的创建参数。
        spec: ContainerCreateSpec,
        /// 就地重建的目标原容器 ID；`None` 表示新建。
        recreate_id: Option<String>,
    },
}

impl EventEmitter<ContainerRunFormEvent> for ContainerRunFormView {}

/// 运行容器表单实体。
pub(crate) struct ContainerRunFormView {
    /// 未打开时为 `None`；每次打开都重建输入控件，避免残留上次编辑内容。
    state: Option<RunContainerFormState>,
}

impl ContainerRunFormView {
    /// 组装常驻实体；打开状态由各 `open_*` 入口控制。
    pub(crate) fn new() -> Self {
        Self { state: None }
    }

    /// 表单是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与脏判定共用同一入口。
    pub(crate) fn state(&self) -> Option<&RunContainerFormState> {
        self.state.as_ref()
    }

    /// 打开新建表单；`image_hint` 来自镜像行，用于预填镜像并推断端口。
    pub(crate) fn open_new(
        &mut self,
        tab_id: TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state = Some(RunContainerFormState::new(tab_id, image_hint, window, cx));
        cx.notify();
    }

    /// 打开克隆表单；基于已有容器配置反向预填。
    pub(crate) fn open_clone(
        &mut self,
        tab_id: TabId,
        details: &ContainerDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state = Some(RunContainerFormState::from_details(
            tab_id, details, window, cx,
        ));
        cx.notify();
    }

    /// 打开就地重建表单；保留原容器名并记录原容器 ID。
    pub(crate) fn open_recreate(
        &mut self,
        tab_id: TabId,
        details: &ContainerDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state = Some(RunContainerFormState::for_recreate(
            tab_id, details, window, cx,
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

    /// 校验并提交表单：成功先关闭再上交 [`ContainerRunFormEvent::Submit`]；
    /// 失败把错误写回弹窗并保持打开，等待用户就地改正。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.as_ref() else {
            return;
        };
        let spec = match form.to_spec(cx) {
            Ok(spec) => spec,
            Err(message) => {
                if let Some(form) = self.state.as_mut() {
                    form.error = Some(message);
                }
                cx.notify();
                return;
            }
        };
        let tab_id = form.tab_id;
        let recreate_id = form.recreate_container_id.clone();
        self.close(cx);
        cx.emit(ContainerRunFormEvent::Submit {
            tab_id,
            spec,
            recreate_id,
        });
    }

    /// 在打开期就地修改字段并重绘；关闭状态下忽略。
    ///
    /// 这是渲染迁移前的临时桥：本阶段 `form/run/*` 仍由外壳渲染，回调拿不到
    /// `&mut RunContainerFormState`，只能经实体更新；渲染迁入实体后删除本入口，
    /// 改为具名状态方法。
    pub(crate) fn edit_state(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut RunContainerFormState),
    ) {
        if let Some(form) = self.state.as_mut() {
            update(form);
            cx.notify();
        }
    }

    /// 追加一条端口映射行；窗口用于创建输入控件。
    pub(crate) fn add_port(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.state.as_mut() {
            form.ports
                .push(PortMappingItem::new(None, None, "tcp", window, cx));
            cx.notify();
        }
    }

    /// 追加一条数据卷挂载行；窗口用于创建输入控件。
    pub(crate) fn add_volume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.state.as_mut() {
            form.volumes
                .push(VolumeMappingItem::new(None, None, false, window, cx));
            cx.notify();
        }
    }

    /// 追加一条环境变量行；窗口用于创建输入控件。
    pub(crate) fn add_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.state.as_mut() {
            form.envs.push(EnvVarItem::new(None, None, window, cx));
            cx.notify();
        }
    }
}
