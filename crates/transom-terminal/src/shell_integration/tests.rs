use super::*;
use transom_core::protocol::shell_integration::{
    BASH_INTEGRATION_SCRIPT, FISH_INTEGRATION_SCRIPT, POWERSHELL_INTEGRATION_SCRIPT,
    ZSH_INTEGRATION_SCRIPT,
};

fn ctx_at(now: Instant) -> IntegrationContext {
    IntegrationContext {
        now,
        output_started: true,
        output_idle: true,
        user_typed: false,
        alt_screen: false,
        exited: false,
    }
}

/// 推进到"探针已发出"的状态，供后续断言复用。
fn probing(integration: &mut ShellIntegration) {
    integration.step(&ctx_at(Instant::now()), false);
}

#[test]
fn test_detect_shell() {
    assert_eq!(detect_shell("/bin/bash"), ShellKind::Bash);
    assert_eq!(detect_shell("/usr/bin/zsh"), ShellKind::Zsh);
    assert_eq!(detect_shell("/usr/local/bin/fish"), ShellKind::Fish);
    assert_eq!(detect_shell("/bin/sh"), ShellKind::Other);
    assert_eq!(detect_shell("/usr/bin/BASH"), ShellKind::Bash);
    assert_eq!(
        detect_shell(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
        ShellKind::PowerShell
    );
    // 只看末段：目录名里带 bash 不能误判成 bash。
    assert_eq!(detect_shell("/opt/bash-tools/run"), ShellKind::Other);
}

#[test]
fn powershell_is_preinstalled_not_remotely_injected() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    probing(&mut integration);
    assert_eq!(
        integration.on_signal(
            &IntegrationSignal::ShellReport {
                shell: r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into(),
            },
            now,
        ),
        IntegrationAction::None
    );
    assert_eq!(integration.halt(), Some(IntegrationHalt::UnsupportedShell));
    assert_eq!(
        get_integration_script(ShellKind::PowerShell),
        Some(POWERSHELL_INTEGRATION_SCRIPT)
    );
}

#[test]
fn test_disabled_never_acts() {
    let mut integration = ShellIntegration::new(false);
    assert_eq!(integration.halt(), Some(IntegrationHalt::Disabled));
    assert_eq!(
        integration.step(&ctx_at(Instant::now()), false),
        IntegrationAction::None
    );
    assert!(integration.is_terminal());
}

#[test]
fn test_waits_for_safe_moment_then_probes() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    // 输出还没开始：不动。
    let mut ctx = ctx_at(now);
    ctx.output_started = false;
    assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
    // 用户在打字：写进去会拼到半截命令行上，不动。
    let mut ctx = ctx_at(now);
    ctx.user_typed = true;
    assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
    // 备屏程序里：探针会被当成程序输入，不动。
    let mut ctx = ctx_at(now);
    ctx.alt_screen = true;
    assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
    // 输出未停歇：提示符大概率还没落位，不动。
    let mut ctx = ctx_at(now);
    ctx.output_idle = false;
    assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
    // 全部满足：发探针。
    assert_eq!(
        integration.step(&ctx_at(now), false),
        IntegrationAction::Probe
    );
    // 已发出后不重复发。
    assert_eq!(
        integration.step(&ctx_at(now), false),
        IntegrationAction::None
    );
}

#[test]
fn test_existing_markers_skip_injection() {
    let mut integration = ShellIntegration::new(true);
    integration.step(&ctx_at(Instant::now()), true);
    assert_eq!(integration.halt(), Some(IntegrationHalt::PreIntegrated));
}

#[test]
fn test_probe_then_inject_then_active() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    assert_eq!(
        integration.step(&ctx_at(now), false),
        IntegrationAction::Probe
    );
    assert_eq!(
        integration.on_signal(
            &IntegrationSignal::ShellReport {
                shell: "/usr/bin/zsh".into()
            },
            now
        ),
        IntegrationAction::Inject(ShellKind::Zsh)
    );
    assert_eq!(
        get_integration_script(ShellKind::Zsh),
        Some(ZSH_INTEGRATION_SCRIPT)
    );
    assert_eq!(
        integration.on_signal(
            &IntegrationSignal::Ready {
                shell: "zsh".into()
            },
            now
        ),
        IntegrationAction::None
    );
    assert!(integration.is_active());
    assert_eq!(integration.shell_kind(), Some(ShellKind::Zsh));
    assert!(integration.is_terminal());
}

#[test]
fn test_unsupported_shell_halts_without_injecting() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    probing(&mut integration);
    assert_eq!(
        integration.on_signal(
            &IntegrationSignal::ShellReport {
                shell: "/bin/csh".into()
            },
            now
        ),
        IntegrationAction::None
    );
    assert_eq!(integration.halt(), Some(IntegrationHalt::UnsupportedShell));
    assert!(get_integration_script(ShellKind::Other).is_none());
}

#[test]
fn test_empty_shell_report_halts() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    probing(&mut integration);
    integration.on_signal(
        &IntegrationSignal::ShellReport {
            shell: "  ".to_string(),
        },
        now,
    );
    assert_eq!(integration.halt(), Some(IntegrationHalt::NoShellReport));
}

#[test]
fn test_probe_timeout_is_honored() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    integration.step(&ctx_at(now), false);
    // 探针发出但未超时：继续等，且不会二次发送。
    assert_eq!(
        integration.step(&ctx_at(now + Duration::from_secs(1)), false),
        IntegrationAction::None
    );
    integration.step(&ctx_at(now + REPLY_TIMEOUT), false);
    assert_eq!(integration.halt(), Some(IntegrationHalt::ProbeTimeout));
}

#[test]
fn test_wait_deadline_gives_up_before_probing() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    integration.started_at = now;
    integration.phase = IntegrationPhase::Waiting;
    let mut ctx = ctx_at(now + WAIT_DEADLINE);
    // 提示符迟迟不出现（输出一直不停歇），到期后放弃，不留常驻定时器。
    ctx.output_idle = false;
    assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
    assert_eq!(integration.halt(), Some(IntegrationHalt::ProbeTimeout));
}

#[test]
fn test_inject_without_ready_marks_failure() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    integration.step(&ctx_at(now), false);
    integration.on_signal(
        &IntegrationSignal::ShellReport {
            shell: "/bin/bash".into(),
        },
        now,
    );
    assert!(!integration.is_active(), "注入后未回报不得声称生效");
    integration.step(
        &ctx_at(now + REPLY_TIMEOUT + Duration::from_millis(1)),
        false,
    );
    assert_eq!(integration.halt(), Some(IntegrationHalt::InjectTimeout));
}

#[test]
fn test_ready_while_waiting_reuses_existing_integration() {
    let mut integration = ShellIntegration::new(true);
    integration.on_signal(
        &IntegrationSignal::Ready {
            shell: "fish".into(),
        },
        Instant::now(),
    );
    assert!(integration.is_active());
    assert_eq!(integration.shell_kind(), Some(ShellKind::Fish));
}

#[test]
fn test_out_of_order_signals_are_ignored() {
    // 尚未发探针就收到 shell 回报、或已终态后收到 ready，都不改变状态。
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    assert_eq!(
        integration.on_signal(
            &IntegrationSignal::ShellReport {
                shell: "/bin/bash".into()
            },
            now
        ),
        IntegrationAction::None
    );
    assert_eq!(integration.phase, IntegrationPhase::Waiting);
    integration.stop(IntegrationHalt::Disabled);
    assert_eq!(
        integration.on_signal(
            &IntegrationSignal::Ready {
                shell: "zsh".into()
            },
            now
        ),
        IntegrationAction::None
    );
    assert_eq!(integration.halt(), Some(IntegrationHalt::Disabled));
}

#[test]
fn test_exit_halts_any_phase() {
    let now = Instant::now();
    let mut integration = ShellIntegration::new(true);
    integration.step(&ctx_at(now), false);
    let mut ctx = ctx_at(now);
    ctx.exited = true;
    integration.step(&ctx, false);
    assert_eq!(integration.halt(), Some(IntegrationHalt::Exited));
}

#[test]
fn test_scripts_self_identify_and_report_ready() {
    // 每份脚本都必须：自带解释器判定、幂等保护、发 A/C/D 标记、回报 ready。
    for (script, ready) in [
        (BASH_INTEGRATION_SCRIPT, "ready=bash"),
        (ZSH_INTEGRATION_SCRIPT, "ready=zsh"),
        (FISH_INTEGRATION_SCRIPT, "ready=fish"),
    ] {
        assert!(script.contains("133;A"), "缺少提示符标记: {script}");
        assert!(script.contains("133;C"), "缺少执行标记: {script}");
        assert!(script.contains("133;D"), "缺少结束标记: {script}");
        assert!(script.contains("777;transom;"), "缺少 ready 回报: {script}");
        assert!(script.contains(ready));
        assert!(
            !script.ends_with('\n'),
            "脚本本体不带换行：注入路径要与清行命令拼成一条，预装路径按行写入: {script}"
        );
        assert_eq!(
            script.lines().count(),
            1,
            "多行脚本会被 Shell 当成多条命令逐行回显"
        );
    }
    assert!(BASH_INTEGRATION_SCRIPT.contains("BASH_VERSION"));
    assert!(ZSH_INTEGRATION_SCRIPT.contains("ZSH_VERSION"));
    assert!(BASH_INTEGRATION_SCRIPT.contains("TRANSOM_SI"));
}

#[test]
fn test_probe_command_shape() {
    let text = probe_command();
    assert!(
        !text.ends_with('\n'),
        "探针不带行结束：换行由注入载荷统一补"
    );
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("777;transom;shell=%s"));
    // 控制字节必须以 `\033` 形式交给 printf 解释，命令文本本身不含裸 ESC。
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\x07'));
}
