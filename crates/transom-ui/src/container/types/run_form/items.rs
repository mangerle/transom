//! 运行容器表单微条目数据结构：端口、数据卷与环境变量输入项。
//!
//! 设计原理（Why）：
//! - 将原本长字符串拆解为可独立绑定焦点、支持增删与模式切换的输入项实体；
//! - 封装条目默认初始占位与协议状态。

use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext, Context, Entity, Window};

/// 端口映射条目。
pub(crate) struct PortMappingItem {
    pub(crate) id: String,
    pub(crate) host_port: Entity<InputState>,
    pub(crate) container_port: Entity<InputState>,
    pub(crate) protocol: String,
}

impl PortMappingItem {
    pub(crate) fn new(
        host: Option<u16>,
        container: Option<u16>,
        protocol: &str,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let id = format!(
            "port-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        let host_port = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("8080".to_string());
            if let Some(h) = host {
                s = s.default_value(h.to_string());
            }
            s
        });

        let container_port = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("80".to_string());
            if let Some(c) = container {
                s = s.default_value(c.to_string());
            }
            s
        });

        Self {
            id,
            host_port,
            container_port,
            protocol: protocol.to_ascii_lowercase(),
        }
    }
}

/// 数据卷挂载条目。
pub(crate) struct VolumeMappingItem {
    pub(crate) id: String,
    pub(crate) host_path: Entity<InputState>,
    pub(crate) container_path: Entity<InputState>,
    pub(crate) read_only: bool,
}

impl VolumeMappingItem {
    pub(crate) fn new(
        host: Option<&str>,
        container: Option<&str>,
        read_only: bool,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let id = format!(
            "vol-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        let host_path = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.run.binds_placeholder").to_string(),
            );
            if let Some(h) = host {
                s = s.default_value(h.to_string());
            }
            s
        });

        let container_path = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("/app/data".to_string());
            if let Some(c) = container {
                s = s.default_value(c.to_string());
            }
            s
        });

        Self {
            id,
            host_path,
            container_path,
            read_only,
        }
    }
}

/// 环境变量条目。
pub(crate) struct EnvVarItem {
    pub(crate) id: String,
    pub(crate) key: Entity<InputState>,
    pub(crate) value: Entity<InputState>,
}

impl EnvVarItem {
    pub(crate) fn new(
        key_str: Option<&str>,
        val_str: Option<&str>,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let id = format!(
            "env-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        let key = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("KEY".to_string());
            if let Some(k) = key_str {
                s = s.default_value(k.to_string());
            }
            s
        });

        let value = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("VALUE".to_string());
            if let Some(v) = val_str {
                s = s.default_value(v.to_string());
            }
            s
        });

        Self { id, key, value }
    }
}

/// 创建具备多语言占位符的表单输入框。
pub(crate) fn run_input(
    placeholder: &'static str,
    initial: Option<String>,
    window: &mut Window,
    cx: &mut Context<crate::app::AppShell>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(rust_i18n::t!(placeholder).to_string());
        if let Some(text) = initial {
            state = state.default_value(text);
        }
        state
    })
}
