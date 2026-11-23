//! 标签模型边界守卫。

use crate::common::walkdir_like;
use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("读取 {relative} 失败: {error}"))
}

#[test]
fn workspace_tab_has_no_session_fields() {
    let model = read("src/app/tab/model.rs");
    let start = model
        .find("pub(crate) struct WorkspaceTab {")
        .expect("未找到 WorkspaceTab 定义");
    let end = model[start..].find("\n}").expect("WorkspaceTab 定义未闭合") + start;
    let body = &model[start..end];
    for forbidden in ["session_id:", "session:", "connection_id:", "cwd:"] {
        assert!(
            !body.contains(forbidden),
            "公共 WorkspaceTab 不得包含会话字段 {forbidden}"
        );
    }
}

#[test]
fn tab_group_uses_typed_owner_not_session_id() {
    let group = read("src/app/tab/group.rs");
    assert!(group.contains("owner: TabGroupOwner"));
    assert!(!group.contains("session_id: transom_core::SessionId"));
}

#[test]
fn legacy_terminal_tab_is_removed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let offenders: Vec<String> = walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            content
                .contains("TerminalTab")
                .then(|| path.display().to_string())
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "旧 TerminalTab 不得复活：\n{}",
        offenders.join("\n")
    );
}

#[test]
fn non_session_tabs_do_not_construct_placeholder_sessions() {
    let git = read("src/git/actions/mod.rs");
    let player = read("src/record/ops.rs");
    assert!(!git.contains("git_placeholder_session"));
    assert!(!git.contains("Connection::Local"));
    assert!(!player.contains("dummy_session"));
}

#[test]
fn container_tabs_use_typed_content() {
    let engines = read("src/container/engines.rs");
    assert!(engines.contains("WorkspaceTab::new_container"));
    assert!(engines.contains("create_group_for_tab"));
}

#[test]
fn session_fields_are_accessed_through_typed_accessors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "tab.session.",
        "tab.session_id ==",
        "tab.session_id !=",
        "tab.connection_id ==",
        "tab.connection_id !=",
        "tab.cwd.",
    ];
    let offenders: Vec<String> = walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            let hits: Vec<&str> = forbidden
                .into_iter()
                .filter(|pattern| content.contains(pattern))
                .collect();
            (!hits.is_empty()).then(|| format!("{}: {}", path.display(), hits.join(", ")))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "标签会话字段必须通过访问器读取：\n{}",
        offenders.join("\n")
    );
}

#[test]
fn window_state_saves_only_session_tabs() {
    let state = read("src/window_state.rs");
    assert!(state.contains("filter_map(session_key)"));
    assert!(state.contains("session_key(tab: &WorkspaceTab) -> Option<String>"));
}

#[test]
fn tab_close_actions_are_type_specific() {
    let model = read("src/app/tab/model.rs");
    let close = read("src/app/tab_coordination.rs");
    assert!(model.contains("Self::CloseTerminal"));
    assert!(model.contains("Self::DisconnectContainer"));
    assert!(model.contains("Self::DropOnly"));
    assert!(close.contains("TabCloseAction::CloseTerminal"));
    assert!(close.contains("ContainerCommand::Disconnect"));
    assert!(close.contains("TabCloseAction::DropOnly"));
}

#[test]
fn tab_presentation_same_for_horizontal_and_vertical() {
    let horizontal = read("src/workspace/tab_strip/item.rs");
    let vertical = read("src/workspace/vertical_tabs/item.rs");
    let presentation = read("src/app/tab/presentation.rs");
    assert!(
        presentation.contains("pub(crate) struct TabPresentation")
            && presentation.contains("pub(crate) fn presentation("),
        "必须存在统一的 TabPresentation 生成入口"
    );
    assert!(
        horizontal.contains("tab.presentation(") && vertical.contains("tab.presentation("),
        "横向标签与纵向标签必须共享 WorkspaceTab::presentation"
    );
}

#[test]
fn api_client_tabs_use_typed_content() {
    let model = read("src/app/tab/model.rs");
    let tab = read("src/app/tab/api_client.rs");
    assert!(
        model.contains("ApiClient(Box<ApiClientTab>)"),
        "API 工作区必须使用类型化 TabContent，不得伪造会话标签"
    );
    assert!(
        tab.contains("fn new_api_client")
            && tab.contains("ApiProjectId")
            && tab.contains("Entity<ApiWorkspace>"),
        "API 标签必须持有项目 id 与工作区实体"
    );
}
