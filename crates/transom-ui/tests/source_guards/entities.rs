//! 面板实体边界与 AppShell 单字段守卫。

use crate::common::{normalized_path, walkdir_like};

#[test]
fn app_shell_keeps_single_state_field() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/mod.rs");
    let content = std::fs::read_to_string(&path).expect("读取 app/mod.rs 失败");
    let start = content
        .find("pub(crate) struct AppShell {")
        .expect("未找到 AppShell 定义");
    // Windows 检出可能使用 CRLF；只匹配换行后的闭合花括号，避免把行尾格式
    // 当作结构错误。
    let end = content[start..].find("\n}").expect("AppShell 定义未闭合") + start;
    let body = &content[start..end];
    let fields: Vec<&str> = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//") && line.ends_with(','))
        .collect();
    assert_eq!(
        fields,
        vec!["state: AppState,"],
        "AppShell 只允许持有 state 一个字段，新增状态请放进 AppState 或 feature 结构体"
    );
}

/// 监控面板的渲染与面板状态必须归属 `MonitorView` 实体：`monitor/` 下只允许
/// `ops.rs` 保留 `impl AppShell`（采样调度、任务生命周期与跨域动作回交外壳）。
/// 一旦渲染方法重新散落到外壳，说明实体化被回退了。
#[test]
fn monitor_rendering_belongs_to_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/monitor");
    let mut offenders = Vec::new();
    for (path, content) in walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some((normalized_path(&path), content))
        })
    {
        if path.ends_with("ops.rs") {
            continue;
        }
        if content.contains("impl AppShell") {
            offenders.push(path);
        }
    }
    assert!(
        offenders.is_empty(),
        "监控渲染必须以 impl MonitorView 实现，不得挂在 AppShell 上：\n{}",
        offenders.join("\n")
    );
}

/// SFTP 视图不得再实现 `Deref`/`DerefMut`：字段访问必须显式写
/// `view.state.xxx`。迁移期的透明访问会让"状态归属"在调用点消失，也给
/// 后续同名字段埋下静默遮蔽。
#[test]
fn sftp_view_has_no_deref() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sftp/entity/mod.rs");
    let content = std::fs::read_to_string(&path).expect("读取 sftp/entity/mod.rs 失败");
    assert!(
        !content.contains("impl std::ops::Deref"),
        "SftpPanelView 不得再实现 Deref/DerefMut，请显式访问 view.state"
    );
}

/// 设置弹窗的页面渲染、保存与取消必须归 `SettingsView`：`settings/` 下只允许
/// `handler.rs`（跨域事件消费）与 `mod.rs`（通用控件与外观下发）保留
/// `impl AppShell`。页面一旦重新挂回外壳，设置实体化就被回退了。
#[test]
fn settings_rendering_belongs_to_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/settings");
    let mut offenders = Vec::new();
    for (path, content) in walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some((normalized_path(&path), content))
        })
    {
        if path.ends_with("handler.rs") || path.ends_with("settings/mod.rs") {
            continue;
        }
        for pattern in ["impl AppShell", "settings_modal(", "settings_modal_mut("] {
            if content.contains(pattern) {
                offenders.push(format!("{path}: {pattern}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "设置页面必须挂在 SettingsView 上，跨域动作走 SettingsEvent：\n{}",
        offenders.join("\n")
    );
}

/// 编辑面板也不得再有 `with_shell` / `read_shell` 同步回交通道。
#[test]
fn editor_panel_has_no_sync_callback_channel() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor");
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains("with_shell(") || content.contains("read_shell(") {
            offenders.push(normalized_path(&entry));
        }
    }
    assert!(
        offenders.is_empty(),
        "编辑面板不得再使用 with_shell / read_shell 同步回交外壳：\n{}",
        offenders.join("\n")
    );
}

/// 所有面板实体都不得持有外壳句柄。
///
/// 数据由外壳注入、意图用事件上报，面板不再有同步回到外壳的路径；
/// 这条结构约束同时消灭"双向调用导致双重租借"的可能。
#[test]
fn panel_entities_have_no_parent_entity_handle() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for base in ["monitor", "editor", "forward", "sftp", "container"] {
        for entry in walkdir_like(&root.join(base)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            // 只在结构体字段声明里检查：注释与文档提及不算违规。
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                if trimmed.contains("WeakEntity<AppShell>") {
                    offenders.push(format!("{}: {}", entry.display(), trimmed));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "面板实体不得持有 WeakEntity<AppShell>：\n{}",
        offenders.join("\n")
    );
}

/// 外壳处理面板事件时不得回读该面板。
///
/// 事件派发阶段面板仍被租借，处理函数里再 `read`/`update` 会触发
/// `cannot read/update ... while it is already being updated`。各面板的处理函数
/// 统一命名为 `on_*_panel_event`，这里扫描它们的函数体。
#[test]
fn panel_event_handlers_do_not_read_back() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for base in ["monitor", "editor", "forward", "sftp"] {
        for entry in walkdir_like(&root.join(base)) {
            if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            let Some(start) = content.find("fn on_") else {
                continue;
            };
            let name_start = start + "fn ".len();
            let Some(name_end) = content[name_start..].find('(') else {
                continue;
            };
            let name = &content[name_start..name_start + name_end];
            if !name.ends_with("panel_event") {
                continue;
            }
            let Some(body_start) = content[name_start..].find('{') else {
                continue;
            };
            let body = &content[name_start + body_start..];
            let mut depth = 0usize;
            let mut end = 0usize;
            for (offset, ch) in body.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &body[..end];
            for pattern in ["sftp_view(", "sftp_state(", "settings_view(", ".read(cx)"] {
                if body.contains(pattern) {
                    offenders.push(format!("{name}: {pattern} @ {}", entry.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "面板事件处理函数不得回读面板：\n{}",
        offenders.join("\n")
    );
}

/// Git 工作区必须由实体持有：标签内容存 `Entity<GitWorkspace>`，旧的可变
/// 引用访问器不得复活。实体化后动作与状态同处，状态生命周期随标签页。
#[test]
fn git_workspace_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let types = std::fs::read_to_string(root.join("src/app/types.rs")).expect("读取 tags 类型失败");
    assert!(
        types.contains("Git(Entity<crate::git::GitWorkspace>)"),
        "TabContent::Git 必须持有 Entity<GitWorkspace>"
    );
    let state = std::fs::read_to_string(root.join("src/app/state.rs")).expect("读取 AppState 失败");
    for forbidden in [
        "fn git_state_of_tab(",
        "fn git_state_of_tab_mut(",
        "fn git_state_of(",
        "fn git_state_of_mut(",
    ] {
        assert!(
            !state.contains(forbidden) && !types.contains(forbidden),
            "旧的 Git 引用访问器不得复活：{forbidden}"
        );
    }
    let entity =
        std::fs::read_to_string(root.join("src/git/entity.rs")).expect("读取 Git 实体失败");
    assert!(
        entity.contains("pub(crate) struct GitWorkspace"),
        "GitWorkspace 实体必须存在于 git/entity.rs"
    );
}

/// 容器工作区必须由实体持有：标签内容存 `Entity<ContainerWorkspace>`，旧的
/// 可变引用访问器不得复活。实体化后状态生命周期随标签页。
#[test]
fn container_workspace_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let types = std::fs::read_to_string(root.join("src/app/types.rs")).expect("读取标签类型失败");
    assert!(
        types.contains("Containers(Entity<crate::container::ContainerWorkspace>)"),
        "TabContent::Containers 必须持有 Entity<ContainerWorkspace>"
    );
    let container = std::fs::read_to_string(root.join("src/container/mod.rs"))
        .expect("读取 container/mod.rs 失败");
    assert!(
        !container.contains("fn container_workspace_mut("),
        "旧的 container_workspace_mut 不得复活"
    );
    let entity =
        std::fs::read_to_string(root.join("src/container/entity.rs")).expect("读取容器实体失败");
    assert!(
        entity.contains("pub(crate) struct ContainerWorkspace"),
        "ContainerWorkspace 实体必须存在于 container/entity.rs"
    );
}

/// 已知主机弹窗必须由实体持有，旧的 `AppState` 刷新入口不得复活。
#[test]
fn known_hosts_modal_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("known_hosts: Entity<KnownHostsView>"),
        "ModalHub 的 known_hosts 必须持有 Entity<KnownHostsView>"
    );
    let view = std::fs::read_to_string(root.join("src/keys/known_hosts_view.rs"))
        .expect("读取已知主机实体失败");
    assert!(
        view.contains("pub(crate) struct KnownHostsView"),
        "KnownHostsView 实体必须存在于 keys/known_hosts_view.rs"
    );
    let shell = std::fs::read_to_string(root.join("src/known_hosts_modal.rs"))
        .expect("读取已知主机视图失败");
    assert!(
        !shell.contains("fn refresh_known_hosts(")
            && !shell.contains("fn render_known_hosts_body("),
        "已知主机的旧外壳入口不得复活"
    );
}

/// 备份弹窗必须由实体持有：状态、在途任务与渲染都归 `BackupPromptView`，
/// 旧的外壳提交入口与 `TaskHub` 任务槽不得复活。外壳只消费事件做目录合并。
#[test]
fn backup_prompt_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("backup_prompt: Entity<BackupPromptView>"),
        "ModalHub 的 backup_prompt 必须持有 Entity<BackupPromptView>"
    );
    let task_hub =
        std::fs::read_to_string(root.join("src/app/task_hub.rs")).expect("读取 task_hub.rs 失败");
    assert!(
        !task_hub.contains("backup: Option<Task<()>>"),
        "备份任务句柄必须归实体，TaskHub 不得保留 backup 槽位"
    );
    let view = std::fs::read_to_string(root.join("src/backup/view.rs")).expect("读取备份实体失败");
    assert!(
        view.contains("pub(crate) struct BackupPromptView")
            && view.contains("task: Option<Task<()>>"),
        "BackupPromptView 必须在 backup/view.rs 持有状态与任务句柄"
    );
    let backup_mod =
        std::fs::read_to_string(root.join("src/backup/mod.rs")).expect("读取 backup/mod.rs 失败");
    assert!(
        !backup_mod.contains("impl AppShell"),
        "备份弹窗的旧外壳方法不得复活，跨域收尾走 BackupPromptEvent"
    );
    let shell = std::fs::read_to_string(root.join("src/app/backup_prompt.rs"))
        .expect("读取备份外壳事件处理失败");
    assert!(
        shell.contains("fn on_backup_event(")
            && !shell.contains("fn submit_backup(")
            && !shell.contains("fn render_backup_prompt_body("),
        "外壳只允许保留打开入口与事件消费，不得重新接管提交或渲染"
    );
}

/// 密钥向导必须由实体持有：状态、在途生成任务与渲染都归 `KeyWizardView`，
/// 旧的外壳动作与 `TaskHub` 任务槽不得复活。外壳只消费事件并注入部署目标。
#[test]
fn key_wizard_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("key_wizard: Entity<KeyWizardView>"),
        "ModalHub 的 key_wizard 必须持有 Entity<KeyWizardView>"
    );
    let task_hub =
        std::fs::read_to_string(root.join("src/app/task_hub.rs")).expect("读取 task_hub.rs 失败");
    assert!(
        !task_hub.contains("keygen: Option<Task<()>>"),
        "密钥生成任务句柄必须归实体，TaskHub 不得保留 keygen 槽位"
    );
    let entity =
        std::fs::read_to_string(root.join("src/keys/entity.rs")).expect("读取密钥实体失败");
    assert!(
        entity.contains("pub(crate) struct KeyWizardView")
            && entity.contains("task: Option<Task<()>>"),
        "KeyWizardView 必须在 keys/entity.rs 持有状态与任务句柄"
    );
    let keys_mod =
        std::fs::read_to_string(root.join("src/keys/mod.rs")).expect("读取 keys/mod.rs 失败");
    assert!(
        !keys_mod.contains("impl AppShell"),
        "密钥向导的旧外壳方法不得复活，跨域提示走 KeyWizardEvent"
    );
    assert!(
        !root.join("src/keys/view.rs").exists(),
        "旧渲染文件 keys/view.rs 已迁入 keys/render.rs，不得复活"
    );
}

/// 会话表单必须由实体持有：打开状态、输入控件、保存/删除动作与渲染都归
/// `SessionFormView`，外壳只保留打开入口、目录/钥匙串落盘与串口数据注入。
#[test]
fn session_form_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("form: Entity<SessionFormView>"),
        "ModalHub 的会话表单必须持有 Entity<SessionFormView>"
    );
    let entity = std::fs::read_to_string(root.join("src/session_form/entity.rs"))
        .expect("读取会话表单实体失败");
    assert!(
        entity.contains("pub(crate) struct SessionFormView"),
        "SessionFormView 实体必须存在于 session_form/entity.rs"
    );
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root.join("src/session_form")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains("impl AppShell {") {
            offenders.push(normalized_path(&entry));
        }
    }
    assert!(
        offenders.is_empty(),
        "会话表单的旧外壳渲染/动作不得复活，跨域数据走 SessionFormEvent：\n{}",
        offenders.join("\n")
    );
    let ops = std::fs::read_to_string(root.join("src/workspace/tabs/session_ops.rs"))
        .expect("读取会话表单外壳事件处理失败");
    for forbidden in [
        "fn save_session_form(",
        "fn delete_session_form(",
        "fn update_session_form(",
    ] {
        assert!(
            !ops.contains(forbidden),
            "会话表单保存/删除必须走 SessionFormEvent：{forbidden}"
        );
    }
    let view = std::fs::read_to_string(root.join("src/session_form/view.rs"))
        .expect("读取会话表单视图失败");
    assert!(
        !view.contains("render_session_form_body"),
        "旧的外壳渲染入口不得复活，弹窗渲染归 SessionFormView::render_body"
    );
}

#[test]
fn audit_viewer_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("audit_viewer: Entity<AuditViewerView>"),
        "ModalHub 的 audit_viewer 必须持有 Entity<AuditViewerView>"
    );
    let entity =
        std::fs::read_to_string(root.join("src/audit_viewer/entity.rs")).expect("读取审计实体失败");
    assert!(
        entity.contains("pub(crate) struct AuditViewerView"),
        "AuditViewerView 必须存在于 audit_viewer/entity.rs"
    );
    assert!(
        !root.join("src/audit_viewer/ops.rs").exists()
            && !root.join("src/audit_viewer/view.rs").exists(),
        "审计查看器旧的 ops.rs/view.rs 应已删除"
    );
}

/// 回放选择弹窗必须由实体持有；录制启停与播放器打开仍可留在外壳。
#[test]
fn replay_modal_is_owned_by_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let hub =
        std::fs::read_to_string(root.join("src/app/modal_hub.rs")).expect("读取 modal_hub.rs 失败");
    assert!(
        hub.contains("replay: Entity<ReplayModalView>"),
        "ModalHub 的 replay 必须持有 Entity<ReplayModalView>"
    );
    let entity =
        std::fs::read_to_string(root.join("src/record/entity.rs")).expect("读取回放实体失败");
    assert!(
        entity.contains("pub(crate) struct ReplayModalView"),
        "ReplayModalView 必须存在于 record/entity.rs"
    );
}

