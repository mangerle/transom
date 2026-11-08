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
