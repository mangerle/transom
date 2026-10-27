use super::stats::{DF_EVERY_N_ROUNDS, stats_sample_command};

/// 命令必须始终保留段落标记：解析器在缺标记时会回退全文扫描，
/// 省略 `__DF__`/`__NET__` 会把 /proc 内容误当 df 行解析。
#[test]
fn stats_sample_command_keeps_section_markers() {
    let lean = stats_sample_command(false, false);
    assert!(lean.contains("__UPTIME__"), "运行时长标记不可省略");
    assert!(lean.contains("__DF__"), "df 标记不可省略");
    assert!(lean.contains("__NET__"), "网卡标记不可省略");
    assert!(!lean.contains("__OS__"), "非首轮不应附带 os-release");
    assert!(!lean.contains("df -P -k"), "无 df 轮次不得启动 df 子进程");
}

/// 首轮与 df 轮次的可选段：OS 段与 df 命令按需出现。
#[test]
fn stats_sample_command_includes_optional_sections() {
    let full = stats_sample_command(true, true);
    assert!(full.contains("__OS__"), "首轮必须附带 os-release");
    assert!(full.contains("df -P -k"), "df 轮次必须执行 df 子进程");
}

/// 降频比例是 1/N 语义：第 0 轮带 df，随后 N-1 轮不带，周期恰好为 N。
#[test]
fn df_scheduling_cycles_every_n_rounds() {
    let with_df: Vec<u64> = (0..DF_EVERY_N_ROUNDS * 2)
        .filter(|round| round.is_multiple_of(DF_EVERY_N_ROUNDS))
        .collect();
    assert_eq!(
        with_df,
        vec![0, DF_EVERY_N_ROUNDS],
        "df 轮次应按固定周期出现"
    );
}
