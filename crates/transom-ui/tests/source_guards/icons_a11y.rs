//! 图标管线、列表虚拟化、可访问性与组件构造器守卫。

use crate::common::{collect_rust_files, normalized_path, ui_source_files, walkdir_like};

/// 可无限增长的数据列表必须使用虚拟列表，不能在渲染时全量生成元素。
#[test]
fn large_lists_use_virtualization() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (relative, required) in [
        ("src/git/log_view/log_table.rs", "v_virtual_list"),
        // Git Diff 与 SFTP 远程编辑器统一委托给基于 gpui_kit 虚拟列表底座的 diff_pane
        ("src/git/diff_view/mod.rs", "render_diff_body("),
        ("src/editor/diff.rs", "render_diff_body("),
        ("src/diff_pane/controls.rs", "Diff::new("),
        ("src/container/list/render.rs", "v_virtual_list"),
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取列表文件失败");
        assert!(content.contains(required), "{relative} 必须使用虚拟列表");
    }
    let local_sftp = std::fs::read_to_string(root.join("src/sftp/local_pane_view/mod.rs"))
        .expect("读取 SFTP 本地列表失败");
    assert!(
        !local_sftp.contains("entries.to_vec()"),
        "SFTP 本地列表不得每帧复制完整条目集合"
    );
}

/// Git 日志三栏必须共用可拖拽分栏，不能退回固定宽度加静态边框。
#[test]
fn git_log_panes_are_resizable() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let content = std::fs::read_to_string(root.join("src/git/workspace.rs"))
        .expect("读取 Git 日志工作区失败");
    for required in [
        "git-log-split-{tab_id}",
        ".with_state(&log_split)",
        "h_resizable(",
    ] {
        assert!(
            content.contains(required),
            "Git 日志三栏未接入可拖拽分栏：{required}"
        );
    }
}

/// 日志文件树的双击不得绕过单击路径去打开独立差异弹窗。
#[test]
fn git_log_file_rows_keep_single_click_behavior() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let content = std::fs::read_to_string(root.join("src/git/log_view/files_tree.rs"))
        .expect("读取 Git 日志文件树失败");
    assert!(
        !content.contains("git_open_diff_modal"),
        "日志文件树不得在双击时打开独立差异弹窗"
    );
    assert!(
        content.contains("diff_commit_file"),
        "日志文件树点击必须走右栏差异选择路径"
    );
}

/// 提交面板文件行的双击与单击保持一致（均走右侧差异栏，不弹独立差异弹窗），且行尾不重复堆叠状态字母或差异按钮。
#[test]
fn git_commit_file_rows_keep_single_click_without_trailing_badges() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let content = std::fs::read_to_string(root.join("src/git/commit_modal/file_row.rs"))
        .expect("读取 Git 提交文件行失败");
    assert!(
        !content.contains("git_open_diff_modal"),
        "提交面板文件行双击不得打开独立差异弹窗"
    );
    assert!(
        !content.contains("visual_letter"),
        "提交面板文件行已由颜色区分状态，不得在行尾重复展示状态字母"
    );
    assert!(
        !content.contains("git-row-diff-"),
        "提交面板文件行点击整行即看差异，不得在行尾重复展示差异图标按钮"
    );
}

/// 主要文件列表必须共享统一的文件类型图标解析器，不能继续使用通用 File 图标。
#[test]
fn file_lists_use_shared_file_type_icons() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/git/log_view/files_tree.rs",
        "src/git/commit_modal/file_row.rs",
        "src/git/conflict_modal/panels.rs",
        "src/git/shelf/mod.rs",
        "src/git/diff_view/tabs.rs",
        "src/git/log_view/menus/filter_menus.rs",
        "src/git/dir_browser/pane.rs",
        "src/sftp/file_row_content.rs",
        "src/sftp/side_remote/types.rs",
        "src/sftp/search_modal.rs",
        "src/sftp/preview_view.rs",
        "src/editor/list.rs",
        "src/editor/view.rs",
        "src/container/detail/files/row.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取文件列表源码失败");
        assert!(
            content.contains("AppIcon::for_file") || content.contains("sftp_file_icon("),
            "{relative} 未使用共享文件类型图标"
        );
    }
}

/// Git 第一阶段的工具条、筛选、差异和文件树必须使用 IntelliJ New UI 图标。
#[test]
fn git_first_phase_uses_new_ui_icons() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/git/bottom_log.rs",
        "src/git/workspace.rs",
        "src/git/commit_page/mod.rs",
        "src/git/commit_box.rs",
        "src/git/sidebar/mod.rs",
        "src/git/sidebar/rows.rs",
        "src/git/log_view/mod.rs",
        "src/git/log_view/ref_toolbar.rs",
        "src/git/log_view/log_refs.rs",
        "src/git/log_view/log_details.rs",
        "src/git/log_view/files_tree.rs",
        "src/git/log_view/branch_tree/view.rs",
        "src/git/log_view/worktrees_group.rs",
        "src/git/diff_view/mod.rs",
        "src/git/diff_view/tabs.rs",
        "src/git/diff_view/menus.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取 Git 图标源码失败");
        assert!(
            !content.contains("IconName::"),
            "{relative} 仍在使用非 New UI 的 IconName 图标"
        );
    }
}

/// UI 与终端源码必须直接选择分类图标，不能重新依赖旧 IconName 兼容层。
#[test]
fn ui_sources_have_no_legacy_icon_names() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::with_capacity(256);
    collect_rust_files(&root.join("src"), &mut files);
    collect_rust_files(&root.join("../transom-terminal/src"), &mut files);
    let mut offenders = Vec::with_capacity(files.len());
    for file in files {
        let content = std::fs::read_to_string(&file).expect("读取源码失败");
        if content.contains("IconName") {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "以下源码仍在依赖旧 IconName 兼容层: {offenders:?}"
    );
}

/// 应用图标必须走全彩渲染；只有组件库仅接受 `Icon` 的少数菜单/页头接口例外。
#[test]
fn app_icons_use_color_pipeline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::with_capacity(256);
    collect_rust_files(&root.join("src"), &mut files);
    collect_rust_files(&root.join("../transom-terminal/src"), &mut files);

    let icon_new_exceptions = [
        "src/workspace/tab_strip/badges.rs",
        "src/workspace/tab_strip/menu.rs",
    ];
    let icon_method_exceptions = [
        "../transom-terminal/src/view/menu.rs",
        "src/command_palette/items.rs",
        "src/workspace/vertical_tabs/mod.rs",
        "src/monitor/view/panel.rs",
    ];
    let dynamic_icon_method_patterns = [
        ".icon(icon)",
        ".icon(default_icon)",
        ".icon(name)",
        ".icon(detail_icon(tab))",
        ".icon(action.icon())",
        ".icon(if ",
    ];

    for file in files {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let content = std::fs::read_to_string(&file).expect("读取图标源码失败");
        if content.contains("Icon::new(AppIcon")
            && !icon_new_exceptions
                .iter()
                .any(|allowed| relative.ends_with(allowed))
        {
            panic!("{relative} 仍用单色 Icon 渲染应用图标，应改用 AppIconElement");
        }
        let has_single_color_method_icon = content.contains(".icon(AppIcon")
            || dynamic_icon_method_patterns
                .iter()
                .any(|pattern| content.contains(pattern));
        if has_single_color_method_icon
            && !icon_method_exceptions
                .iter()
                .any(|allowed| relative.ends_with(allowed))
        {
            panic!("{relative} 的按钮仍用单色 icon() 渲染应用图标，应改用 AppIconElement");
        }
    }
}

/// Git 身份、分支和远端分支必须复用同一个 VCS 图标，不能退回旧分支或服务器图标。
#[test]
fn git_identity_uses_one_icon() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/settings/mod.rs",
        "src/settings/modal.rs",
        "src/git/log_view/branch_tree/view.rs",
        "src/git/log_view/mod.rs",
        "src/git/workspace.rs",
        "src/git/sidebar/rows.rs",
        "src/app/tab/presentation.rs",
    ] {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取 Git 图标源码失败");
        assert!(
            !content.contains("UiIcon::BRANCH") && !content.contains("UiIcon::WEB_SERVER"),
            "{relative} 仍在使用非统一 Git 图标"
        );
        assert!(
            content.contains("UiIcon::VCS"),
            "{relative} 未使用统一 VCS 图标"
        );
    }
    let branch_tree = std::fs::read_to_string(root.join("src/git/log_view/branch_tree/view.rs"))
        .expect("读取 Git 分支树失败");
    assert!(
        branch_tree.contains("RefNodeKind::RemoteBranch => AppIconElement::new(UiIcon::VCS)"),
        "远端分支必须使用统一 VCS 图标"
    );
}

#[test]
fn mutable_list_identities_use_business_ids() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = [
        (
            "src/git/rebase_modal.rs",
            &[
                "git-rebase-{index}",
                "git-rebase-up-{index}",
                "git-rebase-act-{index}",
            ][..],
        ),
        (
            "src/sftp/queue_view/row.rs",
            &[
                "queue-task-{idx}",
                "queue-cancel-{idx}",
                "queue-remove-{idx}",
            ][..],
        ),
        (
            "src/audit_viewer/render.rs",
            &["audit-log-item-{idx}", "delete-audit-log-{idx}"][..],
        ),
        ("src/git/log_view/log_table.rs", &["git-commit-{index}"][..]),
    ];
    for (relative, forbidden) in cases {
        let content = std::fs::read_to_string(root.join(relative)).expect("读取列表文件失败");
        for needle in forbidden {
            assert!(
                !content.contains(needle),
                "{relative} 仍使用位置索引作为身份：{needle}"
            );
        }
    }
    let diff_tabs = std::fs::read_to_string(root.join("src/git/diff_view/tabs.rs"))
        .expect("读取 Git diff 标签失败");
    assert!(
        !diff_tabs.contains("NamedInteger(\"git-diff-tab\""),
        "Git diff 标签不得使用位置索引作为 ElementId"
    );
}

/// 标签导航必须保留可聚焦、可键盘激活的语义，不能退化为纯点击 div。
#[test]
fn tabs_have_keyboard_semantics() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_tabs = std::fs::read_to_string(root.join("src/workspace/tab_strip/item.rs"))
        .expect("读取主标签栏失败");
    for required in [
        "Tab::new(",
        ".selected(",
        ".accessibility_label(",
        ".set_position(",
        "Tabs::new(",
    ] {
        let source = if required == "Tabs::new(" {
            std::fs::read_to_string(root.join("src/workspace/tab_strip/mod.rs"))
                .expect("读取主标签栏入口失败")
        } else {
            workspace_tabs.clone()
        };
        assert!(
            source.contains(required),
            "主标签栏缺少 gpui-kit Tab 键盘/可访问语义：{required}"
        );
    }
    let diff_tabs = std::fs::read_to_string(root.join("src/git/diff_view/tabs.rs"))
        .expect("读取 Git diff 标签栏失败");
    for required in [
        "Tabs::new(",
        "Tab::new(",
        ".selected(",
        ".accessibility_label(",
        ".set_position(",
        "IconButton::new(",
    ] {
        assert!(
            diff_tabs.contains(required),
            "Git diff 标签栏缺少 gpui-kit Tab 键盘/可访问语义：{required}"
        );
    }
}

/// 直接构造的图标按钮必须提供可访问名；tooltip 不能替代 accessibility_label。
#[test]
fn icon_buttons_have_accessibility_labels() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        offenders.extend(icon_button_offenders(&path, &content));
    }
    let terminal_root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../transom-terminal/src");
    for entry in walkdir_like(&terminal_root) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        offenders.extend(icon_button_offenders(&normalized_path(&entry), &content));
    }
    assert!(
        offenders.is_empty(),
        "图标按钮必须设置 label 或 accessibility_label：\n{}",
        offenders.join("\n")
    );
}

fn icon_button_offenders(path: &str, content: &str) -> Vec<String> {
    let mut offenders = Vec::new();
    let mut offset = 0;
    while let Some(relative) = content[offset..].find("Button::new(") {
        let start = offset + relative;
        if content[..start].ends_with("Icon") {
            offset = start + "Button::new(".len();
            continue;
        }
        let Some(end_relative) = content[start..].find(';') else {
            break;
        };
        let end = start + end_relative;
        let chain = &content[start..end];
        if chain.contains(".icon(")
            && !chain.contains(".label(")
            && !chain.contains(".accessibility_label(")
        {
            let line = content[..start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            offenders.push(format!(
                "{path}:{line}: {}",
                chain.lines().next().unwrap_or_default().trim()
            ));
        }
        offset = end + 1;
    }
    offenders
}

/// 菜单项必须走 widgets 构造器：`PopupMenuItem::new(...)` 紧随 `.icon(...)`
/// 是迁移前的写法，散落回页面会让图标顺序与禁用语义各自漂移。
///
/// 例外只保留两处真实差异：带真实 `Action` 的条目（动作自带键位展示）与下拉
/// 里的 `.checked(...)` 选中项，它们不属于同一语义。
#[test]
fn menu_items_use_widget_constructor() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        // 只守卫 transom-ui：transom-terminal 不能反向依赖 UI 组件层。
        if path.contains("/widgets/") || !path.contains("/transom-ui/src/") {
            continue;
        }
        let lines: Vec<&str> = content.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("PopupMenuItem::new(") && !line.contains("PopupMenuItem::new()") {
                continue;
            }
            let window = lines
                .iter()
                .skip(index)
                .take(3)
                .any(|next| next.trim_start().starts_with(".icon("));
            if window {
                offenders.push(format!("{}:{}: {}", path, index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "菜单项请使用 crate::widgets::menu_item / MenuItem：\n{}",
        offenders.join("\n")
    );
}
