//! 运行容器表单模块入口：实体打开入口与提交事件处理。
//!
//! 设计原理（Why）：
//! - 表单状态、输入控件与提交动作归 [`ContainerRunFormView`]，外壳只保留
//!   打开入口、克隆/重建的详情收集与命令投递；
//! - 子模块目录（view、items、preview）继续承载本阶段的外壳渲染，保证各文件
//!   行数健康，渲染迁移另行安排。
//!
//! # 代价与局限
//! 渲染仍由 `form/run/{view,items,preview}.rs` 的 `impl AppShell` 完成，
//! 字段修改经实体的临时桥接入口完成；迁移后这些入口会收敛为具名状态方法。

mod entity;
mod items;
mod preview;
mod view;

use super::*;

pub(crate) use entity::{ContainerRunFormEvent, ContainerRunFormView};

impl AppShell {
    /// 打开「运行容器」表单；从镜像行进入时预填镜像引用。
    pub(crate) fn open_container_run_form(
        &mut self,
        tab_id: transom_core::TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.modals.tools.run_form.clone();
        view.update(cx, |view, cx| view.open_new(tab_id, image_hint, window, cx));
        // 弹窗栈在外壳渲染里对账，实体变脏不会自动跑外壳的 reconcile。
        cx.notify();
    }

    /// 打开「克隆容器」表单；基于已有容器配置反向预填。
    pub(crate) fn open_container_clone_form(
        &mut self,
        tab_id: transom_core::TabId,
        details: &transom_core::ContainerDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.modals.tools.run_form.clone();
        view.update(cx, |view, cx| view.open_clone(tab_id, details, window, cx));
        cx.notify();
    }

    /// 打开容器就地修改与重建表单。
    pub(crate) fn open_container_recreate_form(
        &mut self,
        tab_id: transom_core::TabId,
        details: &transom_core::ContainerDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.modals.tools.run_form.clone();
        view.update(cx, |view, cx| {
            view.open_recreate(tab_id, details, window, cx)
        });
        cx.notify();
    }

    /// 触发容器克隆：优先读取已有详情，未全量 inspect 时由 summary 补齐必要参数。
    pub(crate) fn clone_container(
        &mut self,
        tab_id: transom_core::TabId,
        id: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let workspace = &workspace.read(cx).state;
        if let Some(details) = workspace
            .details
            .as_ref()
            .filter(|d| d.id == id || d.name == name)
            .cloned()
        {
            self.open_container_clone_form(tab_id, &details, window, cx);
        } else if let Some(summary) = workspace
            .containers
            .iter()
            .find(|c| c.id == id || c.name == name)
            .cloned()
        {
            let details = transom_core::ContainerDetails {
                id: summary.id,
                name: summary.name,
                image: summary.image,
                image_id: String::new(),
                command: String::new(),
                entrypoint: Vec::new(),
                user: None,
                working_dir: None,
                env: Vec::new(),
                state: summary.state,
                status: summary.status,
                created: summary.created,
                started_at: None,
                finished_at: None,
                exit_code: None,
                health: None,
                health_details: None,
                restart_policy: None,
                ports: summary.ports,
                mounts: summary.mounts,
                compose_project: summary.compose_project,
                oom_killed: false,
                size_rw: 0,
                size_root_fs: 0,
                exposed_ports: Vec::new(),
                inspect: String::new(),
            };
            self.open_container_clone_form(tab_id, &details, window, cx);
        }
    }

    /// 触发容器就地修改与重建：加载配置后在运行表单中保留原容器名，提交时原子重建。
    pub(crate) fn recreate_container(
        &mut self,
        tab_id: transom_core::TabId,
        id: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let workspace = &workspace.read(cx).state;
        if let Some(details) = workspace
            .details
            .as_ref()
            .filter(|d| d.id == id || d.name == name)
            .cloned()
        {
            self.open_container_recreate_form(tab_id, &details, window, cx);
        } else if let Some(summary) = workspace
            .containers
            .iter()
            .find(|c| c.id == id || c.name == name)
            .cloned()
        {
            let details = transom_core::ContainerDetails {
                id: summary.id,
                name: summary.name,
                image: summary.image,
                image_id: String::new(),
                command: String::new(),
                entrypoint: Vec::new(),
                user: None,
                working_dir: None,
                env: Vec::new(),
                state: summary.state,
                status: summary.status,
                created: summary.created,
                started_at: None,
                finished_at: None,
                exit_code: None,
                health: None,
                health_details: None,
                restart_policy: None,
                ports: summary.ports,
                mounts: summary.mounts,
                compose_project: summary.compose_project,
                oom_killed: false,
                size_rw: 0,
                size_root_fs: 0,
                exposed_ports: Vec::new(),
                inspect: String::new(),
            };
            self.open_container_recreate_form(tab_id, &details, window, cx);
        }
    }

    /// 消费运行表单实体上交的提交意图。
    ///
    /// 事件自带创建参数与目标标签，处理时禁止回读实体：派发阶段实体仍被租借。
    /// 目标工作区已关闭时静默丢弃命令，避免把结果发给不存在的标签页。
    pub(crate) fn on_container_run_form_event(
        &mut self,
        event: &ContainerRunFormEvent,
        cx: &mut Context<Self>,
    ) {
        let ContainerRunFormEvent::Submit {
            tab_id,
            spec,
            recreate_id,
        } = event;
        let Some(workspace) = self.state.container_workspace(*tab_id) else {
            cx.notify();
            return;
        };
        let request_id = ContainerRequestId::new();
        let command = if let Some(old_container_id) = recreate_id {
            ContainerCommand::RecreateContainer {
                tab_id: *tab_id,
                request_id,
                old_container_id: old_container_id.clone(),
                spec: spec.clone(),
            }
        } else {
            ContainerCommand::CreateContainer {
                tab_id: *tab_id,
                request_id,
                spec: spec.clone(),
            }
        };
        workspace.update(cx, |workspace, _| {
            workspace.send_tracked(request_id, command);
            workspace.state.apply_section(ContainerSection::Containers);
            workspace.state.notice = Some(rust_i18n::t!("container.run.submitting").to_string());
        });
        cx.notify();
    }
}
