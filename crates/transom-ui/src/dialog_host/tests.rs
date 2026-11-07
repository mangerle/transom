//! 弹窗层序与枚举完整性测试。

use super::ModalKind;

/// 全部弹窗种类；新增变体时必须同步本表，顺序测试会校验 `ORDER` 不漏不重。
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
];

#[test]
fn order_covers_every_kind_exactly_once() {
    assert_eq!(
        ALL.len(),
        ModalKind::ORDER.len(),
        "ORDER 漏掉或多出弹窗种类"
    );
    for kind in ALL {
        let count = ModalKind::ORDER.iter().filter(|item| *item == kind).count();
        assert_eq!(count, 1, "弹窗种类 {:?} 在 ORDER 中必须恰好出现一次", kind);
    }
}
