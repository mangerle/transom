//! 弹窗层序与枚举完整性测试。

use super::ModalKind;

/// 全部弹窗种类；新增变体时必须同步本表，注册表测试会校验不漏不重。
const ALL: &[ModalKind] = &[
    ModalKind::AuthPrompt,
    ModalKind::HostKeyConfirm,
    ModalKind::ClipboardPrompt,
    ModalKind::ForwardConfirm,
    ModalKind::SftpConfirm,
    ModalKind::SftpInput,
    ModalKind::SftpChmod,
    ModalKind::SftpProperties,
    ModalKind::SftpFilter,
    ModalKind::SftpSearch,
    ModalKind::SftpSync,
    ModalKind::EditorGotoLine,
    ModalKind::ContainerForm,
    ModalKind::ContainerFolderForm,
    ModalKind::ContainerRunForm,
    ModalKind::ContainerCreate,
    ModalKind::ContainerConfirm,
    ModalKind::ContainerRename,
    ModalKind::ContainerTagImage,
    ModalKind::ContainerCommit,
    ModalKind::ContainerUpdate,
    ModalKind::ContainerNetworkConnect,
    ModalKind::ContainerExportConfig,
    ModalKind::ContainerFileRename,
    ModalKind::ContainerFileNewDir,
    ModalKind::ContainerHub,
    ModalKind::ContainerPush,
    ModalKind::ContainerSystemDf,
    ModalKind::ContainerBuild,
    ModalKind::ContainerForward,
    ModalKind::ContainerPods,
    ModalKind::ContainerQuadlet,
    ModalKind::SessionForm,
    ModalKind::FolderForm,
    ModalKind::BatchEdit,
    ModalKind::SnippetForm,
    ModalKind::KnownHosts,
    ModalKind::KeyWizard,
    ModalKind::BackupPrompt,
    ModalKind::AuditViewer,
    ModalKind::Replay,
    ModalKind::Settings,
    ModalKind::ShortcutCapture,
    ModalKind::DiffViewer,
    ModalKind::GitProject,
    ModalKind::GitClone,
    ModalKind::GitWorktree,
    ModalKind::GitName,
    ModalKind::GitRebase,
    ModalKind::GitPush,
    ModalKind::GitConflict,
];

/// 危险或授权决策必须走 AlertDialog，不能退化成带关闭件的普通表单弹窗。
#[test]
fn consequential_decisions_use_alert_dialog() {
    for kind in [
        ModalKind::HostKeyConfirm,
        ModalKind::ClipboardPrompt,
        ModalKind::ForwardConfirm,
        ModalKind::SftpConfirm,
        ModalKind::ContainerConfirm,
    ] {
        assert!(
            kind.uses_alert_dialog(),
            "弹窗 {:?} 必须使用 AlertDialog",
            kind
        );
    }
    for kind in [
        ModalKind::AuthPrompt,
        ModalKind::SftpInput,
        ModalKind::ContainerForm,
        ModalKind::Settings,
    ] {
        assert!(
            !kind.uses_alert_dialog(),
            "普通表单弹窗 {:?} 不应使用 AlertDialog",
            kind
        );
    }
}

#[test]
fn registry_covers_every_kind_exactly_once() {
    assert_eq!(
        ALL.len(),
        super::registry::MODALS.len(),
        "注册表漏掉或多出弹窗种类"
    );
    for kind in ALL {
        let count = super::registry::MODALS
            .iter()
            .filter(|entry| entry.kind == *kind)
            .count();
        assert_eq!(count, 1, "弹窗种类 {:?} 在注册表中必须恰好出现一次", kind);
    }
}

/// 注册表内的 slug 必须唯一且非空：slug 用于弹窗元素标识，重复会让 GPUI
/// 在同一个 `ElementId` 下复用错误的状态，表现为焦点或输入串层。
#[test]
fn registry_slugs_are_unique_and_non_empty() {
    let mut slugs = Vec::with_capacity(super::registry::MODALS.len());
    for entry in super::registry::MODALS {
        assert!(
            !entry.slug.is_empty(),
            "弹窗 {:?} 的 slug 不能为空",
            entry.kind
        );
        assert!(
            !slugs.contains(&entry.slug),
            "弹窗 slug 重复: {}（种类 {:?}）",
            entry.slug,
            entry.kind
        );
        slugs.push(entry.slug);
    }
}
