//! 弹窗注册表：层序、宽度、标识、打开判定与脏检查文本项的唯一来源。
//!
//! # 设计原理（Why）
//! 迁移前，一种弹窗的元数据分散在 `ORDER`、`spec`、`is_open`、`field_values`
//! 与容器/Git 子模块里，新增一种要改六到八处；漏改某一处不会编译失败，
//! 只会表现为“弹窗不出现”“脏检查失效”或层序错乱。
//!
//! 现在每个弹窗在 [`MODALS`] 里只登记一行：层序即表序，宽度与 slug 同处，
//! 打开判定与文本项指向 [`checks`] 里的具名函数。新增弹窗的改动点收敛为
//! “枚举加变体 + 本表加一行 + checks 补函数”，其余渲染/提交/取消仍按领域
//! 分发。
//!
//! # 代价与局限
//! 表是手写常量，枚举新增变体时表不会自动补齐；由
//! `registry_covers_every_kind_exactly_once` 测试兜底。宽度用三种形态表达
//! （固定档位、全局上限、视口比例），新增第四种形态时要先扩展 [`ModalWidth`]
//! 而不是在调用点特判。
use super::{ModalKind, ModalSpec};
use crate::app::AppShell;
use gpui_kit::{App, Pixels, Window};

mod checks;

/// 弹窗宽度形态。
#[derive(Clone, Copy)]
pub(super) enum ModalWidth {
    /// 以 rem 表达的固定档位，随密度缩放。
    Token(f32),
    /// 全局弹窗宽度上限，随密度缩放。
    Max,
    /// 视口宽度比例；只有差异对比使用。
    Viewport(f32),
}

impl ModalWidth {
    /// 按当前窗口换算为像素宽度。
    fn resolve(self, window: &Window) -> Pixels {
        match self {
            Self::Token(width) => window.rem_size() * width,
            Self::Max => window.rem_size() * crate::theme::tokens::size::DIALOG_MAX_WIDTH,
            Self::Viewport(ratio) => window.viewport_size().width * ratio,
        }
    }
}

/// 单个弹窗的注册信息。
pub(super) struct ModalEntry {
    pub(super) kind: ModalKind,
    pub(super) slug: &'static str,
    pub(super) width: ModalWidth,
    pub(super) is_open: fn(&AppShell, &App) -> bool,
    pub(super) field_values: fn(&AppShell, &App) -> Vec<String>,
}

/// 全部弹窗；顺序即层序，越靠后越在上层。
///
/// 设计原理（Why）：表序同时承担“期望栈顺序”职责，避免维护两份顺序数据。
/// 新增弹窗时插到语义相邻的位置，并确认遮挡关系仍然正确。
pub(super) static MODALS: &[ModalEntry] = &[
    ModalEntry {
        kind: ModalKind::SessionForm,
        slug: "session-form",
        width: ModalWidth::Token(crate::theme::tokens::dialog::SESSION_FORM),
        is_open: checks::session_form_open,
        field_values: checks::session_form_values,
    },
    ModalEntry {
        kind: ModalKind::AuditViewer,
        slug: "audit-viewer",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::audit_viewer_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::Replay,
        slug: "replay",
        width: ModalWidth::Token(crate::theme::tokens::dialog::REPLAY),
        is_open: checks::replay_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::FolderForm,
        slug: "folder-form",
        width: ModalWidth::Token(crate::theme::tokens::dialog::FOLDER_FORM),
        is_open: checks::folder_form_open,
        field_values: checks::folder_form_values,
    },
    ModalEntry {
        kind: ModalKind::AuthPrompt,
        slug: "auth",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUTH),
        is_open: checks::auth_prompt_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::HostKeyConfirm,
        slug: "host-key",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUTH),
        is_open: checks::host_key_confirm_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ClipboardPrompt,
        slug: "clipboard",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CLIPBOARD),
        is_open: checks::clipboard_prompt_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::KnownHosts,
        slug: "known-hosts",
        width: ModalWidth::Token(crate::theme::tokens::dialog::KNOWN_HOSTS),
        is_open: checks::known_hosts_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ForwardConfirm,
        slug: "forward",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CONFIRM),
        is_open: checks::forward_confirm_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::Settings,
        slug: "settings",
        width: ModalWidth::Token(crate::theme::tokens::size::SETTINGS_DIALOG_WIDTH),
        is_open: checks::settings_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ShortcutCapture,
        slug: "shortcut-capture",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUTH),
        is_open: checks::shortcut_capture_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::BatchEdit,
        slug: "batch-edit",
        width: ModalWidth::Token(crate::theme::tokens::dialog::BATCH_EDIT),
        is_open: checks::batch_edit_open,
        field_values: checks::batch_edit_values,
    },
    ModalEntry {
        kind: ModalKind::BackupPrompt,
        slug: "backup-prompt",
        width: ModalWidth::Token(crate::theme::tokens::dialog::BACKUP),
        is_open: checks::backup_prompt_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::KeyWizard,
        slug: "key-wizard",
        width: ModalWidth::Max,
        is_open: checks::key_wizard_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpConfirm,
        slug: "sftp-confirm",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CONFIRM),
        is_open: checks::sftp_confirm_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpChmod,
        slug: "sftp-chmod",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CHMOD),
        is_open: checks::sftp_chmod_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpInput,
        slug: "sftp-input",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::sftp_input_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpProperties,
        slug: "sftp-properties",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CHMOD),
        is_open: checks::sftp_properties_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpFilter,
        slug: "sftp-filter",
        width: ModalWidth::Token(crate::theme::tokens::dialog::FILTER),
        is_open: checks::sftp_filter_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpSync,
        slug: "sftp-sync",
        width: ModalWidth::Max,
        is_open: checks::sftp_sync_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::EditorGotoLine,
        slug: "editor-goto-line",
        width: ModalWidth::Token(crate::theme::tokens::dialog::EDITOR),
        is_open: checks::editor_goto_line_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::DiffViewer,
        slug: "diff-viewer",
        width: ModalWidth::Viewport(0.94),
        is_open: checks::diff_viewer_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerForm,
        slug: "container-form",
        width: ModalWidth::Token(crate::theme::tokens::dialog::FILTER),
        is_open: checks::container_form_open,
        field_values: checks::container_form_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerFolderForm,
        slug: "container-folder-form",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_folder_form_open,
        field_values: checks::container_folder_form_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerRunForm,
        slug: "container-run",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CONTAINER_RUN),
        is_open: checks::container_run_form_open,
        field_values: checks::container_run_form_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerCreate,
        slug: "container-create",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CONFIRM),
        is_open: checks::container_create_open,
        field_values: checks::container_create_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerConfirm,
        slug: "container-confirm",
        width: ModalWidth::Token(crate::theme::tokens::dialog::CONFIRM),
        is_open: checks::container_confirm_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerRename,
        slug: "container-rename",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_rename_open,
        field_values: checks::container_rename_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerTagImage,
        slug: "container-tag-image",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_tag_image_open,
        field_values: checks::container_tag_image_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerCommit,
        slug: "container-commit",
        width: ModalWidth::Token(crate::theme::tokens::dialog::FILTER),
        is_open: checks::container_commit_open,
        field_values: checks::container_commit_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerUpdate,
        slug: "container-update",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_update_open,
        field_values: checks::container_update_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerNetworkConnect,
        slug: "container-network-connect",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_network_connect_open,
        field_values: checks::container_network_connect_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerExportConfig,
        slug: "container-export-config",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::container_export_config_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerFileRename,
        slug: "container-file-rename",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_file_rename_open,
        field_values: checks::container_file_rename_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerFileNewDir,
        slug: "container-file-new-dir",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::container_file_new_dir_open,
        field_values: checks::container_file_new_dir_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerHub,
        slug: "container-hub",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::container_hub_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerPush,
        slug: "container-push",
        width: ModalWidth::Token(crate::theme::tokens::dialog::FILTER),
        is_open: checks::container_push_open,
        field_values: checks::container_push_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerSystemDf,
        slug: "container-system-df",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::container_system_df_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerBuild,
        slug: "container-build",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::container_build_open,
        field_values: checks::container_build_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerForward,
        slug: "container-forward",
        width: ModalWidth::Token(crate::theme::tokens::dialog::FILTER),
        is_open: checks::container_forward_open,
        field_values: checks::container_forward_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerPods,
        slug: "container-pods",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::container_pods_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::ContainerQuadlet,
        slug: "container-quadlet",
        width: ModalWidth::Token(crate::theme::tokens::dialog::AUDIT),
        is_open: checks::container_quadlet_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SftpSearch,
        slug: "sftp-search",
        width: ModalWidth::Token(crate::theme::tokens::dialog::SEARCH),
        is_open: checks::sftp_search_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::SnippetForm,
        slug: "snippet-form",
        width: ModalWidth::Token(crate::theme::tokens::dialog::SNIPPET_FORM),
        is_open: checks::snippet_form_open,
        field_values: checks::snippet_form_values,
    },
    ModalEntry {
        kind: ModalKind::GitProject,
        slug: "git-project",
        width: ModalWidth::Token(crate::theme::tokens::dialog::GIT_FORM),
        is_open: checks::git_project_open,
        field_values: checks::git_project_values,
    },
    ModalEntry {
        kind: ModalKind::GitClone,
        slug: "git-clone",
        width: ModalWidth::Token(crate::theme::tokens::dialog::GIT_CLONE),
        is_open: checks::git_clone_open,
        field_values: checks::git_clone_values,
    },
    ModalEntry {
        kind: ModalKind::GitWorktree,
        slug: "git-worktree",
        width: ModalWidth::Token(crate::theme::tokens::dialog::GIT_FORM),
        is_open: checks::git_worktree_open,
        field_values: checks::git_worktree_values,
    },
    ModalEntry {
        kind: ModalKind::GitName,
        slug: "git-name",
        width: ModalWidth::Token(crate::theme::tokens::dialog::INPUT),
        is_open: checks::git_name_open,
        field_values: checks::git_name_values,
    },
    ModalEntry {
        kind: ModalKind::GitRebase,
        slug: "git-rebase",
        width: ModalWidth::Token(crate::theme::tokens::dialog::GIT_FORM),
        is_open: checks::git_rebase_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::GitPush,
        slug: "git-push",
        width: ModalWidth::Token(crate::theme::tokens::dialog::GIT_PUSH),
        is_open: checks::git_push_open,
        field_values: checks::no_values,
    },
    ModalEntry {
        kind: ModalKind::GitConflict,
        slug: "git-conflict",
        width: ModalWidth::Token(crate::theme::tokens::dialog::GIT_CONFLICT),
        is_open: checks::git_conflict_open,
        field_values: checks::no_values,
    },
];

/// 取指定种类的注册信息。
///
/// # Panics
/// 注册表未覆盖该种类时 panic；`registry_covers_every_kind_exactly_once`
/// 测试保证静态表与枚举同步，新增变体必须同时登记。
pub(super) fn entry(kind: ModalKind) -> &'static ModalEntry {
    MODALS
        .iter()
        .find(|entry| entry.kind == kind)
        .expect("弹窗注册表必须覆盖全部 ModalKind；新增变体时同步 registry::MODALS")
}

/// 按层序返回全部弹窗种类。
pub(super) fn ordered_kinds() -> impl Iterator<Item = ModalKind> {
    MODALS.iter().map(|entry| entry.kind)
}

/// 计算弹窗的宽度与元素标识。
pub(super) fn spec(kind: ModalKind, window: &Window) -> ModalSpec {
    let entry = entry(kind);
    ModalSpec {
        width: entry.width.resolve(window),
        slug: entry.slug,
    }
}
