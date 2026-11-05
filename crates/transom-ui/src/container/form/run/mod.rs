//! 运行容器表单模块入口：生命周期管理、克隆与创建提交。
//!
//! 设计原理（Why）：
//! - 将原本单一的长表单解耦为子模块目录（view、items、preview），保证各文件行数健康；
//! - 集中纳管容器创建参数的收集、错误校验与跨系统总线事件分发。

mod items;
mod preview;
mod view;

use super::*;
use crate::app::AppShell;

impl AppShell {
    /// 打开「运行容器」表单；从镜像行进入时预填镜像引用。
    pub(crate) fn open_container_run_form(
        &mut self,
        tab_id: transom_core::TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.containers.run_form =
            Some(ContainerRunForm::new(tab_id, image_hint, window, cx));
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
        self.state.containers.run_form =
            Some(ContainerRunForm::from_details(tab_id, details, window, cx));
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
        self.state.containers.run_form =
            Some(ContainerRunForm::for_recreate(tab_id, details, window, cx));
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

    /// 关闭运行表单。
    pub(crate) fn dismiss_container_run(&mut self, cx: &mut Context<Self>) {
        self.state.containers.run_form = None;
        cx.notify();
    }

    /// 提交运行：解析并校验 spec 后交给控制器，错误就地展示。
    pub(crate) fn submit_container_run(
        &mut self,
        tab_id: transom_core::TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.state.containers.run_form.as_ref() else {
            return;
        };
        let spec = match form.to_spec(cx) {
            Ok(spec) => spec,
            Err(message) => {
                if let Some(form) = self.state.containers.run_form.as_mut() {
                    form.error = Some(message);
                }
                cx.notify();
                return;
            }
        };
        let recreate_id = form.recreate_container_id.clone();
        let request_id = ContainerRequestId::new();
        self.state.containers.run_form = None;
        let command = if let Some(old_container_id) = recreate_id {
            ContainerCommand::RecreateContainer {
                tab_id,
                request_id,
                old_container_id,
                spec,
            }
        } else {
            ContainerCommand::CreateContainer {
                tab_id,
                request_id,
                spec,
            }
        };
        self.send_tracked(tab_id, request_id, command);
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.apply_section(ContainerSection::Containers);
            workspace.notice = Some(rust_i18n::t!("container.run.submitting").to_string());
        }
        cx.notify();
    }
}
