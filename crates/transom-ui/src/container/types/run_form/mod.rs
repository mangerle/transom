//! 运行容器表单数据模型与状态入口。
//!
//! 设计原理（Why）：
//! - 将原本长字符串拆解为可独立绑定焦点、支持增删与模式切换的结构化微表格；
//! - 采用智能端口与容器名预感知，针对高频镜像提供零配置开箱即用能力；
//! - 实时生成标准可执行的 Docker CLI 命令预览，兼顾图形化效率与终端专业控制。

mod infer;
mod items;
mod spec;

pub(crate) use infer::*;
pub(crate) use items::*;

use gpui_kit::component::input::InputState;
use gpui_kit::{App, Context, Entity, Window};
use transom_core::TabId;

/// 容器运行预设模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunPreset {
    /// 后台常驻服务（-d，--restart unless-stopped）。
    Daemon,
    /// 一次性交互调试工具（--rm，-it）。
    Interactive,
}

/// 运行新容器表单会话状态。
pub(crate) type ContainerRunForm = RunContainerFormState;

/// 运行新容器表单会话状态实现。
pub(crate) struct RunContainerFormState {
    pub(crate) tab_id: TabId,
    pub(crate) name: Entity<InputState>,
    pub(crate) image: Entity<InputState>,
    pub(crate) command: Entity<InputState>,
    pub(crate) ports: Vec<PortMappingItem>,
    pub(crate) volumes: Vec<VolumeMappingItem>,
    pub(crate) envs: Vec<EnvVarItem>,
    pub(crate) health: Entity<InputState>,
    pub(crate) health_interval: Entity<InputState>,
    pub(crate) memory: Entity<InputState>,
    pub(crate) cpus: Entity<InputState>,
    pub(crate) restart: Option<String>,
    pub(crate) preset: RunPreset,
    pub(crate) advanced_open: bool,
    pub(crate) error: Option<String>,
    pub(crate) copy_toast: bool,
}

impl RunContainerFormState {
    /// 基于给定镜像引用创建运行容器表单状态，支持智能解析端口与默认名。
    pub(crate) fn new(
        tab_id: TabId,
        image_hint: Option<String>,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let (name_hint, ports_hint) = if let Some(ref img) = image_hint {
            infer_image_defaults(img)
        } else {
            (None, Vec::new())
        };

        let mut ports = Vec::new();
        for (h, c, proto) in ports_hint {
            ports.push(PortMappingItem::new(Some(h), Some(c), proto, window, cx));
        }

        Self {
            tab_id,
            name: run_input("container.run.name_placeholder", name_hint, window, cx),
            image: run_input("container.run.image_placeholder", image_hint, window, cx),
            command: run_input("container.run.command_placeholder", None, window, cx),
            ports,
            volumes: Vec::new(),
            envs: Vec::new(),
            health: run_input("container.run.health_placeholder", None, window, cx),
            health_interval: run_input(
                "container.run.health_interval_placeholder",
                None,
                window,
                cx,
            ),
            memory: run_input("container.run.memory_placeholder", None, window, cx),
            cpus: run_input("container.run.cpus_placeholder", None, window, cx),
            restart: Some("unless-stopped".to_string()),
            preset: RunPreset::Daemon,
            advanced_open: false,
            error: None,
            copy_toast: false,
        }
    }

    /// 基于已有容器实例反向推导出克隆表单状态。
    pub(crate) fn from_details(
        tab_id: TabId,
        details: &transom_core::ContainerDetails,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let name_hint = if details.name.is_empty() {
            None
        } else {
            Some(format!("{}-clone", details.name))
        };
        let image_hint = if details.image.is_empty() {
            None
        } else {
            Some(details.image.clone())
        };
        let command_hint = if details.command.is_empty() {
            None
        } else {
            Some(details.command.clone())
        };

        let mut ports = Vec::new();
        for p in &details.ports {
            ports.push(PortMappingItem::new(
                p.public_port,
                Some(p.private_port),
                &p.protocol,
                window,
                cx,
            ));
        }

        let mut volumes = Vec::new();
        for m in &details.mounts {
            if !m.source.is_empty() && !m.destination.is_empty() {
                volumes.push(VolumeMappingItem::new(
                    Some(&m.source),
                    Some(&m.destination),
                    m.mode.contains("ro"),
                    window,
                    cx,
                ));
            }
        }

        let mut envs = Vec::new();
        for e in &details.env {
            if let Some((k, v)) = e.split_once('=') {
                envs.push(EnvVarItem::new(Some(k), Some(v), window, cx));
            }
        }

        let advanced_open = !envs.is_empty() || !volumes.is_empty();

        Self {
            tab_id,
            name: run_input("container.run.name_placeholder", name_hint, window, cx),
            image: run_input("container.run.image_placeholder", image_hint, window, cx),
            command: run_input(
                "container.run.command_placeholder",
                command_hint,
                window,
                cx,
            ),
            ports,
            volumes,
            envs,
            health: run_input("container.run.health_placeholder", None, window, cx),
            health_interval: run_input(
                "container.run.health_interval_placeholder",
                None,
                window,
                cx,
            ),
            memory: run_input("container.run.memory_placeholder", None, window, cx),
            cpus: run_input("container.run.cpus_placeholder", None, window, cx),
            restart: details.restart_policy.clone(),
            preset: RunPreset::Daemon,
            advanced_open,
            error: None,
            copy_toast: false,
        }
    }

    /// 收集表单当前所有输入框文本（用作关闭弹窗时的未提交内容基线对比与脏检查）。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        let mut values = vec![
            self.name.read(cx).value().to_string(),
            self.image.read(cx).value().to_string(),
            self.command.read(cx).value().to_string(),
            self.health.read(cx).value().to_string(),
            self.health_interval.read(cx).value().to_string(),
            self.memory.read(cx).value().to_string(),
            self.cpus.read(cx).value().to_string(),
        ];
        for p in &self.ports {
            values.push(p.host_port.read(cx).value().to_string());
            values.push(p.container_port.read(cx).value().to_string());
        }
        for v in &self.volumes {
            values.push(v.host_path.read(cx).value().to_string());
            values.push(v.container_path.read(cx).value().to_string());
        }
        for e in &self.envs {
            values.push(e.key.read(cx).value().to_string());
            values.push(e.value.read(cx).value().to_string());
        }
        values
    }
}
