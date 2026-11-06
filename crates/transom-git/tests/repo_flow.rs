//! 以真实 git 子进程驱动的黑盒集成测试。
//!
//! 设计原理（Why）：本 crate 的全部契约都建立在「git 实际输出」之上，
//! 单元测试里的样本是我手抄的，只有对真实仓库跑一遍才能证明解析器没在骗人。
//! 因此这里覆盖状态、历史、差异、分支与工作树探测五条主链路。

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use transom_git::{ChangeCode, HeadState, LogQuery, LogScope, Repo, Side, init, probe};

/// 在指定目录跑一次 git，失败即 panic 并带上 stderr。
///
/// 显式覆盖身份与签名配置：宿主机全局 `gitconfig` 可能开了 `commit.gpgsign`
/// 或设置了无法在本环境使用的作者身份，那会让测试与产品代码无关地失败。
fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=测试用户",
            "-c",
            "user.email=test@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("LC_ALL", "C")
        .output()
        .expect("测试环境必须能执行 git");
    assert!(
        output.status.success(),
        "git {args:?} 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// 建一个带一次提交的仓库。
fn repo_with_initial_commit() -> TempDir {
    let dir = TempDir::new().expect("临时目录可创建");
    git(dir.path(), &["init", "-q"]);
    // 身份写进仓库配置：库内的提交走产品代码路径，不会带上测试辅助函数的 -c 参数。
    git(dir.path(), &["config", "user.name", "测试用户"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.path().join("a.txt"), "line1\nline2\n").expect("写入文件");
    std::fs::create_dir_all(dir.path().join("sub")).expect("创建子目录");
    std::fs::write(dir.path().join("sub/b.txt"), "b\n").expect("写入子文件");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "初始提交: 中文标题"]);
    dir
}

#[tokio::test]
async fn status_reports_tracked_untracked_and_paths_with_spaces() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("a.txt"), "line1\nchanged\n").expect("修改文件");
    std::fs::write(dir.path().join("新 文件.txt"), "x\n").expect("写入带空格路径");

    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    assert_eq!(repo.root(), dir.path().canonicalize().unwrap());
    let status = repo.status(false).await.expect("状态采集成功");
    assert_eq!(status.head, HeadState::OnBranch("main".into()));
    assert!(status.oid.is_some(), "已有提交时应有 HEAD 号");
    let modified = status
        .entries
        .iter()
        .find(|entry| entry.path == "a.txt")
        .expect("应报告 a.txt");
    assert_eq!(
        modified.record,
        transom_git::StatusRecord::Tracked {
            index: ChangeCode::Unmodified,
            worktree: ChangeCode::Modified
        }
    );
    assert!(!modified.staged() && modified.unstaged());
    assert_eq!(status.untracked().len(), 1, "未跟踪文件应单独成条");
    assert_eq!(status.untracked()[0].path, "新 文件.txt");
}

#[tokio::test]
async fn stage_then_commit_then_read_history() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("sub/b.txt"), "b2\n").expect("修改文件");
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    repo.stage(&["sub/b.txt".to_string()])
        .await
        .expect("暂存成功");
    let staged = repo.status(false).await.expect("状态采集成功");
    assert_eq!(staged.staged().len(), 1, "暂存后应有一条待提交");
    assert_eq!(staged.staged()[0].path, "sub/b.txt");

    repo.commit("第二次提交\n\n正文说明", false)
        .await
        .expect("提交成功");
    let after = repo.status(false).await.expect("状态采集成功");
    assert!(
        after.entries.iter().all(|entry| !entry.staged()),
        "提交后索引应干净"
    );

    let page = repo
        .log(&LogQuery {
            scope: LogScope::CurrentBranch,
            ..LogQuery::default()
        })
        .await
        .expect("日志查询成功");
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].subject, "第二次提交");
    assert_eq!(page.entries[0].body, "正文说明");
    assert_eq!(page.entries[0].author_name, "测试用户");
    assert_eq!(page.entries[1].subject, "初始提交: 中文标题");
    assert_eq!(page.entries[1].parents, Vec::<String>::new(), "根提交无父");
    assert!(!page.has_more);

    let files = repo
        .commit_files(&page.entries[0].oid)
        .await
        .expect("提交文件清单成功");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "sub/b.txt");
    assert_eq!(files[0].change, ChangeCode::Modified);
}

#[tokio::test]
async fn rename_carries_old_path() {
    let dir = repo_with_initial_commit();
    git(dir.path(), &["mv", "a.txt", "renamed.txt"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let status = repo.status(false).await.expect("状态采集成功");
    let entry = status
        .entries
        .iter()
        .find(|entry| entry.path == "renamed.txt")
        .expect("重命名应报告新路径");
    assert_eq!(entry.old_path.as_deref(), Some("a.txt"));
}

#[tokio::test]
async fn diff_reads_both_sides_from_index_and_worktree() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("a.txt"), "line1\nworktree\n").expect("修改文件");
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    let working = repo
        .diff(&Side::Index, &Side::Worktree, "a.txt")
        .await
        .expect("工作区差异读取成功");
    assert_eq!(working.old_text, "line1\nline2\n");
    assert_eq!(working.new_text, "line1\nworktree\n");
    assert!(!working.binary && !working.truncated);

    // 新增文件在 HEAD 侧不存在，旧侧应为空串而不是报错。
    std::fs::write(dir.path().join("brand-new.txt"), "new\n").expect("写入新文件");
    let added = repo
        .diff(
            &Side::Commit("HEAD".into()),
            &Side::Worktree,
            "brand-new.txt",
        )
        .await
        .expect("新增文件差异读取成功");
    assert!(added.old_text.is_empty());
    assert_eq!(added.new_text, "new\n");

    let binary_path = dir.path().join("bin.dat");
    std::fs::write(&binary_path, [0u8, 1, 2, 0, 3]).expect("写入二进制文件");
    let binary = repo
        .diff(&Side::Commit("HEAD".into()), &Side::Worktree, "bin.dat")
        .await
        .expect("二进制差异读取成功");
    assert!(binary.binary, "含 NUL 的内容应标记为二进制");
}

#[tokio::test]
async fn branch_listing_create_and_switch() {
    let dir = repo_with_initial_commit();
    git(dir.path(), &["branch", "feature/x"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    let branches = repo.branches().await.expect("分支列表成功");
    let names: Vec<&str> = branches.iter().map(|branch| branch.name.as_str()).collect();
    assert!(names.contains(&"main") && names.contains(&"feature/x"));
    assert!(
        branches
            .iter()
            .any(|branch| branch.name == "main" && branch.current)
    );

    repo.create_branch("new-branch", Some("main"), true)
        .await
        .expect("新建并切换分支成功");
    let after = repo.status(false).await.expect("状态采集成功");
    assert_eq!(after.head, HeadState::OnBranch("new-branch".into()));
}

#[tokio::test]
async fn tags_list_lightweight_and_annotated() {
    let dir = repo_with_initial_commit();
    git(dir.path(), &["tag", "light"]);
    git(dir.path(), &["tag", "-a", "annot", "-m", "附注说明"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let tags = repo.tags().await.expect("标签列表成功");
    let light = tags
        .iter()
        .find(|tag| tag.name == "light")
        .expect("轻量标签");
    let annot = tags
        .iter()
        .find(|tag| tag.name == "annot")
        .expect("附注标签");
    assert_eq!(light.message, None);
    assert_eq!(annot.message.as_deref(), Some("附注说明"));
    assert_eq!(
        light.target, annot.target,
        "两者指向同一提交，附注标签需解引用"
    );
}

#[tokio::test]
async fn unstage_in_repo_without_commits_uses_index_removal() {
    let dir = TempDir::new().expect("临时目录");
    git(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("only.txt"), "x\n").expect("写入文件");
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let initial = repo.status(false).await.expect("状态采集成功");
    assert!(
        matches!(initial.head, HeadState::Initial { .. }),
        "空仓库应报告 Initial 而不是伪造分支"
    );

    repo.stage_all().await.expect("暂存成功");
    assert_eq!(repo.status(false).await.expect("状态").staged().len(), 1);
    // 没有 HEAD 时 `restore --staged` 会失败，本层必须自动改用 rm --cached。
    repo.unstage(&["only.txt".to_string()])
        .await
        .expect("空仓库取消暂存成功");
    let after = repo.status(false).await.expect("状态采集成功");
    assert!(after.staged().is_empty());
    assert_eq!(after.untracked().len(), 1, "文件应回到未跟踪状态");
}

#[tokio::test]
async fn stash_round_trip_keeps_selector_stable() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("a.txt"), "line1\nstashed\n").expect("修改文件");
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    assert!(repo.stashes().await.expect("初始无搁置").is_empty());
    repo.stash_save("我的搁置", false)
        .await
        .expect("新建搁置成功");
    let entries = repo.stashes().await.expect("搁置列表成功");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].selector, "stash@{0}");
    assert_eq!(entries[0].display(), "我的搁置");
    // 搁置后工作区应回到干净状态。
    assert!(
        repo.status(false)
            .await
            .expect("状态采集成功")
            .unstaged()
            .is_empty()
    );

    // apply 保留栈项，pop 移除栈项。
    repo.stash_apply("stash@{0}").await.expect("应用成功");
    assert_eq!(
        repo.stashes().await.expect("列表").len(),
        1,
        "应用不应吃掉栈项"
    );
    assert_eq!(
        repo.status(false).await.expect("状态").unstaged().len(),
        1,
        "应用后改动应回到工作区"
    );
    // 弹出前先把刚应用出来的改动丢掉：同一份改动应用两次是 git 会拒绝的冲突，
    // 这不是被实现的缺陷，而是测试要还原到"干净工作区"这一步。
    repo.discard(&["a.txt".to_string()])
        .await
        .expect("丢弃改动成功");
    repo.stash_pop("stash@{0}").await.expect("弹出成功");
    assert!(
        repo.stashes().await.expect("列表").is_empty(),
        "弹出后栈应空"
    );
    assert_eq!(
        repo.status(false).await.expect("状态").unstaged().len(),
        1,
        "弹出后改动应回到工作区且栈项消失"
    );
}

#[tokio::test]
async fn stash_files_lists_tracked_and_untracked_sides() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("a.txt"), "line1\nstashed\n").expect("修改文件");
    std::fs::write(dir.path().join("extra.txt"), "新文件\n").expect("写入未跟踪文件");
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    repo.stash_save("带未跟踪", true)
        .await
        .expect("连未跟踪一起搁置成功");
    let files = repo
        .stash_files("stash@{0}")
        .await
        .expect("搁置清单读取成功");
    let names: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    assert!(names.contains(&"a.txt"), "实际: {names:?}");
    assert!(
        names.contains(&"extra.txt"),
        "未跟踪部分必须出现在清单里，实际: {names:?}"
    );

    let modified = files
        .iter()
        .find(|file| file.path == "a.txt")
        .expect("已跟踪改动在列");
    assert_eq!(modified.change, ChangeCode::Modified);
    assert!(!modified.untracked, "已跟踪改动住在搁置树里");

    let added = files
        .iter()
        .find(|file| file.path == "extra.txt")
        .expect("未跟踪文件在列");
    assert_eq!(added.change, ChangeCode::Added);
    assert!(added.untracked, "未跟踪条目的内容在第三个父提交里");
}

#[tokio::test]
async fn fold_tip_into_parent_squashes_and_refuses_unsafe_states() {
    let dir = repo_with_initial_commit();
    // 第二个提交：改 a.txt 并留一条说明。
    std::fs::write(dir.path().join("a.txt"), "line1\n第二行\n").expect("修改文件");
    git(dir.path(), &["commit", "-aqm", "第二个提交"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    // 压缩：两条说明串接成一条。
    repo.fold_tip_into_parent(transom_core::GitFoldMode::Squash)
        .await
        .expect("压缩应成功");
    assert_eq!(
        git(dir.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "1",
        "压缩后应只剩一条提交"
    );
    let message = git(dir.path(), &["log", "-1", "--format=%B"]);
    assert!(
        message.contains("初始提交: 中文标题") && message.contains("第二个提交"),
        "压缩应保留两条说明，实际: {message}"
    );

    // 优选：再来一条，折叠后只留父提交说明。
    std::fs::write(dir.path().join("sub/b.txt"), "fixed\n").expect("修改子文件");
    git(dir.path(), &["commit", "-aqm", "修个笔误"]);
    repo.fold_tip_into_parent(transom_core::GitFoldMode::Fixup)
        .await
        .expect("优选应成功");
    let after_fixup = git(dir.path(), &["log", "-1", "--format=%B"]);
    assert!(
        !after_fixup.contains("修个笔误"),
        "优选应丢掉尾部说明，实际: {after_fixup}"
    );
    assert_eq!(
        git(dir.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "1",
        "优选后仍应只有一条提交"
    );
    // 内容不能丢：优选只改历史形状，不改树。
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sub/b.txt")).expect("读子文件"),
        "fixed\n"
    );

    // 暂存区不干净时拒绝：否则用户准备提交的内容会被一起吞进父提交。
    std::fs::write(dir.path().join("c.txt"), "新文件\n").expect("写入文件");
    git(dir.path(), &["add", "c.txt"]);
    git(dir.path(), &["commit", "-qm", "第三条"]);
    std::fs::write(dir.path().join("c.txt"), "改动未提交\n").expect("再次修改");
    git(dir.path(), &["add", "c.txt"]);
    let err = repo
        .fold_tip_into_parent(transom_core::GitFoldMode::Squash)
        .await
        .expect_err("暂存区脏时必须拒绝");
    assert!(
        matches!(err, transom_git::GitError::Precondition { .. }),
        "应是前置条件拒绝，实际: {err}"
    );

    // 只有一个提交（无父提交）时同样拒绝。
    let single = repo_with_initial_commit();
    let single_repo = Repo::open(single.path()).await.expect("打开仓库");
    assert!(
        matches!(
            single_repo
                .fold_tip_into_parent(transom_core::GitFoldMode::Squash)
                .await,
            Err(transom_git::GitError::Precondition { .. })
        ),
        "根提交没有父提交，必须拒绝"
    );

    // 游离头指针时拒绝：折叠会直接丢掉原头提交的引用。
    git(dir.path(), &["reset", "-q", "--hard"]);
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "-q", "--detach", head.trim()]);
    let detached = Repo::open(dir.path()).await.expect("重新打开仓库");
    assert!(
        matches!(
            detached
                .fold_tip_into_parent(transom_core::GitFoldMode::Fixup)
                .await,
            Err(transom_git::GitError::Precondition { .. })
        ),
        "游离头指针必须拒绝折叠"
    );
}

#[tokio::test]
async fn worktree_lock_round_trip_decodes_reason_and_blocks_removal() {
    let dir = repo_with_initial_commit();
    let linked = TempDir::new().expect("临时工作树目录可创建");
    let linked_path = linked.path().to_str().expect("路径可入参").to_string();
    git(
        dir.path(),
        &["worktree", "add", "--detach", "-q", &linked_path],
    );
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    // 中文原因在 porcelain 输出里是八位转义，解析层必须还原成可读文本。
    repo.lock_worktree(&linked_path, Some("临时保留"))
        .await
        .expect("锁定成功");
    let entries = repo.worktrees().await.expect("工作树列表成功");
    let locked = entries
        .iter()
        .find(|entry| entry.path.display().to_string() == linked_path)
        .expect("锁定的工作树应在列");
    assert!(locked.locked, "锁定态应被识别");
    assert_eq!(
        locked.lock_reason.as_deref(),
        Some("临时保留"),
        "锁定原因应还原为中文，实际: {:?}",
        locked.lock_reason
    );

    // 锁住时删除必须被 git 拒绝；不解锁就删是数据事故。
    let err = repo
        .remove_worktree(&linked_path, false)
        .await
        .expect_err("锁定的工作树不应被删除");
    assert!(
        matches!(err, transom_git::GitError::Failed { .. }),
        "应收到 git 的拒绝，实际: {err}"
    );

    repo.unlock_worktree(&linked_path).await.expect("解锁成功");
    // 解锁后是干净的游离头工作树，普通删除就够，不需要强推式删除。
    repo.remove_worktree(&linked_path, false)
        .await
        .expect("解锁后应可删除");
    let after = repo.worktrees().await.expect("工作树列表成功");
    assert!(
        after
            .iter()
            .all(|entry| entry.path.display().to_string() != linked_path),
        "删除后不应再出现在列表里"
    );
}

#[tokio::test]
async fn push_up_to_uses_upstream_remote_and_rejects_non_fast_forward() {
    let dir = repo_with_initial_commit();
    let bare = TempDir::new().expect("临时远端目录可创建");
    git(bare.path(), &["init", "-q", "--bare", "."]);
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            bare.path().to_str().expect("路径可入参"),
        ],
    );
    // 分支名由宿主机的 init.defaultBranch 决定，不能写死 master。
    let branch = git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();
    git(dir.path(), &["push", "-q", "-u", "origin", &branch]);

    // 再加两条提交，然后只推「到此提交为止」的历史。
    std::fs::write(dir.path().join("later.txt"), "后加的\n").expect("写入文件");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "后续提交"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let head = repo
        .resolve("HEAD")
        .await
        .expect("解析 HEAD 成功")
        .expect("头指针应存在");
    let first = repo
        .resolve("HEAD^")
        .await
        .expect("解析父提交成功")
        .expect("父提交应存在");

    repo.push_up_to(&first, &branch)
        .await
        .expect("推送到上游远端应成功");
    let pushed = git(bare.path(), &["rev-parse", &format!("refs/heads/{branch}")]);
    assert_eq!(pushed.trim(), first.as_str(), "远端分支应停在所选提交上");

    // 先把远端推进到最新提交，再往回推更早的提交：不是快进，git 自己拒绝，
    // 本层不加 `--force`，也不在客户端重复祖先判断。
    repo.push_up_to(&head, &branch)
        .await
        .expect("推到最新提交应成功");
    let err = repo
        .push_up_to(&first, &branch)
        .await
        .expect_err("往回推必须被拒绝");
    assert!(
        matches!(err, transom_git::GitError::Failed { .. }),
        "应收到 git 的失败退出，实际: {err}"
    );
}

#[tokio::test]
async fn commit_url_uses_origin_remote_when_present() {
    let dir = repo_with_initial_commit();
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let head = repo
        .resolve("HEAD")
        .await
        .expect("解析 HEAD 成功")
        .expect("头指针应存在");

    // 没有任何远端时如实返回 None，界面据此提示而不是猜地址。
    assert_eq!(
        repo.commit_url(&head).await.expect("取址成功"),
        None,
        "无远端不应有网页地址"
    );

    // 两个远端并存时优先 origin，与 IDEA 只浏览主远端的口径一致。
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "mirror",
            "ssh://git@m.internal:29418/t/m.git",
        ],
    );
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:someone/x.git"],
    );
    let expected = format!("https://github.com/someone/x/commit/{head}");
    assert_eq!(
        repo.commit_url(&head).await.expect("取址成功").as_deref(),
        Some(expected.as_str()),
    );
}

#[tokio::test]
async fn rename_branch_moves_ref_and_rejects_bad_names() {
    let dir = repo_with_initial_commit();
    git(dir.path(), &["branch", "feature"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    repo.rename_branch("feature", "feature-v2")
        .await
        .expect("重命名应成功");
    let listed = git(dir.path(), &["branch", "--format=%(refname:short)"]);
    assert!(listed.contains("feature-v2"), "实际: {listed}");
    assert!(
        !listed.split_whitespace().any(|name| name == "feature"),
        "旧名应消失，实际: {listed}"
    );

    // 非法名字在边界上拒绝，不能变成 git 的选项或畸形引用。
    for bad in ["-m", "有 空格", "a..b", "~/x", "end."] {
        let err = repo
            .rename_branch("feature-v2", bad)
            .await
            .expect_err("非法名字必须被拒绝");
        assert!(
            matches!(err, transom_git::GitError::InvalidPath { .. }),
            "应为名字校验错误，实际: {err}"
        );
    }
    // 被拒后分支没被改名。
    let after = git(dir.path(), &["branch", "--format=%(refname:short)"]);
    assert!(after.contains("feature-v2"), "实际: {after}");
}

#[tokio::test]
async fn rebase_todo_lists_range_and_applies_edited_plan() {
    let dir = repo_with_initial_commit();
    std::fs::write(
        dir.path().join("one.txt"),
        "一
",
    )
    .expect("写入文件");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "第一条"]);
    std::fs::write(
        dir.path().join("two.txt"),
        "二
",
    )
    .expect("写入文件");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "第二条"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let base = repo
        .resolve("HEAD~2")
        .await
        .expect("解析基线成功")
        .expect("基线应存在");

    let mut steps = repo.rebase_todo(&base).await.expect("清单读取成功");
    assert_eq!(steps.len(), 2, "基线之后应有两条提交");
    assert_eq!(steps[0].subject, "第一条", "清单顺序必须是旧→新");
    assert_eq!(steps[1].subject, "第二条");
    assert_eq!(steps[0].action, transom_core::GitRebaseAction::Pick);

    // 把第二条压进第一条：变基后基线之上只剩一条提交，说明含两条标题。
    steps[1].action = transom_core::GitRebaseAction::Squash;
    repo.rebase_with_todo(&base, &steps)
        .await
        .expect("按清单变基应成功");
    assert_eq!(
        git(dir.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "2",
        "基线 + 合并后的一条"
    );
    let message = git(dir.path(), &["log", "-1", "--format=%B"]);
    assert!(
        message.contains("第一条") && message.contains("第二条"),
        "squash 应合并两条说明，实际: {message}"
    );
    assert!(
        std::fs::read_to_string(dir.path().join("one.txt")).is_ok()
            && std::fs::read_to_string(dir.path().join("two.txt")).is_ok(),
        "两条提交的改动都要在"
    );

    // 非法基线（带选项前缀）必须被拒，不能变成 git 的参数。
    assert!(repo.rebase_todo("--exec=rm").await.is_err());
}

#[tokio::test]
async fn commit_patch_carries_subject_and_diff_body() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("a.txt"), "line1\nchanged\n").expect("修改文件");
    git(dir.path(), &["commit", "-aqm", "改动一行"]);
    let repo = Repo::open(dir.path()).await.expect("打开仓库");

    let head = repo
        .resolve("HEAD")
        .await
        .expect("解析 HEAD 成功")
        .expect("头指针应存在");
    let text = String::from_utf8(repo.commit_patch(&head).await.expect("补丁生成成功"))
        .expect("补丁是 UTF-8");
    assert!(text.contains("Subject: [PATCH]"), "实际: {text}");
    assert!(
        text.contains("-line2\n+changed"),
        "补丁应内联正文差异，实际: {text}"
    );

    // 根提交没有父提交，`-1` 仍然出得来完整补丁。
    let root = repo
        .resolve("HEAD^")
        .await
        .expect("解析父提交成功")
        .expect("父提交应存在");
    let root_text = String::from_utf8(repo.commit_patch(&root).await.expect("根提交补丁成功"))
        .expect("补丁是 UTF-8");
    // 非 ASCII 标题走 RFC2047 编码（`=?UTF-8?q?...=`），`git am` 自己会解回，
    // 因此这里只断言补丁骨架，不把编码形式写死成产品契约。
    assert!(root_text.contains("Subject: [PATCH]"), "实际: {root_text}");
    assert!(
        root_text.contains("create mode 100644 a.txt"),
        "根提交补丁应包含新建模式，实际: {root_text}"
    );

    assert!(
        repo.commit_patch("不存在的提交号").await.is_err(),
        "坏提交号必须报错而不是产出空补丁"
    );
}

#[tokio::test]
async fn tracked_files_lists_repository_content() {
    let dir = repo_with_initial_commit();
    std::fs::write(dir.path().join("not-tracked.txt"), "x\n").expect("写入未跟踪文件");
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    let files = repo.tracked_files().await.expect("跟踪文件清单成功");
    assert!(files.contains(&"a.txt".to_string()), "实际: {files:?}");
    assert!(files.contains(&"sub/b.txt".to_string()));
    assert!(
        !files.contains(&"not-tracked.txt".to_string()),
        "未跟踪文件不应出现在清单里"
    );
}

#[tokio::test]
async fn opening_a_plain_directory_reports_not_a_repo() {
    let dir = TempDir::new().expect("临时目录");
    let error = Repo::open(dir.path()).await.expect_err("普通目录不是仓库");
    assert!(matches!(error, transom_git::GitError::NotRepo { .. }));
    assert!(error.is_not_repo());
}

#[tokio::test]
async fn paths_escaping_the_repository_are_rejected() {
    let dir = repo_with_initial_commit();
    let repo = Repo::open(dir.path()).await.expect("打开仓库");
    for attempt in ["../outside.txt", "/etc/passwd", "ok/../../escape"] {
        let error = repo
            .resolve_path(attempt)
            .expect_err("越界路径必须在访问前拒绝");
        assert!(
            matches!(error, transom_git::GitError::InvalidPath { .. }),
            "实际错误: {error}"
        );
    }
    assert!(repo.resolve_path("sub/b.txt").is_ok());
}

#[tokio::test]
async fn linked_worktree_is_detected() {
    let dir = repo_with_initial_commit();
    let worktree = TempDir::new().expect("临时目录");
    let target = worktree.path().join("wt");
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            target.to_str().expect("临时路径应为合法 UTF-8"),
        ],
    );
    let repo = Repo::open(&target).await.expect("打开链接工作树");
    let info = probe(&target).await.expect("探测链接工作树");
    assert!(info.linked_worktree, "链接工作树必须被识别");
    assert_eq!(info.toplevel, target.canonicalize().unwrap());
    assert_eq!(
        repo.status(false).await.expect("状态采集成功").head,
        HeadState::OnBranch("wt-branch".into())
    );
    assert!(!probe(dir.path()).await.expect("探测主仓库").linked_worktree);
}

#[tokio::test]
async fn init_creates_a_usable_repository() {
    let dir = TempDir::new().expect("临时目录");
    let target = dir.path().join("fresh");
    std::fs::create_dir(&target).expect("创建目录");
    let repo = init(&target).await.expect("初始化仓库");
    assert!(
        repo.status(false)
            .await
            .expect("新仓库可采集状态")
            .oid
            .is_none()
    );
    assert!(repo.resolve("HEAD").await.expect("解析 HEAD").is_none());
}
