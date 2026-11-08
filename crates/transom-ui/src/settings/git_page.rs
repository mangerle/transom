//! 偏好设置中心之 Git 版本控制配置页（对标 openidea / IntelliJ IDEA 设置体系）。
//!
//! 设计原理（Why）：
//! - 将版本控制相关偏好独立归档，集中管理 Git 可执行程序路径、提交推送安全与差异对比偏好；
//! - 遵循 GPUI-kit 与 Transom 统一设置组件规范（`setting_row`, `setting_field_row`）；
//! - 包含 openidea / IntelliJ IDEA 标志性的“测试 (Test)”Git 可执行程序与版本检测回显；
//! - 各输入字段直接绑定 `SettingsModalState`，由 `collect_git_preferences` 统一校验与落盘。

use crate::app::spawn_blocking_local;
use crate::app::{app_settings, update_app_settings};
use crate::palette::Palette;
use crate::settings::SettingsView;
use crate::theme::tokens::{space, typography};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl SettingsView {
    /// 执行 Git 可执行程序版本探测（`git --version`）。
    fn test_git_executable(&mut self, cx: &mut Context<Self>) {
        if self.state.git_testing {
            return;
        }
        let exe = self
            .state
            .git_executable_path
            .read(cx)
            .value()
            .trim()
            .to_string();
        let exe = if exe.is_empty() {
            "git".to_string()
        } else {
            exe
        };
        self.state.git_testing = true;
        self.state.git_test_result = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let res = spawn_blocking_local(move || {
                let output = std::process::Command::new(&exe).arg("--version").output();
                match output {
                    Ok(out) if out.status.success() => {
                        let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        Ok(ver)
                    }
                    Ok(out) => {
                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        let msg = if err.is_empty() {
                            rust_i18n::t!(
                                "settings.git_process_exit_status",
                                status = out.status.to_string()
                            )
                            .to_string()
                        } else {
                            err
                        };
                        Err(msg)
                    }
                    Err(e) => Err(e.to_string()),
                }
            })
            .await;

            let test_result = match res {
                Some(r) => r,
                None => Err(rust_i18n::t!("settings.git_probe_thread_failed").to_string()),
            };

            let _ = this.update(cx, |this, cx| {
                this.state.git_testing = false;
                this.state.git_test_result = Some(test_result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Git 可执行程序与基础环境组。
    pub(crate) fn render_settings_git_executable(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let test_label = if self.state.git_testing {
            rust_i18n::t!("settings.git_testing_executable").to_string()
        } else {
            rust_i18n::t!("settings.git_test_executable").to_string()
        };

        let test_btn = Button::new("btn-test-git-executable")
            .small()
            .label(test_label)
            .disabled(self.state.git_testing)
            .on_click(cx.listener(|this, _, _, cx| {
                this.test_git_executable(cx);
            }));

        let exe_input = Input::new(&self.state.git_executable_path).flex_1();

        let exe_row = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(exe_input)
            .child(test_btn);

        let exe_field = v_flex()
            .gap(rems(space::XS))
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.git_executable_path").to_string()),
            )
            .child(exe_row)
            .when_some(self.state.git_test_result.as_ref(), |this, res| {
                let (icon, color, text) = match res {
                    Ok(ver) => (
                        IconName::Check,
                        p.success,
                        rust_i18n::t!("settings.git_test_success", version = ver).to_string(),
                    ),
                    Err(err) => (
                        IconName::CircleAlert,
                        p.danger,
                        rust_i18n::t!("settings.git_test_failed", error = err).to_string(),
                    ),
                };
                this.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(icon).xsmall().text_color(color))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(color)
                                .child(text),
                        ),
                )
            });

        let auto_fetch_current = app_settings(cx).git.auto_fetch_interval;
        let auto_fetch_options = vec![
            (
                0,
                rust_i18n::t!("settings.git_auto_fetch_never").to_string(),
            ),
            (
                5,
                rust_i18n::t!("settings.git_auto_fetch_mins", count = 5).to_string(),
            ),
            (
                15,
                rust_i18n::t!("settings.git_auto_fetch_mins", count = 15).to_string(),
            ),
            (
                30,
                rust_i18n::t!("settings.git_auto_fetch_mins", count = 30).to_string(),
            ),
            (
                60,
                rust_i18n::t!("settings.git_auto_fetch_mins", count = 60).to_string(),
            ),
        ];
        let auto_fetch_dropdown = Self::setting_dropdown(
            "dropdown-git-auto-fetch",
            auto_fetch_current,
            auto_fetch_options,
            |_, val, cx| {
                update_app_settings(cx, |s| s.git.auto_fetch_interval = val);
                cx.notify();
            },
            cx,
        );

        v_flex()
            .gap_2()
            .child(crate::widgets::setting_field_row(
                exe_field.into_any_element(),
                Some(rust_i18n::t!("settings.git_executable_path_desc").to_string()),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_auto_update_on_focus",
                Some("settings.git_auto_update_on_focus_desc"),
                Switch::new("toggle-git-auto-update")
                    .small()
                    .checked(app_settings(cx).git.auto_update_on_focus)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.auto_update_on_focus = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_auto_fetch_interval",
                Some("settings.git_auto_fetch_interval_desc"),
                auto_fetch_dropdown,
                p,
            ))
            .into_any_element()
    }

    /// 提交与推送安全组（提交选项、推送弹窗与保护分支）。
    pub(crate) fn render_settings_git_commit_push(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let push_tags_current = app_settings(cx).git.default_push_tags.clone();
        let push_tags_options = vec![
            (
                "none".to_string(),
                rust_i18n::t!("settings.git_push_tags_none").to_string(),
            ),
            (
                "follow".to_string(),
                rust_i18n::t!("settings.git_push_tags_follow").to_string(),
            ),
            (
                "all".to_string(),
                rust_i18n::t!("settings.git_push_tags_all").to_string(),
            ),
        ];
        let push_tags_dropdown = Self::setting_dropdown(
            "dropdown-git-push-tags",
            push_tags_current,
            push_tags_options,
            |_, val, cx| {
                update_app_settings(cx, |s| s.git.default_push_tags = val);
                cx.notify();
            },
            cx,
        );

        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.git_push_dialog",
                Some("settings.git_push_dialog_desc"),
                Switch::new("toggle-git-push-dialog")
                    .small()
                    .checked(app_settings(cx).git.push_dialog)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.push_dialog = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_clear_message",
                Some("settings.git_clear_message_desc"),
                Switch::new("toggle-git-clear-message")
                    .small()
                    .checked(app_settings(cx).git.clear_message_on_commit)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.clear_message_on_commit = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_warn_uncommitted",
                Some("settings.git_warn_uncommitted_desc"),
                Switch::new("toggle-git-warn-uncommitted")
                    .small()
                    .checked(app_settings(cx).git.warn_uncommitted)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.warn_uncommitted = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_default_push_tags",
                Some("settings.git_default_push_tags_desc"),
                push_tags_dropdown,
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_protected_branches").to_string(),
                    self.state.git_protected_branches.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_protected_branches_desc").to_string()),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_author_name").to_string(),
                    self.state.git_author_name.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_author_name_desc").to_string()),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_author_email").to_string(),
                    self.state.git_author_email.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_author_email_desc").to_string()),
                p,
            ))
            .into_any_element()
    }

    /// 差异对比偏好组（空白忽略与默认视图）。
    pub(crate) fn render_settings_git_diff(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let diff_mode_current = app_settings(cx).git.default_diff_mode.clone();
        let diff_mode_options = vec![
            (
                "unified".to_string(),
                rust_i18n::t!("settings.git_diff_mode_unified").to_string(),
            ),
            (
                "split".to_string(),
                rust_i18n::t!("settings.git_diff_mode_split").to_string(),
            ),
        ];
        let diff_mode_dropdown = Self::setting_dropdown(
            "dropdown-git-diff-mode",
            diff_mode_current,
            diff_mode_options,
            |_, val, cx| {
                update_app_settings(cx, |s| s.git.default_diff_mode = val);
                cx.notify();
            },
            cx,
        );

        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.git_default_diff_mode",
                Some("settings.git_default_diff_mode_desc"),
                diff_mode_dropdown,
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_ignore_whitespace",
                Some("settings.git_ignore_whitespace_desc"),
                Switch::new("toggle-git-ignore-whitespace")
                    .small()
                    .checked(app_settings(cx).git.ignore_whitespace)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.ignore_whitespace = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
