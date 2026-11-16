//! 引擎连接的发现、保存、删除与工作区打开。

use super::*;
use transom_core::{ContainerEndpoint, ContainerEngineKind, ContainerEngineRef, SessionId};

/// 打开（或聚焦）一个引擎连接的工作区标签。
pub(crate) fn open_engine_workspace(
    shell: &mut AppShell,
    profile: ContainerEngineProfile,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(existing) =
        shell
            .state
            .workspace
            .tabs
            .iter()
            .find_map(|tab| match &tab.session.connection {
                Connection::ContainerEngine(existing) if existing.id == profile.id => Some(tab.id),
                _ => None,
            })
    {
        shell.activate_tab(existing, window, cx);
        return;
    }
    let tab_id = TabId::new();
    let session = Session::new(
        profile.display_name(),
        Connection::ContainerEngine(profile.clone()),
    );
    shell.state.workspace.create_group(tab_id, session.id);
    let mut tab = TerminalTab::pending(tab_id, session);
    tab.status = TabStatus::Connecting;
    tab.title = profile.display_name();
    let state = ContainerWorkspaceState::new(profile.clone(), window, cx);
    let command_tx = shell.state.runtime.services.command_tx.clone();
    let workspace = cx.new(|_| ContainerWorkspace::new(state, tab_id, command_tx));
    // 订阅一次并在标签存续期间持有：头部把需要外壳配合的动作发成事件，
    // 处理时按这里捕获的标签 id 定位，避免回读正在派发事件的实体。
    let subscription = cx.subscribe(
        &workspace,
        move |shell, _workspace, event: &ContainerWorkspaceEvent, cx| {
            crate::container::on_container_workspace_event(shell, tab_id, event, cx);
        },
    );
    shell
        .state
        .runtime
        .tasks
        .subscriptions
        .container_workspaces
        .insert(tab_id, subscription);
    tab.content = TabContent::Containers(workspace);
    shell.state.workspace.tabs.push(tab);
    shell.state.workspace.active_tab = Some(tab_id);
    shell.state.present_active_group();
    shell
        .state
        .send_command(ContainerCommand::Connect { tab_id, profile });
    cx.notify();
}

/// 通过已有 SSH 主机会话一键打开或复用其远端 Docker 工作区。
pub(crate) fn open_container_workspace_for_session(
    shell: &mut AppShell,
    session_id: SessionId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(session) = shell
        .state
        .session
        .catalog
        .sessions
        .iter()
        .find(|s| s.id == session_id)
    else {
        return;
    };
    let Connection::Ssh(ssh) = &session.connection else {
        return;
    };

    let (secrets, _) = crate::connection::collect_connection_secrets(
        shell.state.runtime.services.secret_store.as_ref(),
        session,
    );

    let password = secrets.password.map(|p| p.to_string());
    let identity_file = match &ssh.auth {
        transom_core::AuthMethod::PrivateKey { path, .. } => Some(path.display().to_string()),
        _ => None,
    };

    let (use_sudo, sudo_password) = shell
        .state
        .session
        .catalog
        .engines
        .iter()
        .find_map(|p| match &p.engine.endpoint {
            ContainerEndpoint::Ssh {
                host,
                port,
                user,
                use_sudo,
                sudo_password,
                ..
            } if host == &ssh.host
                && *port == (if ssh.port != 22 { Some(ssh.port) } else { None })
                && user.as_deref()
                    == (!ssh.username.is_empty()).then_some(ssh.username.as_str()) =>
            {
                Some((*use_sudo, sudo_password.clone()))
            }
            _ => None,
        })
        .unwrap_or((false, None));

    let endpoint = ContainerEndpoint::Ssh {
        host: ssh.host.clone(),
        port: if ssh.port != 22 { Some(ssh.port) } else { None },
        user: if !ssh.username.is_empty() {
            Some(ssh.username.clone())
        } else {
            None
        },
        identity_file,
        password,
        use_sudo,
        sudo_password,
    };

    let engine = ContainerEngineRef {
        kind: ContainerEngineKind::Docker,
        endpoint,
    };

    // 若已有相同端点的已保存引擎配置则优先复用，否则生成按会话命名的专属临时 Profile。
    let profile = shell
        .state
        .session
        .catalog
        .engines
        .iter()
        .find(|p| p.engine == engine)
        .cloned()
        .unwrap_or_else(|| {
            ContainerEngineProfile::new(format!("{} (Docker)", session.name), engine)
        });

    open_engine_workspace(shell, profile, window, cx);
}

/// 保存引擎连接配置（新建或更新）。
/// 新建连接并放进指定分组（`None` 为根层级）。
pub(crate) fn open_new_container_in_folder(
    shell: &mut AppShell,
    folder: Option<transom_core::FolderId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    crate::container::open_container_form(shell, None, folder, window, cx);
}

pub(crate) fn save_container_profile(
    shell: &mut AppShell,
    profile: ContainerEngineProfile,
    cx: &mut Context<AppShell>,
) {
    if let Some(existing) = shell
        .state
        .session
        .catalog
        .engines
        .iter_mut()
        .find(|existing| existing.id == profile.id)
    {
        *existing = profile;
    } else {
        shell.state.session.catalog.engines.push(profile);
    }
    shell.state.persist_catalog();
    cx.notify();
}

/// 消费引擎连接表单实体上交的保存与删除意图。
///
/// 事件自带完整配置与 id，处理时禁止回读实体：实体 emit 期间仍被租借。
pub(crate) fn on_container_form_event(
    shell: &mut AppShell,
    event: &ContainerFormEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        ContainerFormEvent::Save(profile) => {
            save_container_profile(shell, (**profile).clone(), cx);
        }
        ContainerFormEvent::Delete(id) => {
            delete_container_profile(shell, *id, window, cx);
        }
    }
}

/// 删除引擎连接配置并关闭其工作区标签。
pub(crate) fn delete_container_profile(
    shell: &mut AppShell,
    id: transom_core::EngineProfileId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    shell
        .state
        .session
        .catalog
        .engines
        .retain(|profile| profile.id != id);
    shell.state.persist_catalog();
    let tabs: Vec<TabId> = shell
        .state
        .workspace
        .tabs
        .iter()
        .filter_map(|tab| match &tab.session.connection {
            Connection::ContainerEngine(profile) if profile.id == id => Some(tab.id),
            _ => None,
        })
        .collect();
    for tab_id in tabs {
        shell.close_tab(tab_id, window, cx);
    }
    cx.notify();
}
