//! AI 工作组摘要文案生成与分组单元测试。

use transom_ui_icons::{AppIcon, DomainIcon, UiIcon};

use super::state::UiChatItem;

pub(super) fn summarize_work_group_compact(items: &[UiChatItem]) -> String {
    let mut cmd_count = 0usize;
    let mut file_count = 0usize;
    let mut sub_count = 0usize;
    let mut hosts: Vec<&str> = Vec::new();

    for item in items {
        if let UiChatItem::ToolCard {
            tool_name,
            host_label,
            ..
        } = item
        {
            match tool_name.as_str() {
                "remote_exec" | "remote_exec_bulk" | "container_ops" => cmd_count += 1,
                "remote_fs" | "transfer_file" | "read_output" => file_count += 1,
                "delegate_task" => sub_count += 1,
                _ => {}
            }
            let h = host_label.trim();
            if !h.is_empty()
                && h != rust_i18n::t!("ai.scope_global_label").as_ref()
                && !hosts.contains(&h)
            {
                hosts.push(h);
            }
        }
    }

    let mut parts = Vec::new();
    if file_count > 0 {
        parts.push(rust_i18n::t!("ai.wg_file_ops", count = file_count).to_string());
    }
    if cmd_count > 0 {
        parts.push(rust_i18n::t!("ai.wg_cmd_ops", count = cmd_count).to_string());
    }
    if sub_count > 0 {
        parts.push(rust_i18n::t!("ai.wg_subtasks", count = sub_count).to_string());
    }
    if !hosts.is_empty() {
        let host_text = if hosts.len() <= 2 {
            hosts.join(", ")
        } else {
            rust_i18n::t!(
                "ai.wg_hosts_more",
                h1 = hosts[0],
                h2 = hosts[1],
                total = hosts.len()
            )
            .to_string()
        };
        parts.push(host_text);
    }
    parts.join(" · ")
}

pub(super) fn summarize_tool_activity(items: &[UiChatItem]) -> Option<(AppIcon, String)> {
    let mut has_cmd = false;
    let mut has_file = false;
    for item in items {
        if let UiChatItem::ToolCard { tool_name, .. } = item {
            match tool_name.as_str() {
                "remote_exec" | "remote_exec_bulk" | "container_ops" => has_cmd = true,
                "remote_fs" | "transfer_file" | "read_output" => has_file = true,
                _ => {}
            }
        }
    }
    match (has_file, has_cmd) {
        (true, true) => Some((
            AppIcon::Ui(UiIcon::LIST_FILES),
            rust_i18n::t!("ai.wg_read_and_ran").to_string(),
        )),
        (true, false) => Some((
            AppIcon::Ui(UiIcon::LIST_FILES),
            rust_i18n::t!("ai.wg_read_files").to_string(),
        )),
        (false, true) => Some((
            AppIcon::Domain(DomainIcon::TERMINAL),
            rust_i18n::t!("ai.wg_ran_commands").to_string(),
        )),
        (false, false) => None,
    }
}
