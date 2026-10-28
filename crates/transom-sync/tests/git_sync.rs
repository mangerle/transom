//! Git 端到端加密同步集成测试：使用本地裸仓库模拟私有远端。
//!
//! 验证三件事：远端只保存密文、两台设备的目录能合并、第二次同步能做
//! 三路合并带回远端新增条目。

use std::path::Path;
use std::process::Command;
use transom_core::{
    AuthMethod, Connection, GitSyncSettings, Session, SessionCatalog, SshConnection,
};
use transom_sync::GitAuth;
use transom_sync::sync_git_blocking;

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("启动 git 失败");
    assert!(status.success(), "git {args:?} 执行失败");
}

fn catalog_with(name: &str) -> SessionCatalog {
    let mut catalog = SessionCatalog::default();
    catalog.sessions.push(Session::new(
        name,
        Connection::Ssh(SshConnection {
            host: format!("{name}.example.com"),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Agent,
            options: Default::default(),
            proxy_jump: None,
        }),
    ));
    catalog
}

fn settings(remote: &Path) -> GitSyncSettings {
    GitSyncSettings {
        remote: remote.to_string_lossy().to_string(),
        branch: "transom-sync".into(),
        interval_minutes: 0,
    }
}

#[test]
fn git_sync_merges_two_devices_and_stores_only_ciphertext() {
    if !git_available() {
        eprintln!("跳过：宿主机未安装 git");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-q"]);

    let cache_a = dir.path().join("cache-a");
    let cache_b = dir.path().join("cache-b");
    let auth = GitAuth::default();
    let passphrase = "correct horse battery staple";

    let local_a = catalog_with("alpha");
    let outcome = sync_git_blocking(&settings(&remote), &local_a, passphrase, &cache_a, &auth)
        .expect("首次推送应成功");
    assert!(outcome.pushed);
    assert_eq!(outcome.catalog.sessions.len(), 1);
    // 缓存里是会话拓扑密文与共同祖先，目录与文件都必须只对属主可见。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&cache_a), 0o700, "同步缓存目录必须为 0700");
        assert_eq!(
            mode(&cache_a.join("transom-sync.envelope")),
            0o600,
            "同步缓存信封必须为 0600"
        );
    }

    let outcome = sync_git_blocking(
        &settings(&remote),
        &catalog_with("beta"),
        passphrase,
        &cache_b,
        &auth,
    )
    .expect("第二台设备合并应成功");
    let names: Vec<&str> = outcome
        .catalog
        .sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect();
    assert!(
        names.contains(&"alpha"),
        "合并结果应带回落端条目: {names:?}"
    );
    assert!(names.contains(&"beta"));
    assert_eq!(outcome.report.sessions_added, 1);

    // 远端对象必须是密文：明文主机名不得出现在仓库文件里。
    let envelope = std::fs::read(cache_b.join("transom-sync.envelope")).unwrap();
    assert!(
        !String::from_utf8_lossy(&envelope).contains("example.com"),
        "远端文件不得包含明文主机信息"
    );

    // 第一台设备再次同步：三路合并把 beta 带回来。
    let outcome = sync_git_blocking(&settings(&remote), &local_a, passphrase, &cache_a, &auth)
        .expect("再次同步应成功");
    let names: Vec<&str> = outcome
        .catalog
        .sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect();
    assert!(names.contains(&"beta"), "三路合并应带回远端新增: {names:?}");
    assert!(!outcome.pushed, "本地与远端一致时不应产生空提交");
}

#[test]
fn git_sync_rejects_wrong_passphrase() {
    if !git_available() {
        eprintln!("跳过：宿主机未安装 git");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "-q"]);
    let auth = GitAuth::default();

    sync_git_blocking(
        &settings(&remote),
        &catalog_with("alpha"),
        "right",
        &dir.path().join("cache-a"),
        &auth,
    )
    .expect("首次推送应成功");

    let error = sync_git_blocking(
        &settings(&remote),
        &catalog_with("beta"),
        "wrong",
        &dir.path().join("cache-b"),
        &auth,
    )
    .expect_err("错误口令必须被拒绝");
    assert!(error.to_string().contains("解密"), "错误: {error}");
}
