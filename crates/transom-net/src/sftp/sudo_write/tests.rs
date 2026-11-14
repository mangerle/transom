use super::*;

#[test]
fn test_posix_shell_quote() {
    assert_eq!(posix_shell_quote("hello"), "'hello'");
    assert_eq!(posix_shell_quote("nginx'conf"), r"'nginx'\''conf'");
}

#[test]
fn test_parent_directory() {
    assert_eq!(get_parent_directory("/etc/nginx/nginx.conf"), "/etc/nginx");
    assert_eq!(get_parent_directory("/hosts"), "/");
    assert_eq!(get_parent_directory("foo.txt"), ".");
}

#[test]
fn test_generate_same_dir_temp_path() {
    let temp = generate_same_dir_temp_path("/etc/nginx/nginx.conf", "1234");
    assert_eq!(temp, "/etc/nginx/.nginx.conf.transom_sudo_1234");
    assert_eq!(
        generate_same_dir_temp_path("/hosts", "5678"),
        "/.hosts.transom_sudo_5678"
    );
}

/// 备份后缀必须与普通写回路径一致，否则同一目录里会出现两套备份文件。
#[test]
fn test_backup_path_matches_plain_write_suffix() {
    assert_eq!(backup_path("/etc/hosts"), "/etc/hosts.transom-bak");
}

/// 脚本体内不含任何外部文本：路径与字节数一律走位置参数，注入面为零。
#[test]
fn test_script_references_positional_args_only() {
    let script = build_sudo_write_script();
    assert!(script.contains("dst=$1"));
    assert!(script.contains("tmp=$2"));
    assert!(script.contains("[ \"$got\" = \"$3\" ] || exit 41"));
    assert!(!script.contains("sudo "));
    assert!(script.contains("trap 'rm -f \"$tmp\"' EXIT"));
    // 长度闸门必须先于 mv：这是"中断不会截断生产文件"的唯一保证。
    let gate = script.find("exit 41").expect("长度闸门");
    let rename = script.find("mv -f").expect("原子替换");
    assert!(gate < rename, "长度闸门必须在替换之前");
}

/// 特殊字符路径被完整引用，且不会被拆成额外命令。
#[test]
fn test_command_quotes_path_with_injection_attempt() {
    let evil = "/etc/$(rm -rf '/') ; reboot";
    let command = build_sudo_write_command(evil, "/etc/.tmp", 42, true);
    assert!(command.starts_with("sudo -n -- sh -c '"), "{command}");
    // 参数以 `transom '<target>' '<tmp>' 42 1` 平铺，target 内部单引号已闭合转义。
    assert!(
        command.ends_with("transom '/etc/$(rm -rf '\\''/'\\'') ; reboot' '/etc/.tmp' 42 1"),
        "{command}"
    );
}

#[test]
fn test_command_backup_flag_and_expected_length() {
    let with_backup = build_sudo_write_command("/etc/hosts", "/etc/.t", 7, true);
    let without = build_sudo_write_command("/etc/hosts", "/etc/.t", 7, false);
    assert!(with_backup.ends_with("' 7 1"));
    assert!(without.ends_with("' 7 0"));
    // 备份意图只改最后一个位置参数，脚本体与其余参数必须完全一致。
    let script_part = |command: &str| {
        command
            .split(" transom ")
            .next()
            .unwrap_or(command)
            .to_string()
    };
    assert_eq!(
        script_part(&with_backup),
        script_part(&without),
        "备份意图不该改变脚本结构"
    );
}

/// stderr 尾部必须有界，且不能在多字节字符中间截断。
#[test]
fn test_stderr_tail_is_bounded_and_char_safe() {
    let mut tail = String::new();
    for _ in 0..200 {
        append_tail(&mut tail, "权限不足，无法写入。".as_bytes());
    }
    assert!(
        tail.len() <= STDERR_TAIL_BYTES + 32,
        "实际长度 {}",
        tail.len()
    );
    assert!(
        std::str::from_utf8(tail.as_bytes()).is_ok(),
        "必须仍是合法 UTF-8"
    );
}

/// 大正文的写入时限按块数放宽，但仍有封顶，不会退化成"没有超时"。
#[test]
fn test_feed_timeout_scales_with_size_and_caps() {
    assert_eq!(payload_feed_timeout(0), Duration::from_secs(0));
    assert_eq!(
        payload_feed_timeout(STDIN_CHUNK_SIZE * 5),
        Duration::from_secs(10)
    );
    assert_eq!(payload_feed_timeout(usize::MAX), Duration::from_secs(600));
}

/// 提权通道的命令必须区分"校验口令"与"写正文"两种时序。
#[test]
fn test_write_channel_never_asks_for_password() {
    // 正文通道固定用 -n：sudo 永不提示口令，stdin 100% 是文件内容。
    let command = build_sudo_write_command("/etc/hosts", "/etc/.t", 3, false);
    assert!(command.contains("sudo -n"));
    assert!(!command.contains("-S"));
}

#[test]
fn test_needs_password_detects_both_diagnostic_forms() {
    let mut outcome = PipedOutcome {
        stderr_tail: "sudo: a password is required".to_string(),
        ..Default::default()
    };
    assert!(outcome.needs_password());
    outcome.stderr_tail = "sudo: sorry, you must have a tty to run sudo".to_string();
    assert!(!outcome.needs_password(), "requiretty 不能被误判成缺口令");
}

#[test]
fn test_failure_text_keeps_exit_code_when_no_stderr() {
    let outcome = PipedOutcome {
        exit_status: Some(1),
        ..Default::default()
    };
    let error = outcome.failure("Sudo 提权写入失败");
    assert!(error.detail.unwrap_or_default().contains("退出码 1"));
}
