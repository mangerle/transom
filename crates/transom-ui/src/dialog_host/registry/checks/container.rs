//! 容器类弹窗的判定与文本项。
use crate::app::AppShell;
use gpui_kit::App;

pub(in crate::app::dialog_host) fn container_form_open(shell: &AppShell, _cx: &App) -> bool {
    shell.state.containers.form.is_some()
}

pub(in crate::app::dialog_host) fn container_form_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .state
        .containers
        .form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_folder_form_open(shell: &AppShell, _cx: &App) -> bool {
    shell.state.containers.engine_folder_form.is_some()
}

pub(in crate::app::dialog_host) fn container_folder_form_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .state
        .containers
        .engine_folder_form
        .as_ref()
        .map(|form| vec![form.input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_run_form_open(shell: &AppShell, _cx: &App) -> bool {
    shell.state.containers.run_form.is_some()
}

pub(in crate::app::dialog_host) fn container_run_form_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .state
        .containers
        .run_form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_create_open(shell: &AppShell, _cx: &App) -> bool {
    shell.state.containers.create_dialog.is_some()
}

pub(in crate::app::dialog_host) fn container_create_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .state
        .containers
        .create_dialog
        .as_ref()
        .map(|dialog| {
            let mut values = Vec::new();
            if let Some(workspace) = shell.state.container_workspace(dialog.tab_id) {
                values.push(
                    workspace
                        .read(cx)
                        .state
                        .create_input
                        .read(cx)
                        .value()
                        .to_string(),
                );
            }
            for input in [&dialog.driver, &dialog.subnet, &dialog.gateway]
                .into_iter()
                .flatten()
            {
                values.push(input.read(cx).value().to_string());
            }
            values
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_confirm_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.confirm.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_rename_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_rename_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.rename_container.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.rename_container.as_ref())
        .map(|(_, input)| vec![input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_tag_image_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_tag_image_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.tag_image.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.tag_image.as_ref())
        .map(|(_, input)| vec![input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_commit_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_commit_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.commit_container.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.commit_container.as_ref())
        .map(|state| {
            vec![
                state.repo.read(cx).value().to_string(),
                state.tag.read(cx).value().to_string(),
                state.comment.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_update_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.update_container.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_update_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.update_container.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.update_container.as_ref())
        .map(|(_, mem, cpu)| {
            vec![
                mem.read(cx).value().to_string(),
                cpu.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_network_connect_open(
    shell: &AppShell,
    cx: &App,
) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_network_connect_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.connect_network.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.connect_network.as_ref())
        .map(|(_, input)| vec![input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_export_config_open(
    shell: &AppShell,
    cx: &App,
) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.export_config.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_file_rename_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.files.prompt_input.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_file_rename_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.files.prompt_input.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.files.prompt_input.as_ref())
        .map(|input| vec![input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_file_new_dir_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| {
            ws.files.new_dir_open && ws.files.new_dir_input.is_some()
        })
        .is_some()
}

pub(in crate::app::dialog_host) fn container_file_new_dir_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.files.new_dir_input.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.files.new_dir_input.as_ref())
        .map(|input| vec![input.read(cx).value().to_string()])
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_hub_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.hub_dialog.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_push_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_push_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.push_dialog.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.push_dialog.as_ref())
        .map(|push| {
            vec![
                push.target_repo.read(cx).value().to_string(),
                push.target_tag.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_system_df_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.system_df_dialog_open)
        .is_some()
}

pub(in crate::app::dialog_host) fn container_build_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_build_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.build_dialog.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.build_dialog.as_ref())
        .map(|build| {
            vec![
                build.context_path.read(cx).value().to_string(),
                build.dockerfile.read(cx).value().to_string(),
                build.tag.read(cx).value().to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_forward_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
        .is_some()
}

pub(in crate::app::dialog_host) fn container_forward_values(
    shell: &AppShell,
    cx: &App,
) -> Vec<String> {
    shell
        .find_container_modal_tab(cx, |ws| ws.forward_dialog.is_some())
        .and_then(|(_, ws)| ws.read(cx).state.forward_dialog.as_ref())
        .map(|fwd| {
            vec![
                fwd.local_port_input.read(cx).value().to_string(),
                fwd.target_host_input.read(cx).value().to_string(),
                fwd.selected_ssh_index.to_string(),
            ]
        })
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn container_pods_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.pods_dialog_open)
        .is_some()
}

pub(in crate::app::dialog_host) fn container_quadlet_open(shell: &AppShell, cx: &App) -> bool {
    shell
        .find_container_modal_tab(cx, |ws| ws.quadlets_dialog_open)
        .is_some()
}
