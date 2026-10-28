use super::parser::*;
use std::time::{Duration, Instant};

#[test]
fn test_cpu_jiffies_and_percent() {
    let sample1 = "cpu  1000 200 300 8000 100 50 20 0\ncpu0 500 100 150 4000 50 25 10 0\n";
    let (idle1, total1) = parse_cpu_jiffies(sample1).expect("Failed to parse cpu1");
    assert_eq!(idle1, 8100);
    assert_eq!(total1, 9670);

    let sample2 = "cpu  1500 200 400 8500 100 50 20 0\n";
    let (idle2, total2) = parse_cpu_jiffies(sample2).expect("Failed to parse cpu2");
    assert_eq!(idle2, 8600);
    assert_eq!(total2, 10770);

    let percent = calculate_cpu_percent((idle1, total1), (idle2, total2));
    assert!(percent > 54.0 && percent < 55.0);
}

#[test]
fn test_meminfo_and_uptime() {
    let mem =
        "MemTotal:       16384000 kB\nMemFree:         2000000 kB\nMemAvailable:   12000000 kB\n";
    let (used, total) = parse_meminfo(mem);
    assert_eq!(total, 16384000 * 1024);
    assert_eq!(used, 4384000 * 1024);

    let uptime = "123456.78 987654.32\n";
    assert_eq!(parse_uptime(uptime), 123456);

    let os = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nVERSION_ID=\"24.04\"\n";
    assert_eq!(parse_os_name(os), "Ubuntu 24.04.1 LTS");
}

#[test]
fn test_parse_uptime_in_combined_output() {
    let combined = "cpu  1000 200 300 8000 100 50 20 0\nMemTotal: 1000 kB\n185200.50 9999.00\nPRETTY_NAME=\"Debian\"\n";
    assert_eq!(parse_uptime(combined), 185200);

    let with_marker = "cpu  1000 200 300 8000 100 50 20 0\n__UPTIME__\n3600.25 1200.00\n__DF__\n";
    assert_eq!(parse_uptime(with_marker), 3600);
}

/// 多挂载点：只保留以 `/` 开头的真实挂载点，重复挂载点去重。
#[test]
fn parse_filesystems_keeps_real_mounts_only() {
    let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
/dev/sda1 1000 400 600 40% /\n\
tmpfs 200 10 190 5% /run\n\
/dev/sdb1 2000 500 1500 25% /data\n\
proc 0 0 0 - /proc\n\
/dev/sda1 1000 400 600 40% /";
    let filesystems = parse_filesystems(df);
    assert_eq!(filesystems.len(), 3);
    assert_eq!(filesystems[0].mount, "/");
    assert_eq!(filesystems[0].used_bytes, 400 * 1024);
    assert_eq!(filesystems[1].mount, "/run");
    assert_eq!(filesystems[2].mount, "/data");
}

/// 每网卡速率按两次累计快照与间隔计算；新网卡首轮速率为零。
#[test]
fn interface_rates_use_previous_snapshot() {
    let previous = vec![("eth0".to_string(), 1_000, 500)];
    let current = vec![
        ("eth0".to_string(), 3_000, 1_500),
        ("wlan0".to_string(), 800, 200),
    ];
    let rates = calculate_interface_rates(&previous, &current, Duration::from_secs(2));
    assert_eq!(rates.len(), 2);
    assert_eq!(rates[0].name, "eth0");
    assert_eq!(rates[0].rx_bytes_per_sec, 1_000);
    assert_eq!(rates[0].tx_bytes_per_sec, 500);
    assert_eq!(rates[1].rx_bytes_per_sec, 0, "首轮无前值应为 0");
}

/// 进程采样行按 `ps -eo pid,ppid,user,%cpu,%mem,rss,comm` 解析。
#[test]
fn parse_processes_reads_ps_columns() {
    let output = "  PID  PPID USER     %CPU %MEM   RSS COMMAND\n\
  1     0 root      0.0  0.1  1024 systemd\n\
4321  4000 deploy   42.5  3.2 65536 cargo build --release\n\
bad line\n";
    let processes = parse_processes(output);
    assert_eq!(processes.len(), 2);
    assert_eq!(processes[1].pid, 4321);
    assert_eq!(processes[1].ppid, 4000);
    assert_eq!(processes[1].user, "deploy");
    assert!((processes[1].cpu_percent - 42.5).abs() < f32::EPSILON);
    assert_eq!(processes[1].rss_bytes, 65536 * 1024);
    assert_eq!(processes[1].cmdline, "cargo build --release");
}

/// 每核使用率来自独立 jiffies 差；首轮没有前值时不输出伪造数据。
#[test]
fn parse_cpu_cores_and_rates() {
    let sample =
        "cpu  1000 0 500 8000 0 0 0 0\ncpu0 500 0 250 4000 0 0 0 0\ncpu1 500 0 250 4000 0 0 0 0\n";
    let cores = parse_cpu_cores(sample);
    assert_eq!(cores.len(), 2);

    let mut state = StatsSamplerState::default();
    let first = state.ingest(sample, Instant::now());
    assert!(first.cpu_cores.is_empty(), "首轮没有前值");

    let later = "cpu  2000 0 1000 16000 0 0 0 0\ncpu0 1000 0 500 8000 0 0 0 0\ncpu1 1000 0 500 8000 0 0 0 0\n";
    let second = state.ingest(later, Instant::now() + Duration::from_secs(2));
    assert_eq!(second.cpu_cores.len(), 2);
    assert!(second.cpu_cores.iter().all(|core| core.percent > 0.0));
}

#[test]
fn test_parse_disk() {
    let df_output = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/sda1        104857600  45000000  59857600      43% /\n";
    let (used, total) = parse_disk(df_output);
    assert_eq!(total, 104857600 * 1024);
    assert_eq!(used, 45000000 * 1024);

    let combined = "MemTotal: 100 kB\n__DF__\n文件系统 1024的块 已用 可用 容量 挂载点\n/dev/nvme0n1p6 200000000 150000000 50000000 75% /\n__OS__\n";
    let (used2, total2) = parse_disk(combined);
    assert_eq!(total2, 200000000 * 1024);
    assert_eq!(used2, 150000000 * 1024);
}
