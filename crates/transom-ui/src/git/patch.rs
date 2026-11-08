//! 提交补丁导出：默认文件名清洗与保存对话框。
//!
//! 设计原理（Why）：
//! - 先弹保存对话框再发命令：补丁正文可能上百 MB，让它走一遍事件总线
//!   再丢弃是纯浪费；用户取消对话框时连子进程都不该起。
//! - 默认名里的标题来自提交说明，属于不可信字符集（可能含 `/`、`..`、换行），
//!   因此清洗成单个文件名再交给对话框，避免把用户选定的目录顶掉。

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GitAction, GitCommand, GitProject, GitProjectId};

/// 清洗成可作文件名的补丁名：`<短号>-<标题slug>.patch`。
///
/// 设计原理（Why）：只保留可见且不含路径语义的字符——`/`、`\\` 会造出子目录，
/// 连续的点会被当成相对路径片段，换行会让对话框显示断裂。清洗后的名字仍然
/// 只作为对话框的默认值，用户改不改都不影响落盘位置的合法性。
pub(crate) fn patch_file_name(short_oid: &str, subject: &str) -> String {
    let mut slug = String::with_capacity(subject.len());
    for ch in subject.chars() {
        if slug.chars().count() >= 48 {
            break;
        }
        // Windows 不允许文件名里出现这些字符；应用要跨平台，一律换成短横。
        if ch.is_whitespace()
            || matches!(
                ch,
                '/' | '\\' | '.' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        {
            // 连续的被禁字符（如 `..`、`:` + 空格）合成一个短横，名字不至于全是分隔符。
            if !slug.ends_with('-') {
                slug.push('-');
            }
        } else if ch.is_control() {
            continue;
        } else {
            slug.push(ch);
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("patch");
    }
    format!("{short_oid}-{slug}.patch")
}

impl AppShell {
    /// 「创建补丁…」：问用户要写到哪里，然后交给控制器落盘。
    pub(crate) fn git_create_patch(
        &mut self,
        project: &GitProject,
        oid: &str,
        cx: &mut Context<Self>,
    ) {
        let (short_oid, subject) = self
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| {
                let state = &workspace.read(cx).state;
                state
                    .log
                    .entries
                    .iter()
                    .find(|entry| entry.oid == oid)
                    .map(|entry| (entry.short_oid().to_string(), entry.subject.clone()))
            })
            .unwrap_or_else(|| (oid.get(..8).unwrap_or(oid).to_string(), String::new()));
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("git.menu.create_patch").to_string())
            .set_file_name(patch_file_name(&short_oid, &subject))
            .add_filter("Patch", &["patch", "mbox"])
            .save_file()
        else {
            return;
        };
        if let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) {
            workspace.update(cx, |workspace, _| {
                workspace.state.patch_target = Some(path.clone());
            });
        }
        self.send_command(GitCommand::CreatePatch {
            project: project.clone(),
            oid: oid.to_string(),
            path,
        });
        cx.notify();
    }

    /// 补丁写成功后把状态栏换成落盘路径；没有待定路径时返回 `None` 让通用文案兜底。
    pub(crate) fn git_patch_done_status(
        &mut self,
        project_id: GitProjectId,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let workspace = self.state.git_workspace_of_project(project_id, cx)?;
        let path = workspace.update(cx, |workspace, _| workspace.state.patch_target.take())?;
        Some(rust_i18n::t!("git.patch.saved", path = path.display().to_string()).to_string())
    }

    /// 动作失败时也要清掉待定路径，否则下一次同类动作的成功会报出旧路径。
    pub(crate) fn git_patch_failed(
        &mut self,
        project_id: GitProjectId,
        action: GitAction,
        cx: &mut Context<Self>,
    ) {
        if action != GitAction::CreatePatch {
            return;
        }
        if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
            workspace.update(cx, |workspace, _| {
                workspace.state.patch_target = None;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::patch_file_name;

    #[test]
    fn slug_replaces_path_separators() {
        let name = patch_file_name("1a2b3c4", "fix: a/b\\c..d");
        assert_eq!(name, "1a2b3c4-fix-a-b-c-d.patch", "实际: {name}");
        for forbidden in ['/', '\\', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(
                !name.contains(forbidden),
                "文件名不应含 {forbidden}，实际: {name}"
            );
        }
    }

    #[test]
    fn slug_keeps_chinese_and_truncates_long_subject() {
        let long = "补".repeat(80);
        let name = patch_file_name("abc", &long);
        assert!(name.starts_with("abc-补"), "实际: {name}");
        // 48 个字符上限 + 前缀 + 扩展名，杜绝把一个百字标题整段塞进文件名。
        assert!(
            name.chars().count() <= 48 + 4 + 6,
            "实际长度: {}",
            name.chars().count()
        );
    }

    #[test]
    fn slug_falls_back_when_subject_is_unusable() {
        assert_eq!(patch_file_name("abc", "..."), "abc-patch.patch");
        assert_eq!(patch_file_name("abc", "\n\t "), "abc-patch.patch");
    }
}
