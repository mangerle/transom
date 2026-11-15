//! 容器与 Git 实体边界、命令总线与迁移防回退守卫。

use crate::common::{impl_blocks, walkdir_like};
#[test]
fn container_form_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("container_form: Entity<ContainerFormView>"),
        "ModalHub 的容器连接表单必须持有 Entity<ContainerFormView>"
    );
    let entity = std::fs::read_to_string(root.join("src/container/form/entity.rs"))
        .expect("读取容器连接表单实体失败");
    assert!(
        entity.contains("pub(crate) struct ContainerFormView"),
        "ContainerFormView 实体必须存在于 container/form/entity.rs"
    );
    let dialogs = std::fs::read_to_string(root.join("src/container/types/dialogs.rs"))
        .expect("读取容器界面状态失败");
    assert!(
        !dialogs.contains("form: Option<ContainerFormState>"),
        "ContainerUiState 不得重新持有连接表单状态"
    );
    assert!(
        !root.join("src/container/form/engine_actions.rs").exists(),
        "旧的外壳动作文件 engine_actions.rs 不得复活"
    );
    let mut offenders = Vec::new();
    for file in [
        "src/container/form/appearance.rs",
        "src/container/form/connection.rs",
        "src/container/form/render.rs",
        "src/container/form/entity.rs",
    ] {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content.contains("impl AppShell {") || content.contains("Context<AppShell>") {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "容器连接表单的旧外壳渲染/动作不得复活，跨域数据走 ContainerFormEvent：\n{}",
        offenders.join("\n")
    );
    let connection_events =
        std::fs::read_to_string(root.join("src/container/events/connection.rs"))
            .expect("读取容器连接事件失败");
    assert!(
        connection_events.contains("set_test_result"),
        "测试连接结果必须按 request_id 回写 ContainerFormView"
    );
}

/// 搁置项必须走控制平面：UI 不得直接构造 git 进程或读写搁置项存储。
#[test]
fn shelf_operations_use_command_bus() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git/actions/commit_actions.rs");
    let shelf = std::fs::read_to_string(&path).expect("读取 git/actions/commit_actions.rs 失败");
    for forbidden in ["transom_git::Repo", "transom_store::"] {
        assert!(
            !shelf.contains(forbidden),
            "Shelf 操作不得在 UI 直接使用 {forbidden}，请改走 GitCommand"
        );
    }
    for command in [
        "GitCommand::LoadShelves",
        "GitCommand::Shelve",
        "GitCommand::ApplyShelf",
        "GitCommand::DropShelf",
    ] {
        assert!(shelf.contains(command), "Shelf 入口必须投递 {command}");
    }
}

/// Git 进程级配置只能由控制平面写入：界面不得再直接调用 set_git_*。
#[test]
fn git_config_is_controller_owned() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for pattern in [
            "transom_git::set_git_program",
            "transom_git::set_git_author",
        ] {
            if content.contains(pattern) {
                offenders.push(format!("{}: {pattern}", entry.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "界面不得直接写 Git 全局配置，请投递 GitCommand::Configure：\n{}",
        offenders.join("\n")
    );
}

/// 测试模块声明不能把其后的生产代码一起隐藏：`mod tests;` 是外部测试模块，
/// 只应屏蔽这一行，后面的硬编码中文仍必须被扫描到。

#[test]
fn migrated_container_render_slices_do_not_return_to_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = [
        "src/container/pull_drawer.rs",
        "src/container/view/header.rs",
        "src/container/list/render.rs",
        "src/container/list/filters.rs",
        "src/container/detail/overview.rs",
        "src/container/detail/overview_cards/mod.rs",
        "src/container/detail/logs.rs",
        "src/container/detail/network.rs",
        "src/container/detail/volume.rs",
        "src/container/detail/image.rs",
        "src/container/detail/image_history.rs",
        "src/container/detail/raw_inspect.rs",
        "src/container/detail/mod.rs",
        "src/container/detail/header.rs",
        "src/container/detail/tab_strip.rs",
        "src/container/detail/header_container.rs",
        "src/container/detail/changes.rs",
        "src/container/detail/confirm.rs",
        "src/container/detail/compose.rs",
        "src/container/detail/compose_topology.rs",
        "src/container/detail/header_actions.rs",
        "src/container/detail/terminal.rs",
        "src/container/detail/file_preview.rs",
        "src/container/detail/files/mod.rs",
        "src/container/detail/files/row.rs",
        "src/container/detail/files/toolbar.rs",
        "src/container/actions/changes.rs",
        "src/container/logs_actions.rs",
        "src/container/stats/mod.rs",
        "src/container/stats/toolbar.rs",
        "src/container/stats/ports.rs",
        "src/container/stats/processes.rs",
        "src/container/stats_actions.rs",
        "src/container/files_actions/browse.rs",
        "src/container/files_actions/edit.rs",
        "src/container/files_actions/mutate.rs",
        "src/container/files_actions/prompt.rs",
        "src/container/files_actions/transfer.rs",
        "src/container/modals/tag.rs",
        "src/container/modals/export.rs",
        "src/container/modals/commit.rs",
        "src/container/modals/update.rs",
        "src/container/modals/forward.rs",
        "src/container/modals/network.rs",
        "src/container/modals/files.rs",
        "src/container/modals/pods.rs",
        "src/container/modals/quadlet.rs",
        "src/container/tag.rs",
        "src/container/export.rs",
        "src/container/commit.rs",
        "src/container/update.rs",
        "src/container/network_connect.rs",
        "src/container/push_dialog.rs",
        "src/container/system_df_dialog.rs",
        "src/container/build_dialog/render.rs",
        "src/container/build_dialog/actions.rs",
        "src/container/hub/dialog.rs",
        "src/container/hub/mod.rs",
        "src/container/view/mod.rs",
        "src/container/sidebar/mod.rs",
        "src/container/sidebar/entity.rs",
        "src/container/sidebar/drag.rs",
        "src/container/sidebar/empty.rs",
        "src/container/sidebar/engine_row.rs",
        "src/container/sidebar/folder_form.rs",
        "src/container/sidebar/folder_row.rs",
        "src/container/sidebar/rows.rs",
    ];
    let mut offenders = Vec::new();
    for file in files {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let uses_shell_context = content.lines().any(|line| {
            let line = line.trim_start();
            !line.starts_with("//") && line.contains("Context<AppShell>")
        });
        if content.contains("impl AppShell") || uses_shell_context {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "已下沉的容器渲染文件不得再挂回外壳：\n{}",
        offenders.join("\n")
    );
}

/// 提取源码中所有以 `header` 开头的 impl 块正文。
///
/// 设计原理（Why）：只看文件级字符串会把“实体 impl 里的 render 方法”误判成
/// 外壳渲染；按花括号配平切出 `impl AppShell` 块，才能精确约束外壳。

#[test]
fn container_has_no_shell_render_methods() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/container")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for block in impl_blocks(&content, "impl AppShell {") {
            if block.contains("fn render_") {
                offenders.push(entry.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "容器渲染不得再挂回 impl AppShell，请改为自由函数或实体方法：\n{}",
        offenders.join("\n")
    );
}

/// 容器侧栏必须由实体持有：搜索、选中、分组表单与发现面板状态都归
/// `ContainerSidebarView`，侧栏目录不得再由 `ContainerUiState` 平铺。
#[test]
fn container_sidebar_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dialogs = std::fs::read_to_string(root.join("src/container/types/dialogs.rs"))
        .expect("读取容器界面状态失败");
    assert!(
        dialogs.contains("sidebar: Entity<super::super::ContainerSidebarView>"),
        "ContainerUiState 必须持有 Entity<ContainerSidebarView>"
    );
    for forbidden in [
        "sidebar_search: Option<Entity<InputState>>",
        "sidebar_selected: Option<transom_core::EngineProfileId>",
        "engine_folder_form: Option<EngineFolderForm>",
        "discovery: Option<ContainerDiscovery>",
        "discovering: bool",
    ] {
        assert!(
            !dialogs.contains(forbidden),
            "侧栏状态不得继续平铺在 ContainerUiState：{forbidden}"
        );
    }
    let entity = std::fs::read_to_string(root.join("src/container/sidebar/entity.rs"))
        .expect("读取侧栏实体失败");
    assert!(
        entity.contains("pub(crate) struct ContainerSidebarView"),
        "ContainerSidebarView 实体必须存在于 container/sidebar/entity.rs"
    );
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/container/sidebar")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let uses_shell_context = content.lines().any(|line| {
            let line = line.trim_start();
            !line.starts_with("//") && line.contains("Context<AppShell>")
        });
        if content.contains("impl AppShell") || uses_shell_context {
            offenders.push(entry.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "侧栏实体文件不得回挂外壳，跨域动作走 ContainerSidebarEvent：\n{}",
        offenders.join("\n")
    );
}

/// `with_container*` 迁移桥必须彻底删除，事件处理与动作直接操作实体。
#[test]
fn container_migration_bridge_is_removed() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for forbidden in ["with_container(", "with_container_mut("] {
            if content.contains(forbidden) {
                offenders.push(format!("{}: {forbidden}", entry.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "with_container 迁移桥不得复活：\n{}",
        offenders.join("\n")
    );

    let container = std::fs::read_to_string(root.join("src/container/mod.rs"))
        .expect("读取 container/mod.rs 失败");
    for forbidden in [
        "fn take_container_request(",
        "fn container_request_tab(",
        "fn container_request_label(",
    ] {
        assert!(
            !container.contains(forbidden),
            "请求反查访问器必须归 AppState，不得留在 container/mod.rs：{forbidden}"
        );
    }
}

/// 容器实体不得持有父实体句柄或同步回交外壳的通道。
///
/// 设计原理（Why）：容器工作区与侧栏只通过事件向外壳表达意图，数据由外壳
/// 注入；一旦出现 `WeakEntity<AppShell>` 或 `with_shell` 同步回调，就会重新
/// 引入双向调用与双重租借风险。
#[test]
fn container_entities_have_no_parent_handle_or_sync_callback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/container");
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }
            for forbidden in ["WeakEntity<AppShell>", "with_shell(", "read_shell("] {
                if trimmed.contains(forbidden) {
                    offenders.push(format!("{}: {trimmed}", entry.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "容器实体不得持有父实体句柄或同步回交通道：\n{}",
        offenders.join("\n")
    );
}

/// 会话、容器与 Git 的侧栏行必须复用同一套表现状态，列表留白也必须走同一入口。
#[test]
fn sidebars_share_row_presentation_and_list_frame() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/sidebar/tree/session_row.rs",
        "src/container/sidebar/engine_row.rs",
        "src/git/sidebar/rows.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取侧栏行失败");
        assert!(
            content.contains("sidebar_row_band") && content.contains("SidebarRowState"),
            "{relative} 必须使用共享 SidebarRowState 与 sidebar_row_band"
        );
        assert!(
            !content.contains("p.list_background"),
            "{relative} 不得重新维护普通行底色"
        );
    }
    for relative in [
        "src/sidebar/tree/folder_row.rs",
        "src/container/sidebar/folder_row.rs",
        "src/git/sidebar/rows.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取侧栏分组行失败");
        assert!(
            content.contains("sidebar_group_row_band"),
            "{relative} 必须使用共享 sidebar_group_row_band"
        );
    }
    for relative in [
        "src/sidebar/mod.rs",
        "src/container/sidebar/mod.rs",
        "src/git/sidebar/mod.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取侧栏列表失败");
        assert!(
            content.contains("sidebar_list_frame"),
            "{relative} 必须使用共享 sidebar_list_frame"
        );
    }
}

/// Git 项目行与另外两套侧栏保持一致：单击只选中，双击才打开。
#[test]
fn git_sidebar_project_row_selects_then_opens() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git/sidebar/rows.rs");
    let content = std::fs::read_to_string(path).expect("读取 Git 侧栏行失败");
    assert!(
        content.contains("click_count() >= 2")
            && content.contains("select_git_project(this, id, cx)")
            && content.contains("open_git_tab(this, project_for_click.clone(), window, cx)"),
        "Git 项目行必须单击选中、双击打开"
    );
}

/// 容器详情页签的选中图标必须统一使用强调色，不能因图标分类不同而保持原色。
#[test]
fn container_detail_tab_icons_share_selected_tint() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/container/detail/tab_strip.rs");
    let content = std::fs::read_to_string(path).expect("读取容器详情页签失败");
    assert!(
        content.contains("action_button_with_icon_tint")
            && content.contains("selected.then_some(p.list_active_border)"),
        "容器详情页签选中态必须统一传入选中的强调色"
    );
}

/// Git 工作区主视图与子视图已完成向 `Entity<GitWorkspace>` 迁移，不得再依赖 `AppShell`。
#[test]
fn git_workspace_views_have_no_app_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for dir in [
        "src/git/log_view",
        "src/git/diff_view",
        "src/git/commit_page",
        "src/git/commit_modal",
        "src/git/shelf",
    ] {
        for entry in walkdir_like(&root.join(dir)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            if content.contains("AppShell") {
                offenders.push(entry.display().to_string());
            }
        }
    }
    for file in [
        "src/git/stash.rs",
        "src/git/workspace.rs",
        "src/git/bottom_log.rs",
    ] {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content.contains("AppShell") {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "以下 Git 工作区视图文件仍包含 AppShell 耦合：\n{}",
        offenders.join("\n")
    );
}
