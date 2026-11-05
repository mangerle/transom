//! 偏好设置中的容器与镜像源页：远端镜像仓库（Docker Hub / 私有 Harbor）管理。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与推送所依赖的远端仓库凭证统一在设置中心进行可视化配置与管理；
//! - 采用列表与内联表单解耦拆分子模块设计，单文件代码量均严格控制在规范行数内；
//! - 遵循应用级单一设置源契约，任何增删改均通过 `update_app_settings` 驱动响应式更新，
//!   并调用 `persist_settings_reported` 异步安全落盘。

mod form;
mod list;

use super::SettingsView;
use super::modal_state::RegistryEditState;
use crate::app::{app_settings, update_app_settings};
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{ContainerRegistry, SecretKey};

impl SettingsView {
    /// 渲染容器与镜像源设置组。
    pub(crate) fn render_settings_container_registries(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let auto_prune = app_settings(cx).container_automation.auto_prune_enabled;
        let automation = crate::widgets::setting_row(
            "settings.container_auto_prune",
            Some("settings.container_auto_prune_desc"),
            Switch::new("container-auto-prune")
                .checked(auto_prune)
                .on_click(cx.listener(|this, &checked, _, cx| {
                    update_app_settings(cx, |settings| {
                        settings.container_automation.auto_prune_enabled = checked;
                    });
                    let snapshot = app_settings(cx).clone();
                    this.persist_settings_reported(&snapshot, cx);
                    cx.notify();
                }))
                .into_any_element(),
            p,
        );
        let body = if let Some(edit_state) = self.state.registry_edit.clone() {
            self.render_registry_edit_form(edit_state, p, cx)
        } else {
            self.render_registries_list(p, cx)
        };
        v_flex()
            .gap_3()
            .w_full()
            .child(automation)
            .child(body)
            .into_any_element()
    }

    /// 初始化仓库编辑状态。
    pub(super) fn init_registry_edit(
        &mut self,
        target_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (name_val, url_val, user_val, pass_val, is_default_val) =
            if let Some(ref id) = target_id {
                let regs = app_settings(cx).container_registries.clone();
                if let Some(r) = regs.iter().find(|r| r.id == *id) {
                    (
                        r.name.clone(),
                        r.url.clone(),
                        r.username.clone(),
                        r.password.clone(),
                        r.is_default,
                    )
                } else {
                    (
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        false,
                    )
                }
            } else {
                (
                    String::new(),
                    "https://".to_string(),
                    String::new(),
                    String::new(),
                    false,
                )
            };

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.registry_name_placeholder").to_string())
                .default_value(name_val)
        });
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.registry_url_placeholder").to_string())
                .default_value(url_val)
        });
        let username = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.registry_username_placeholder").to_string())
                .default_value(user_val)
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(rust_i18n::t!("settings.registry_password_placeholder").to_string())
                .default_value(pass_val)
        });

        self.state.registry_edit = Some(RegistryEditState {
            editing_id: target_id,
            name,
            url,
            username,
            password,
            is_default: is_default_val,
            error: None,
        });
        cx.notify();
    }

    /// 提交当前正在编辑的镜像仓库。
    pub(super) fn submit_registry_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.state.registry_edit.as_ref() else {
            return;
        };

        let name = edit.name.read(cx).value().trim().to_string();
        let mut url = edit.url.read(cx).value().trim().to_string();
        let username = edit.username.read(cx).value().trim().to_string();
        let password = edit.password.read(cx).value().trim().to_string();
        let is_default = edit.is_default;
        let editing_id = edit.editing_id.clone();

        if name.is_empty() {
            if let Some(e) = self.state.registry_edit.as_mut() {
                e.error = Some(rust_i18n::t!("settings.registry_name_required").to_string());
            }
            cx.notify();
            return;
        }

        if url.is_empty() || url == "https://" || url == "http://" {
            if let Some(e) = self.state.registry_edit.as_mut() {
                e.error = Some(rust_i18n::t!("settings.registry_url_required").to_string());
            }
            cx.notify();
            return;
        }

        if !url.starts_with("http://") && !url.starts_with("https://") {
            url = format!("https://{url}");
        }

        let reg_id = editing_id.unwrap_or_else(|| {
            format!(
                "reg-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            )
        });

        let secret_key = SecretKey::registry_password(&reg_id);
        if password.is_empty() {
            if let Err(error) = self.services.secret_store.delete(&secret_key) {
                log::warn!("删除镜像仓库密码失败，仓库: {reg_id}，原因: {error}");
            }
        } else if let Err(error) = self.services.secret_store.set(&secret_key, &password) {
            if let Some(edit) = self.state.registry_edit.as_mut() {
                edit.error = Some(
                    rust_i18n::t!("settings.registry_secret_failed", error = error.to_string())
                        .to_string(),
                );
            }
            cx.notify();
            return;
        }

        let new_registry = ContainerRegistry {
            id: reg_id.clone(),
            name,
            url,
            username,
            password,
            is_default,
        };

        update_app_settings(cx, |s| {
            if is_default {
                for r in &mut s.container_registries {
                    r.is_default = false;
                }
            }

            if let Some(idx) = s.container_registries.iter().position(|r| r.id == reg_id) {
                s.container_registries[idx] = new_registry;
            } else {
                s.container_registries.push(new_registry);
            }
        });

        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot, cx);

        self.state.registry_edit = None;
        cx.notify();
    }

    /// 设置某个仓库为默认源。
    pub(super) fn set_default_registry(&mut self, reg_id: &str, cx: &mut Context<Self>) {
        update_app_settings(cx, |s| {
            for r in &mut s.container_registries {
                r.is_default = r.id == reg_id;
            }
        });

        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot, cx);
        cx.notify();
    }

    /// 删除一个自定义镜像仓库。
    pub(super) fn delete_registry(&mut self, reg_id: &str, cx: &mut Context<Self>) {
        if reg_id == "docker-hub" {
            return;
        }
        if let Err(error) = self
            .services
            .secret_store
            .delete(&SecretKey::registry_password(reg_id))
        {
            log::warn!("删除镜像仓库密码失败，仓库: {reg_id}，原因: {error}");
        }

        update_app_settings(cx, |s| {
            s.container_registries.retain(|r| r.id != reg_id);
            // 若删除了当前默认源，自动将 Docker Hub 设为默认
            if !s.container_registries.iter().any(|r| r.is_default)
                && let Some(hub) = s
                    .container_registries
                    .iter_mut()
                    .find(|r| r.id == "docker-hub")
            {
                hub.is_default = true;
            }
        });

        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot, cx);
        cx.notify();
    }
}
