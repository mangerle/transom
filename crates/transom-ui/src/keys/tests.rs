use super::{default_ssh_dir, expand_home};
use std::path::PathBuf;

/// `~` 展开是会话表单与密钥向导共用的口径，绝对路径必须原样通过。
#[test]
fn expand_home_handles_tilde_and_absolute_paths() {
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    let Some(home) = home else {
        // 拿不到家目录的环境属于可用降级：原样返回，不 panic。
        assert_eq!(expand_home("~/id_rsa"), PathBuf::from("~/id_rsa"));
        return;
    };
    assert_eq!(
        expand_home("~/.ssh/id_ed25519"),
        home.join(".ssh/id_ed25519")
    );
    assert_eq!(expand_home("  ~/id_rsa  "), home.join("id_rsa"));
    assert_eq!(expand_home("~"), home);
    assert_eq!(
        expand_home("/etc/ssh/host_key"),
        PathBuf::from("/etc/ssh/host_key")
    );
    assert_eq!(expand_home("./id_rsa"), PathBuf::from("./id_rsa"));
}

/// 默认密钥目录必须落在用户主目录的 `.ssh` 下，而不是当前工作目录。
#[test]
fn default_ssh_dir_targets_home_ssh() {
    assert!(default_ssh_dir().ends_with(".ssh"));
}
