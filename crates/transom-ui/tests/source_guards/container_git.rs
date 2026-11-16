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
            && content.contains("GitSidebarEvent::SelectProject")
            && content.contains("GitSidebarEvent::OpenProjectTab"),
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
        "src/git/conflict_modal",
        "src/git/push_modal",
        "src/git/shelf",
        "src/git/sidebar",
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
        "src/git/rebase_modal.rs",
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

/// Git 工作区实体级动作（提交、差异、日志、引用、工作树等）禁止再依赖 `AppShell`。
#[test]
fn git_workspace_actions_have_no_app_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for dir in ["src/git/actions/log_actions"] {
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
        "src/git/actions/commit_actions.rs",
        "src/git/actions/diff_actions.rs",
        "src/git/actions/diff_nav.rs",
        "src/git/actions/diff_search.rs",
        "src/git/actions/workspace_refs.rs",
        "src/git/actions/shelf_actions.rs",
        "src/git/actions/ref_actions.rs",
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
        "以下 Git 工作区动作文件仍包含 AppShell 耦合：\n{}",
        offenders.join("\n")
    );
}

/// 容器详情、侧栏、Compose、列表键盘导航、拉取抽屉与基础状态类型禁止在代码中依赖 `AppShell`。
#[test]
fn container_domain_modules_have_no_app_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for dir in [
        "src/container/detail",
        "src/container/sidebar",
        "src/container/compose",
        "src/container/actions",
        "src/container/list",
        "src/container/view",
    ] {
        for entry in walkdir_like(&root.join(dir)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            if content
                .lines()
                .any(|line| !line.trim_start().starts_with("//") && line.contains("AppShell"))
            {
                offenders.push(entry.display().to_string());
            }
        }
    }
    for file in [
        "src/container/list/keyboard.rs",
        "src/container/list/filters.rs",
        "src/container/pull_drawer.rs",
        "src/container/types/dialogs.rs",
        "src/container/types/workspace/state.rs",
    ] {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content
            .lines()
            .any(|line| !line.trim_start().starts_with("//") && line.contains("AppShell"))
        {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "以下容器领域模块仍在代码中包含 AppShell 耦合：\n{}",
        offenders.join("\n")
    );
}

/// Git 目录选择器、分组字段与项目/克隆独立弹窗实体禁止在代码中依赖 `AppShell`。
#[test]
fn git_modals_and_browser_have_no_app_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/git/dir_browser")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content
            .lines()
            .any(|line| !line.trim_start().starts_with("//") && line.contains("AppShell"))
        {
            offenders.push(entry.display().to_string());
        }
    }
    for file in [
        "src/git/form.rs",
        "src/git/folder_field.rs",
        "src/git/project_modal.rs",
        "src/git/clone_modal/mod.rs",
        "src/git/clone_modal/view.rs",
        "src/session_appearance.rs",
    ] {
        let path = root.join(file);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content
            .lines()
            .any(|line| !line.trim_start().starts_with("//") && line.contains("AppShell"))
        {
            offenders.push(file.to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "以下 Git 弹窗与浏览组件仍在代码中包含 AppShell 耦合：\n{}",
        offenders.join("\n")
    );
}

/// 容器工作区弹窗必须收敛在 `ContainerWorkspaceModals` 子状态中，且 `dialog_host` 不得逐字段探测。
#[test]
fn container_workspace_modals_are_grouped_and_dispatched_uniformly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let state_src = std::fs::read_to_string(root.join("src/container/types/workspace/state.rs"))
        .expect("读取 workspace/state.rs 失败");
    assert!(
        state_src.contains("pub(crate) modals: ContainerWorkspaceModals"),
        "ContainerWorkspaceState 必须通过 modals: ContainerWorkspaceModals 聚合工作区弹窗"
    );
    for forbidden in [
        "pub(crate) confirm:",
        "pub(crate) rename_container:",
        "pub(crate) update_container:",
        "pub(crate) tag_image:",
        "pub(crate) connect_network:",
        "pub(crate) export_config:",
        "pub(crate) commit_container:",
        "pub(crate) hub_dialog:",
        "pub(crate) push_dialog:",
        "pub(crate) build_dialog:",
        "pub(crate) forward_dialog:",
        "pub(crate) system_df_dialog_open:",
        "pub(crate) pods_dialog_open:",
        "pub(crate) quadlets_dialog_open:",
    ] {
        assert!(
            !state_src.contains(forbidden),
            "弹窗字段不得重新平铺在 ContainerWorkspaceState：{forbidden}"
        );
    }

    let checks_src =
        std::fs::read_to_string(root.join("src/dialog_host/registry/checks/container.rs"))
            .expect("读取 checks/container.rs 失败");
    assert!(
        !checks_src.contains("find_container_modal_tab"),
        "dialog_host/registry/checks/container.rs 不得逐个枚举工作区弹窗，应由 ContainerWorkspaceModal 统一接管"
    );
}

/// 容器模块不得再以 `Shell` 类型的参数接收外壳：领域逻辑只借应用状态，
/// 需要窗口的动作走 `WindowRequest` 队列。
///
/// 设计原理（Why）：`Context<AppShell>` 只是订阅与通知所需的上下文，允许存在；
/// 真正要杜绝的是把整个外壳当参数传进领域函数，那样状态边界与可测试性都会退化。
#[test]
fn container_modules_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/container")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{}:{}", entry.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "容器模块不得再接收外壳参数，改为 &mut AppState 并经 WindowRequest 处理窗口动作: {offenders:?}"
    );
}

/// Git 模块不得再以 `shell` 类型的参数接收外壳：领域逻辑只借应用状态，
/// 需要窗口或跨标签的动作经 `WindowRequest` 队列交给外壳。
#[test]
fn git_modules_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/git")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{}:{}", entry.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "Git 模块不得再接收外壳参数，改为 &mut AppState 并经 WindowRequest 处理窗口动作: {offenders:?}"
    );
}

/// 弹窗宿主不得以 `shell` 类型的参数接收外壳：判定、提交、取消与渲染只借
/// 应用状态；设置保存等动作已下沉到 `impl AppState`。
#[test]
fn dialog_host_modules_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/dialog_host")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{}:{}", entry.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "弹窗宿主不得再接收外壳参数，改为 &mut AppState: {offenders:?}"
    );
}

/// 连接模块不得以 `shell` 类型的参数接收外壳：连接建立、本地终端启动、
/// 凭据事件与 Shell 菜单只借应用状态，标签创建与命令投递都在 `AppState`。
#[test]
fn connection_modules_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/connection")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{}:{}", entry.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "连接模块不得再接收外壳参数，改为 &mut AppState: {offenders:?}"
    );
}

/// 分屏模块不得以 `shell` 类型的参数接收外壳：并排状态增删查改只借应用状态，
/// 激活与关闭动作经 `WindowRequest` 在渲染期执行。
#[test]
fn tiling_modules_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/tiling")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{}:{}", entry.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "分屏模块不得再接收外壳参数，改为 &mut AppState: {offenders:?}"
    );
}

/// 录制模块不得以 `shell` 类型的参数接收外壳：录制启停、回放弹窗与播放器
/// 启动只借应用状态，激活标签经 `WindowRequest` 排期。
#[test]
fn record_modules_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/record")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{}:{}", entry.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "录制模块不得再接收外壳参数，改为 &mut AppState: {offenders:?}"
    );
}

/// 面板与设置类模块不得以 `shell` 类型的参数接收外壳：这些动作只改应用，
/// 窗口级副作用已统一走 `WindowRequest`。
#[test]
fn panels_take_app_state_not_shell() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for dir in [
        "src/forward",
        "src/settings",
        "src/snippets",
        "src/batch",
        "src/audit_viewer",
        "src/monitor",
        "src/keys",
        "src/status_bar",
    ] {
        for entry in walkdir_like(&root.join(dir)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                    offenders.push(format!("{}:{}", entry.display(), index + 1));
                }
            }
        }
    }
    for file in ["src/shortcuts.rs", "src/zmodem.rs"] {
        let Ok(content) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                offenders.push(format!("{file}:{}", index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "这些模块不得再接收外壳参数，改为 &mut AppState: {offenders:?}"
    );
}

/// 编辑器关闭续行的决策必须留在实体侧：桥接只借工作区快照、脏标签集合，
/// 不再在桥接里解析标签组。
#[test]
fn editor_close_resolution_lives_on_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let close = std::fs::read_to_string(root.join("src/editor/close.rs"))
        .expect("读取 editor/close.rs 失败");
    assert!(
        close.contains("fn finish_close_intent") && close.contains("fn tabs_for_pending_close"),
        "关闭续行决策必须由 EditorPanels 自行完成（finish_close_intent / tabs_for_pending_close）"
    );
    let bridge = std::fs::read_to_string(root.join("src/editor/bridge/lifecycle.rs"))
        .expect("读取 editor/bridge/lifecycle.rs 失败");
    assert!(
        !bridge.contains("fn tabs_for_pending_close"),
        "标签解析不得回退到 editor/bridge/lifecycle.rs"
    );
}

/// Git 补初始化回执必须由实体落地：注册表与徽标归 `GitHub`，标签页状态归
/// `GitWorkspace`，桥接不得直接改字段。
#[test]
fn git_init_result_applied_by_entities() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sidebar = std::fs::read_to_string(root.join("src/git/types/sidebar.rs"))
        .expect("读取 git/types/sidebar.rs 失败");
    assert!(
        sidebar.contains("fn take_project_init") && sidebar.contains("fn set_badge_result"),
        "Git 注册表必须自行管理补初始化请求与徽标"
    );
    let entity =
        std::fs::read_to_string(root.join("src/git/entity.rs")).expect("读取 git/entity.rs 失败");
    assert!(
        entity.contains("fn apply_project_init_result"),
        "GitWorkspace 必须自行应用补初始化结果"
    );
    let bridge = std::fs::read_to_string(root.join("src/git/actions/init_event.rs"))
        .expect("读取 git/actions/init_event.rs 失败");
    assert!(
        !bridge.contains("state.init_running")
            && !bridge.contains("state.not_repo")
            && !bridge.contains("badge.loading"),
        "补初始化桥接不得直接改实体字段"
    );
}

/// Git 读取类结果必须由实体落地：引用、提交文件、日志分页与失败回执都不得
/// 在事件桥接里直接改字段。
#[test]
fn git_read_results_applied_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let entity = std::fs::read_to_string(root.join("src/git/actions/log_actions/mod.rs"))
        .expect("读取 git/actions/log_actions/mod.rs 失败");
    for method in [
        "fn apply_refs_loaded",
        "fn apply_commit_files_loaded",
        "fn apply_log_loaded",
        "fn apply_read_failure",
    ] {
        assert!(
            entity.contains(method),
            "读取结果必须由 GitWorkspace 落地：{method}"
        );
    }
    let bridge =
        std::fs::read_to_string(root.join("src/git/actions/event_dispatch/read_events.rs"))
            .expect("读取 git/actions/event_dispatch/read_events.rs 失败");
    for forbidden in [
        "state.log.loading",
        "state.commit_files_loading",
        "state.branches",
        "badge.loading",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "read_events 桥接不得直接改字段：{forbidden}"
        );
    }
}

/// Git 动作完成结果必须由实体落地：提交收尾、状态快照与徽标都不得在
/// action_done 桥接里直接改字段。
#[test]
fn git_action_results_applied_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, methods) in [
        (
            "src/git/actions/commit_actions.rs",
            vec!["fn apply_commit_done"],
        ),
        (
            "src/git/entity.rs",
            vec!["fn apply_repo_status", "fn take_pending_worktree"],
        ),
        (
            "src/git/types/sidebar.rs",
            vec!["fn apply_repo_status_badge"],
        ),
    ] {
        let content = std::fs::read_to_string(root.join(file)).expect("读取 Git 实体文件失败");
        for method in methods {
            assert!(content.contains(method), "{file} 缺少实体方法：{method}");
        }
    }
    let bridge =
        std::fs::read_to_string(root.join("src/git/actions/event_dispatch/action_done.rs"))
            .expect("读取 git/actions/event_dispatch/action_done.rs 失败");
    for forbidden in [
        "workspace.state",
        "state.commit_submitting",
        "state.checked",
        "state.not_repo",
        "badge.branch",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "action_done 桥接不得直接改实体字段：{forbidden}"
        );
    }
}

/// Git 货架与工作树事件必须由实体落地：展开态清理、列表去重与页签切换
/// 都不得在 shelf_events 桥接里直接改字段。
#[test]
fn git_shelf_results_applied_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let entity = std::fs::read_to_string(root.join("src/git/actions/shelf_actions.rs"))
        .expect("读取 git/actions/shelf_actions.rs 失败");
    for method in [
        "fn apply_worktrees_loaded",
        "fn apply_stashes_loaded",
        "fn apply_stash_files_loaded",
        "fn apply_shelves_loaded",
        "fn apply_shelved",
        "fn apply_shelf_applied",
        "fn apply_shelf_dropped",
    ] {
        assert!(
            entity.contains(method),
            "货架结果必须由 GitWorkspace 落地：{method}"
        );
    }
    let bridge =
        std::fs::read_to_string(root.join("src/git/actions/event_dispatch/shelf_events.rs"))
            .expect("读取 git/actions/event_dispatch/shelf_events.rs 失败");
    for forbidden in [
        "workspace.state",
        "state.shelves",
        "state.worktrees",
        "GitCommitTab",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "shelf_events 桥接不得直接改实体字段：{forbidden}"
        );
    }
}

/// 云同步事件结果必须由实体方法落地：调度节拍、冲突清理、历史与提示文案
/// 不得在 events 桥接里直接写 `view.state`。
#[test]
fn cloud_sync_events_use_entity_methods() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let entity = std::fs::read_to_string(root.join("src/cloud_sync/entity.rs"))
        .expect("读取 cloud_sync/entity.rs 失败");
    for method in [
        "fn mark_check_finished",
        "fn apply_push_success",
        "fn apply_pull_success",
        "fn set_message",
    ] {
        assert!(
            entity.contains(method),
            "云同步结果必须由实体落地：{method}"
        );
    }
    let bridge = std::fs::read_to_string(root.join("src/cloud_sync/events.rs"))
        .expect("读取 cloud_sync/events.rs 失败");
    assert!(
        !bridge.contains("view.state."),
        "cloud_sync/events.rs 不得直接写实体字段，应调用 CloudSyncView 方法"
    );
}

/// SFTP 输入提交的参数必须收敛为参数对象，避免平铺七个参数造成顺序错误。
#[test]
fn sftp_commit_uses_params_object() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let commit = std::fs::read_to_string(root.join("src/sftp/file_ops/commit.rs"))
        .expect("读取 sftp/file_ops/commit.rs 失败");
    assert!(
        commit.contains("params: SftpInputCommit<'_>") && commit.contains("struct SftpInputCommit"),
        "commit_sftp_input_value 必须使用 SftpInputCommit 参数对象"
    );
}

/// Git 模态上下文必须是窄上下文：只借弹窗组、订阅表、命令端、提示队列、
/// 工作区快照与窗口请求队列，不得再持有整份 AppState。
#[test]
fn git_modal_actions_is_narrow_context() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/git/actions/modal_bridges/mod.rs"))
        .expect("读取 git/actions/modal_bridges/mod.rs 失败");
    assert!(
        source.contains("struct GitModalActions<'a>")
            && source.contains("modals: &'a mut GitModals"),
        "GitModalActions 必须显式借用所需字段"
    );
    assert!(
        !source.contains("\n    state: &'a mut"),
        "GitModalActions 不得再持有整份 AppState 字段"
    );
}

/// 禁止再新增"整状态上下文"（`{ state: &mut AppState }` 形式的马甲）。
///
/// 允许清单是迁移期债务：当前只剩 GitActions，修复后必须把清单清空，
/// 新出现的整状态上下文一律失败。
#[test]
fn whole_state_contexts_are_registered() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for line in content.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.starts_with("state:")
                && trimmed.contains("AppState")
                && trimmed.contains("&'a mut")
            {
                let relative = entry.strip_prefix(root.join("src")).unwrap_or(&entry);
                offenders.push(crate::common::normalized_path(relative));
            }
        }
    }
    offenders.sort();
    offenders.dedup();
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "不允许任何整状态上下文（`{{ state: &mut AppState }}`）复活"
    );
}

/// GitActions 必须是字段级窄上下文：注册表、侧栏、订阅、服务、落盘、
/// 工作区、弹窗与请求队列逐项借用，不得持有整份 AppState。
#[test]
fn git_actions_is_narrow_context() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/git/actions/mod.rs"))
        .expect("读取 git/actions/mod.rs 失败");
    for field in [
        "sidebar: Entity<GitSidebarView>",
        "session_sidebar: &'a mut SidebarState",
        "services: &'a UiServices",
        "persistence: &'a Persistence",
        "workspace: &'a mut WorkspaceState",
        "modals: &'a mut GitModals",
        "window_requests: &'a mut WindowRequests",
        "pending_notices: &'a mut PendingNotices",
    ] {
        assert!(source.contains(field), "GitActions 缺少窄字段：{field}");
    }
    assert!(
        !source.contains("\n    state: &'a mut"),
        "GitActions 不得再持有整份 AppState 字段"
    );
}

/// 全仓携壳参数只允许出现在窗口生命周期文件；其余模块必须清零。
#[test]
fn shell_parameters_are_registered() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for line in content.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("shell: &mut AppShell") || line.contains("shell: &AppShell") {
                let relative = entry.strip_prefix(root.join("src")).unwrap_or(&entry);
                offenders.push(crate::common::normalized_path(relative));
            }
        }
    }
    assert_eq!(
        offenders,
        vec!["window_state.rs".to_string(), "window_state.rs".to_string()],
        "携壳参数只允许留在窗口生命周期（保存窗口状态与关闭应用）"
    );
}

/// 超参热点必须使用参数对象：端口转发、SFTP 同步执行与回放播放器开场三处
/// 原先平铺 6–7 个参数，禁止回退。
#[test]
fn hotspot_functions_use_params_objects() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = [
        (
            "src/container/port_forward.rs",
            "struct ContainerPortForwardOpen",
            "params: ContainerPortForwardOpen<'_>",
        ),
        (
            "src/sftp/sync_actions.rs",
            "struct SftpSyncExecute",
            "params: SftpSyncExecute<'_>",
        ),
        (
            "src/record/ops.rs",
            "struct CastPlayerOpen",
            "params: CastPlayerOpen",
        ),
    ];
    for (file, marker, param) in cases {
        let content = std::fs::read_to_string(root.join(file)).expect("读取参数对象文件失败");
        assert!(content.contains(marker), "{file} 缺少参数对象：{marker}");
        assert!(
            content.contains(param),
            "{file} 函数未使用参数对象：{param}"
        );
    }
}

/// 生产源码单文件不得超过 500 行：超限必须按关注点拆分子模块。
#[test]
fn no_source_file_over_500_lines() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let lines = content.lines().count();
        if lines > 500 {
            offenders.push(format!("{}: {lines}", entry.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "单文件超过 500 行必须拆分为子模块: {offenders:?}"
    );
}

/// 容器详情渲染必须保持分阶段拆分：主函数只做空态、头部与内容分发，
/// 单函数不超过 80 行。
#[test]
fn container_detail_render_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/detail/render.rs"))
        .expect("读取 container/detail/render.rs 失败");
    for helper in [
        "fn render_empty_detail",
        "fn render_detail_header_block",
        "fn render_detail_content",
        "fn render_containers_content",
    ] {
        assert!(source.contains(helper), "详情渲染缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn render_container_detail")
        .expect("缺少 render_container_detail");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.expect("函数体未闭合");
    let lines = source[start..end].lines().count();
    assert!(lines <= 80, "render_container_detail 超过 80 行：{lines}");
}

/// SFTP 本地右键菜单必须按动作域拆分：主构造器只决定顺序与条件，
/// 单函数不超过 60 行。
#[test]
fn local_context_menu_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/sftp/local_pane_view/context_menu.rs"))
        .expect("读取 sftp/local_pane_view/context_menu.rs 失败");
    for helper in [
        "fn build_parent_entry_menu",
        "fn append_edit_item",
        "fn append_create_items",
        "fn append_paste_item",
    ] {
        assert!(source.contains(helper), "右键菜单缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn build_local_context_menu")
        .expect("缺少 build_local_context_menu");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 60, "build_local_context_menu 超过 60 行：{lines}");
}

/// 容器转发表单渲染必须按区块拆分：主函数只组装内容与页脚，
/// 单函数不超过 80 行。
#[test]
fn container_forward_dialog_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/modals/forward.rs"))
        .expect("读取 container/modals/forward.rs 失败");
    for helper in [
        "fn forward_info_card",
        "fn forward_session_section",
        "fn forward_inputs",
        "fn forward_footer",
    ] {
        assert!(source.contains(helper), "转发表单缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn render_container_forward_dialog_body")
        .expect("缺少主渲染函数");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "转发表单主函数超过 80 行：{lines}");
}

/// 镜像结果卡片必须按部件拆分，主渲染函数不超过 80 行，并继续使用参数对象。
#[test]
fn hub_result_card_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/hub/card.rs"))
        .expect("读取 container/hub/card.rs 失败");
    for helper in [
        "fn image_avatar",
        "fn image_name_row",
        "fn image_meta_row",
        "fn tag_chip_row",
        "fn card_actions",
    ] {
        assert!(source.contains(helper), "镜像卡片缺少拆分函数：{helper}");
    }
    assert!(
        source.contains("struct HubResultCardParams"),
        "镜像卡片渲染必须使用参数对象"
    );
    let start = source
        .find("fn render_hub_result_card")
        .expect("缺少 render_hub_result_card");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "render_hub_result_card 超过 80 行：{lines}");
}

/// 容器仓库列表渲染必须拆分：主函数只负责顶栏与卡片组装，不超过 80 行。
#[test]
fn registries_list_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/settings/containers_page/list.rs"))
        .expect("读取 settings/containers_page/list.rs 失败");
    for helper in [
        "fn registry_top_bar",
        "fn registry_card",
        "fn registry_identity",
        "fn registry_actions",
    ] {
        assert!(source.contains(helper), "仓库列表缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn render_registries_list")
        .expect("缺少 render_registries_list");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "render_registries_list 超过 80 行：{lines}");
}

/// 端口统计卡片必须拆分：主函数只负责卡片外壳与折叠，不超过 80 行。
#[test]
fn ports_card_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/stats/ports.rs"))
        .expect("读取 container/stats/ports.rs 失败");
    for helper in [
        "fn ports_loading_card",
        "fn ports_header",
        "fn ports_body",
        "fn published_port_row",
        "fn exposed_port_row",
    ] {
        assert!(source.contains(helper), "端口卡片缺少拆分函数：{helper}");
    }
    let start = source.find("fn ports_card").expect("缺少 ports_card");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "ports_card 超过 80 行：{lines}");
}

/// 容器头部动作必须拆分：主函数只分派生命周期动作与“更多”按钮，不超过 80 行。
#[test]
fn container_header_actions_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/detail/header_container.rs"))
        .expect("读取 container/detail/header_container.rs 失败");
    for helper in [
        "fn lifecycle_actions",
        "fn stop_action",
        "fn terminal_jump_button",
        "fn container_more_button",
    ] {
        assert!(source.contains(helper), "头部动作缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn container_header_actions")
        .expect("缺少 container_header_actions");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "container_header_actions 超过 80 行：{lines}");
}

/// 纵向标签行必须拆分：主函数只做快照、拖拽与事件装配，不超过 80 行。
#[test]
fn vertical_tab_row_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/workspace/vertical_tabs/item.rs"))
        .expect("读取 workspace/vertical_tabs/item.rs 失败");
    for helper in [
        "fn collect_row_data",
        "fn build_tab_band",
        "fn tab_band_labels",
        "fn add_tab_band_badges",
        "fn apply_row_events",
    ] {
        assert!(source.contains(helper), "纵向标签行缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn render_vertical_tab_row")
        .expect("缺少 render_vertical_tab_row");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "render_vertical_tab_row 超过 80 行：{lines}");
}

/// 引擎侧栏行必须拆分并参数化：入口只做快照、菜单装配与行体拼接。
#[test]
fn engine_row_is_split_and_parameterized() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/sidebar/engine_row.rs"))
        .expect("读取 container/sidebar/engine_row.rs 失败");
    assert!(
        source.contains("struct EngineRowParams"),
        "引擎行渲染必须使用参数对象"
    );
    for helper in [
        "fn engine_row_outline",
        "fn engine_row_shell",
        "fn engine_context_menu",
        "fn engine_row_band",
    ] {
        assert!(source.contains(helper), "引擎行缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn render_engine_row")
        .expect("缺少 render_engine_row");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "render_engine_row 超过 80 行：{lines}");
}

/// 小于 30 行的源码文件只能是模块索引、共享导入或测试模块；
/// 新出现的单函数碎片文件一律视为碎片化回退。
#[test]
fn tiny_source_files_are_index_or_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.lines().count() >= 30 {
            continue;
        }
        let name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !matches!(name, "mod.rs" | "common.rs" | "tests.rs") {
            offenders.push(format!("{}", entry.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "小于 30 行的文件只允许 mod.rs/common.rs/tests.rs: {offenders:?}"
    );
}

/// 系统磁盘看板弹窗必须拆分：主函数只做内容组装，不超过 80 行。
#[test]
fn system_df_dialog_is_split() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/container/system_df_dialog.rs"))
        .expect("读取 container/system_df_dialog.rs 失败");
    for helper in [
        "fn df_summary_banner",
        "fn df_options_section",
        "fn df_prune_all_row",
        "fn df_prune_volumes_row",
        "fn df_actions_bar",
    ] {
        assert!(source.contains(helper), "磁盘看板缺少拆分函数：{helper}");
    }
    let start = source
        .find("fn render_container_system_df_dialog_body")
        .expect("缺少磁盘看板主渲染函数");
    let brace = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("缺少函数体");
    let mut depth = 0i32;
    let mut end = None;
    for (offset, ch) in source[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(brace + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let lines = source[start..end.expect("函数体未闭合")].lines().count();
    assert!(lines <= 80, "磁盘看板主函数超过 80 行：{lines}");
}
