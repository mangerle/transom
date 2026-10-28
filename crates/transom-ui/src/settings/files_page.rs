//! 偏好设置中心之文件与传输配置页（内置编辑器、文件面板排除、传输队列与 ZMODEM 接收）。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{ChecksumAlgorithm, VerifyMode};

impl AppShell {
    /// 编辑器组：软换行与行号显示开关。
    pub(crate) fn render_settings_editor(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.editor_soft_wrap",
                Some("settings.editor_soft_wrap_desc"),
                Switch::new("toggle-editor-soft-wrap")
                    .small()
                    .checked(self.settings.sftp.editor_soft_wrap)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.sftp.editor_soft_wrap = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.editor_line_numbers",
                Some("settings.editor_line_numbers_desc"),
                Switch::new("toggle-editor-line-numbers")
                    .small()
                    .checked(self.settings.sftp.editor_line_numbers)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.sftp.editor_line_numbers = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 文件面板组：显示排除规则（只影响可见性，不拦截传输）。
    pub(crate) fn render_settings_file_panel(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.panel_hidden").to_string(),
                self.modals.settings.panel_hidden_patterns.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.panel_hidden_note").to_string()),
            )
            .into_any_element()
    }

    /// 传输组：并发上限、完成后校验与校验算法。
    pub(crate) fn render_settings_transfer(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_algo = self.settings.sftp.checksum_algorithm;
        let current_mode = self.settings.sftp.verify_mode;
        let stepper =
            Self::setting_stepper(&self.modals.settings.max_concurrent_transfers, px(110.0));
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.max_concurrent_transfers",
                Some("settings.max_concurrent_transfers_desc"),
                stepper,
                p,
            ))
            .child(Self::setting_row(
                "settings.verify_after_transfer",
                Some("settings.verify_after_transfer_desc"),
                Switch::new("toggle-verify-after-transfer")
                    .small()
                    .checked(self.settings.sftp.verify_after_transfer)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.sftp.verify_after_transfer = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.checksum_algorithm",
                Some("settings.checksum_algorithm_desc"),
                Self::setting_dropdown(
                    "dropdown-checksum-algo",
                    current_algo,
                    vec![
                        (
                            ChecksumAlgorithm::Sha256,
                            rust_i18n::t!("settings.checksum_sha256").to_string(),
                        ),
                        (
                            ChecksumAlgorithm::Md5,
                            rust_i18n::t!("settings.checksum_md5").to_string(),
                        ),
                    ],
                    |this, algo, cx| {
                        this.settings.sftp.checksum_algorithm = algo;
                        cx.notify();
                    },
                    cx,
                ),
                p,
            ))
            .child(Self::setting_row(
                "settings.verify_mode",
                Some("settings.verify_mode_desc"),
                Self::setting_dropdown(
                    "dropdown-verify-mode",
                    current_mode,
                    vec![
                        (
                            VerifyMode::FullSha256,
                            rust_i18n::t!("settings.verify_mode_full").to_string(),
                        ),
                        (
                            VerifyMode::QuickSample,
                            rust_i18n::t!("settings.verify_mode_quick").to_string(),
                        ),
                    ],
                    |this, mode, cx| {
                        this.settings.sftp.verify_mode = mode;
                        cx.notify();
                    },
                    cx,
                ),
                p,
            ))
            .into_any_element()
    }

    /// ZMODEM 组：下载目录与自动接收开关。
    ///
    /// 与其余设置一致采用"即时改内存、保存时统一落盘"；下载目录为空表示
    /// 跟随系统下载目录，选择后写入显式路径。
    pub(crate) fn render_settings_zmodem(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resolved = self.settings.zmodem.resolved_download_dir();
        let dir_label = match self.settings.zmodem.download_dir.clone() {
            Some(dir) => dir.display().to_string(),
            None => format!(
                "{}（{}）",
                rust_i18n::t!("settings.zmodem_dir_default"),
                resolved.display()
            ),
        };
        let browse = cx.listener(|this, _, _, _cx| {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                this.settings.zmodem.download_dir = Some(dir);
            }
        });
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.zmodem_dir",
                Some("settings.zmodem_dir_desc"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Self::setting_path_value("zmodem-dir", dir_label, p))
                    .child(
                        Button::new("zmodem-dir-browse")
                            .small()
                            .label(rust_i18n::t!("settings.zmodem_dir_browse").to_string())
                            .on_click(browse),
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.zmodem_auto_receive",
                Some("settings.zmodem_auto_receive_desc"),
                Switch::new("toggle-zmodem-auto-receive")
                    .small()
                    .checked(self.settings.zmodem.auto_receive)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.zmodem.auto_receive = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
