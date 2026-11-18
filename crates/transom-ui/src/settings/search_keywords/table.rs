//! 分组关键词表：每个设置分组包含哪些可搜索的文案键。
//!
//! 单独成文件只为控制单文件长度，内容与 [`super::GroupSearch`] 同属一个实体。

use super::GroupSearch;

impl GroupSearch {
    /// 分组标题键与组内行文案键的唯一来源。
    ///
    /// 声明式映射表：一臂一组，行数较多但都是数据，不拆函数。
    pub(super) fn entry(self) -> (&'static str, &'static [&'static str]) {
        match self {
            GroupSearch::UiTheme => (
                "settings.ui_theme_group",
                &[
                    "settings.ui_mode",
                    "settings.ui_mode_system",
                    "settings.ui_mode_light",
                    "settings.ui_mode_dark",
                    "settings.density",
                    "settings.ui_theme_note",
                    "settings.appearance_contrast",
                    "settings.theme_mode_dark",
                    "settings.theme_mode_light",
                ],
            ),
            GroupSearch::ThemeEditor => (
                "settings.theme_editor_group",
                &[
                    "settings.theme_editor_note",
                    "settings.theme_editor_load_active",
                    "settings.theme_editor_save",
                    "settings.theme_editor_import",
                    "settings.theme_editor_export",
                ],
            ),
            GroupSearch::UiFont => (
                "settings.ui_font_group",
                &[
                    "settings.ui_font_family",
                    "settings.ui_font_note",
                    "settings.font_cjk_unsupported",
                ],
            ),
            GroupSearch::WindowAppearance => (
                "settings.window_appearance_group",
                &[
                    "settings.window_background",
                    "settings.window_background_desc",
                    "settings.window_transparency",
                    "settings.window_transparency_desc",
                ],
            ),
            GroupSearch::Divider => (
                "settings.divider_group",
                &[
                    "settings.divider_color",
                    "settings.divider_color_desc",
                    "settings.divider_width",
                    "settings.divider_width_desc",
                ],
            ),
            GroupSearch::TerminalFont => (
                "settings.terminal_font_group",
                &[
                    "settings.font_size",
                    "settings.line_height",
                    "settings.font_family",
                    "settings.cjk_font",
                    "settings.cjk_font_note",
                    "settings.font_cjk_unsupported",
                ],
            ),
            GroupSearch::LocalShell => (
                "settings.local_shell_group",
                &[
                    "settings.local_shell",
                    "settings.local_shell_desc",
                    "settings.local_shell_system",
                    "settings.local_shell_custom",
                    "settings.local_shell_custom_path",
                    "settings.local_shell_custom_path_desc",
                    "settings.local_shell_custom_hint",
                    "settings.local_shell_custom_required",
                    "settings.local_shell_missing",
                    "settings.local_shell_admin",
                    "settings.local_shell_admin_desc",
                ],
            ),
            GroupSearch::Cursor => (
                "settings.cursor_group",
                &[
                    "settings.cursor_shape",
                    "settings.cursor_shape_desc",
                    "settings.cursor_blink",
                    "settings.cursor_blink_desc",
                    "settings.bold_is_bright",
                    "settings.bold_is_bright_desc",
                    "settings.alternate_scroll",
                    "settings.alternate_scroll_desc",
                    "settings.scroll_on_input",
                    "settings.scroll_on_input_desc",
                    "settings.scrollback",
                ],
            ),
            GroupSearch::TerminalTheme => (
                "settings.terminal_theme_group",
                &[
                    "settings.terminal_follow_ui",
                    "settings.terminal_follow_ui_note",
                    "settings.terminal_theme_note",
                    "settings.terminal_minimum_contrast",
                    "settings.terminal_minimum_contrast_desc",
                    "settings.terminal_contrast_off",
                    "settings.terminal_contrast_minimum",
                    "settings.terminal_contrast_aa",
                    "settings.terminal_contrast_aaa",
                    "settings.terminal_contrast_maximum",
                ],
            ),
            GroupSearch::Highlight => (
                "settings.highlight_group",
                &[
                    "settings.keyword_highlight",
                    "settings.keyword_highlight_desc",
                    "settings.highlight_keywords",
                    "settings.highlight_keywords_note",
                ],
            ),
            GroupSearch::Trigger => (
                "settings.trigger_group",
                &[
                    "trigger.master_switch",
                    "trigger.master_switch_desc",
                    "trigger.rules_list",
                    "trigger.add_rule",
                    "trigger.rule_name",
                    "trigger.rule_pattern",
                    "trigger.action_kind",
                    "trigger.case_sensitive",
                ],
            ),
            GroupSearch::BlockView => (
                "settings.block_view_group",
                &[
                    "settings.block_view_enable",
                    "settings.block_view_enable_desc",
                    "settings.shell_integration_enable",
                    "settings.shell_integration_enable_desc",
                    "settings.block_view_style",
                    "settings.block_view_rows",
                    "settings.block_view_rows_hint",
                ],
            ),
            GroupSearch::KeyBindings => (
                "settings.key_bindings_group",
                &["settings.key_bindings_desc", "settings.key_bindings_hint"],
            ),
            GroupSearch::Editor => (
                "settings.editor_group",
                &[
                    "settings.editor_soft_wrap",
                    "settings.editor_soft_wrap_desc",
                    "settings.editor_line_numbers",
                    "settings.editor_line_numbers_desc",
                ],
            ),
            GroupSearch::FilePanel => (
                "settings.file_panel_group",
                &["settings.panel_hidden", "settings.panel_hidden_note"],
            ),
            GroupSearch::Transfer => (
                "settings.transfer_group",
                &[
                    "settings.max_concurrent_transfers",
                    "settings.max_concurrent_transfers_desc",
                    "settings.verify_after_transfer",
                    "settings.verify_after_transfer_desc",
                    "settings.checksum_algorithm",
                    "settings.checksum_algorithm_desc",
                    "settings.verify_mode",
                    "settings.verify_mode_desc",
                ],
            ),
            GroupSearch::Zmodem => (
                "settings.zmodem_group",
                &[
                    "settings.zmodem_dir",
                    "settings.zmodem_dir_desc",
                    "settings.zmodem_dir_default",
                    "settings.zmodem_auto_receive",
                    "settings.zmodem_auto_receive_desc",
                ],
            ),
            GroupSearch::Audit => (
                "settings.audit_group",
                &[
                    "settings.audit_enabled",
                    "settings.audit_enabled_desc",
                    "settings.audit_dir",
                    "settings.audit_dir_desc",
                    "settings.audit_dir_open",
                ],
            ),
            GroupSearch::Identities => (
                "settings.identity_group",
                &["identity.new", "identity.empty", "identity.used_by"],
            ),
            GroupSearch::History => (
                "settings.history_group",
                &["settings.history_enabled", "settings.history_enabled_desc"],
            ),
            GroupSearch::Mouse => (
                "settings.mouse_group",
                &[
                    "settings.right_click_action",
                    "settings.right_click_action_desc",
                    "settings.right_click_paste_action",
                    "settings.right_click_menu",
                    "settings.right_click_menu_paste",
                    "settings.middle_click_paste",
                    "settings.middle_click_paste_desc",
                ],
            ),
            GroupSearch::Selection => (
                "settings.selection_group",
                &["settings.word_separators", "settings.word_separators_note"],
            ),
            GroupSearch::Clipboard => (
                "settings.clipboard_group",
                &[
                    "settings.copy_on_select",
                    "settings.copy_on_select_desc",
                    "settings.default_allow_remote_clipboard_write",
                    "settings.default_allow_remote_clipboard_write_desc",
                    "settings.multiline_paste",
                    "settings.multiline_paste_desc",
                    "settings.trim_trailing_whitespace",
                    "settings.trim_trailing_whitespace_desc",
                    "settings.suppress_ctrl_v",
                    "settings.suppress_ctrl_v_desc",
                ],
            ),
            GroupSearch::Shortcuts => (
                "settings.tab_shortcuts",
                &[
                    "settings.shortcut_action",
                    "settings.shortcut_keys",
                    "settings.shortcut_edit",
                    "settings.shortcut_reset",
                    "settings.shortcut_reset_all",
                    "settings.shortcut_conflicts",
                    "settings.shortcut_hint",
                ],
            ),
            GroupSearch::Tabs => (
                "settings.tabs_group",
                &[
                    "settings.show_tab_close_button",
                    "settings.show_tab_close_button_desc",
                    "settings.close_window_on_all_tabs_closed",
                    "settings.close_window_on_all_tabs_closed_desc",
                    "settings.tab_position",
                    "settings.tab_position_desc",
                ],
            ),
            GroupSearch::Window => (
                "settings.window_group",
                &[
                    "settings.sftp_panel_side",
                    "settings.sftp_panel_side_desc",
                    "settings.remember_window_bounds",
                    "settings.remember_window_bounds_desc",
                ],
            ),
            GroupSearch::ActivityBar => super::table_workspace::ACTIVITY_BAR,
            GroupSearch::StatusBar => super::table_workspace::STATUS_BAR,
            GroupSearch::Startup => super::table_workspace::STARTUP,
            GroupSearch::Language => super::table_workspace::LANGUAGE,
            GroupSearch::Notify => super::table_workspace::NOTIFY,
            GroupSearch::Proxy => super::table_workspace::PROXY,
            #[cfg(target_os = "linux")]
            GroupSearch::DesktopIntegration => super::table_workspace::DESKTOP_INTEGRATION,
            GroupSearch::Backup => super::table_workspace::BACKUP,
            GroupSearch::Sync => super::table_workspace::SYNC,
            GroupSearch::Logs => super::table_workspace::LOGS,
            GroupSearch::MonitorSections => super::table_workspace::MONITOR_SECTIONS,
            GroupSearch::MonitorBehavior => super::table_workspace::MONITOR_BEHAVIOR,
            GroupSearch::About => super::table_workspace::ABOUT,
            GroupSearch::Update => super::table_workspace::UPDATE,
            GroupSearch::ContainerRegistries => super::table_workspace::CONTAINER_REGISTRIES,
            GroupSearch::GitExecutable => super::table_workspace::GIT_EXECUTABLE,
            GroupSearch::GitCommitPush => super::table_workspace::GIT_COMMIT_PUSH,
            GroupSearch::GitDiff => super::table_workspace::GIT_DIFF,
        }
    }
}
