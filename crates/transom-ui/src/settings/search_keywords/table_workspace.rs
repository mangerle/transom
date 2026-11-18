//! 设置页工作区与 Git 分组的搜索关键字静态表。

pub(super) const ACTIVITY_BAR: (&str, &[&str]) = (
    "settings.activity_bar",
    &[
        "activity.restore_default",
        "app.sessions",
        "activity.tabs",
        "app.quick_open",
        "forward.title",
        "sftp.title",
        "monitor.title",
        "activity.language",
        "app.import_ssh_config",
        "known_hosts.title",
        "keys.title",
        "settings.title",
    ],
);

pub(super) const STATUS_BAR: (&str, &[&str]) = (
    "settings.status_bar",
    &[
        "activity.restore_default",
        "status.network",
        "monitor.cpu",
        "monitor.mem",
        "monitor.disk",
        "status.os",
        "monitor.uptime",
    ],
);

pub(super) const STARTUP: (&str, &[&str]) = (
    "settings.startup_group",
    &[
        "settings.startup_tabs",
        "settings.startup_tabs_desc",
        "settings.sidebar_startup",
        "settings.sidebar_startup_desc",
    ],
);

pub(super) const LANGUAGE: (&str, &[&str]) = (
    "settings.language_group",
    &[
        "app.language",
        "settings.language_desc",
        "settings.language_system",
    ],
);

pub(super) const NOTIFY: (&str, &[&str]) = (
    "settings.notify_group",
    &[
        "settings.bell_style",
        "settings.bell_style_desc",
        "settings.desktop_notifications_desc",
        "notify.desktop_switch",
        "notify.threshold",
        "notify.threshold_note",
    ],
);

pub(super) const PROXY: (&str, &[&str]) = (
    "settings.proxy_group",
    &[
        "settings.proxy_mode",
        "settings.proxy_mode_desc",
        "settings.proxy_host",
        "settings.proxy_port",
        "settings.proxy_username",
        "settings.proxy_password",
        "settings.proxy_auth_hint",
        "form.proxy_direct",
        "form.proxy_http",
        "form.proxy_socks5",
    ],
);

#[cfg(target_os = "linux")]
pub(super) const DESKTOP_INTEGRATION: (&str, &[&str]) = (
    "settings.desktop_integration_group",
    &[
        "desktop.default_terminal_switch",
        "desktop.default_terminal_desc",
    ],
);

pub(super) const BACKUP: (&str, &[&str]) = (
    "settings.backup_group",
    &[
        "backup.settings_hint",
        "backup.export_action",
        "backup.import_action",
    ],
);

pub(super) const SYNC: (&str, &[&str]) = (
    "settings.sync_group",
    &[
        "sync.enabled",
        "sync.enabled_desc",
        "sync.endpoint",
        "sync.endpoint_desc",
        "sync.device_name",
        "sync.device_name_hint",
        "sync.webdav_url",
        "sync.s3_endpoint",
        "sync.session_token",
        "sync.virtual_host_style",
        "sync.virtual_host_style_desc",
        "sync.git_remote",
        "sync.directory",
        "sync.git_sync_now",
        "sync.push",
        "sync.pull",
        "sync.include_secrets",
        "sync.include_secrets_desc",
        "sync.git_interval",
        "sync.check",
        "sync.test",
        "sync.auto_check_on_startup",
        "sync.auto_push_on_change",
        "sync.auto_pull_remote_changes",
        "sync.auto_push_debounce",
    ],
);

pub(super) const LOGS: (&str, &[&str]) = (
    "settings.log_group",
    &[
        "settings.log_level",
        "settings.log_level_desc",
        "settings.log_dir",
        "settings.log_dir_desc",
        "settings.log_dir_open",
        "settings.log_cleanup",
        "settings.log_cleanup_desc",
        "settings.log_cleanup_action",
    ],
);

pub(super) const MONITOR_SECTIONS: (&str, &[&str]) = (
    "settings.monitor_sections_group",
    &[
        "settings.monitor_cpu_expanded",
        "settings.monitor_cpu_expanded_desc",
        "settings.monitor_memory_expanded",
        "settings.monitor_memory_expanded_desc",
        "settings.monitor_disk_expanded",
        "settings.monitor_disk_expanded_desc",
        "settings.monitor_network_expanded",
        "settings.monitor_network_expanded_desc",
        "settings.monitor_ports_expanded",
        "settings.monitor_ports_expanded_desc",
        "settings.monitor_services_expanded",
        "settings.monitor_services_expanded_desc",
        "settings.monitor_processes_expanded",
        "settings.monitor_processes_expanded_desc",
    ],
);

pub(super) const MONITOR_BEHAVIOR: (&str, &[&str]) = (
    "settings.monitor_behavior_group",
    &[
        "settings.monitor_default_width",
        "settings.monitor_default_width_desc",
        "settings.monitor_default_interval",
        "settings.monitor_default_interval_desc",
    ],
);

pub(super) const ABOUT: (&str, &[&str]) = (
    "settings.about_group",
    &[
        "settings.about_version",
        "settings.about_build",
        "settings.about_license",
        "settings.about_author",
        "settings.about_platform",
        "settings.about_homepage",
        "settings.about_config_dir",
        "settings.about_summary",
    ],
);

pub(super) const UPDATE: (&str, &[&str]) = (
    "settings.update_group",
    &[
        "settings.update_title",
        "settings.update_desc",
        "settings.update_check_action",
        "settings.update_download_action",
        "settings.update_install_action",
        "settings.update_restart_action",
        "settings.update_notes_title",
    ],
);

pub(super) const CONTAINER_REGISTRIES: (&str, &[&str]) = (
    "settings.container_registries_group",
    &[
        "settings.container_registries_desc",
        "settings.add_registry",
        "settings.edit_registry",
        "settings.registry_name",
        "settings.registry_url",
        "settings.registry_username",
        "settings.registry_default",
    ],
);

pub(super) const GIT_EXECUTABLE: (&str, &[&str]) = (
    "settings.git_executable_group",
    &[
        "settings.git_executable_path",
        "settings.git_executable_path_desc",
        "settings.git_test_executable",
        "settings.git_auto_update_on_focus",
        "settings.git_auto_update_on_focus_desc",
        "settings.git_auto_fetch_interval",
        "settings.git_auto_fetch_interval_desc",
    ],
);

pub(super) const GIT_COMMIT_PUSH: (&str, &[&str]) = (
    "settings.git_commit_push_group",
    &[
        "settings.git_push_dialog",
        "settings.git_push_dialog_desc",
        "settings.git_clear_message",
        "settings.git_clear_message_desc",
        "settings.git_warn_uncommitted",
        "settings.git_warn_uncommitted_desc",
        "settings.git_default_push_tags",
        "settings.git_default_push_tags_desc",
        "settings.git_protected_branches",
        "settings.git_protected_branches_desc",
        "settings.git_author_name",
        "settings.git_author_name_desc",
        "settings.git_author_email",
        "settings.git_author_email_desc",
    ],
);

pub(super) const GIT_DIFF: (&str, &[&str]) = (
    "settings.git_diff_group",
    &[
        "settings.git_default_diff_mode",
        "settings.git_default_diff_mode_desc",
        "settings.git_ignore_whitespace",
        "settings.git_ignore_whitespace_desc",
    ],
);
