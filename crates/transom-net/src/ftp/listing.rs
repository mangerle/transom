//! FTP 目录条目到领域契约的转换与时间换算。
//!
//! 设计原理（Why）：FTP 没有统一的目录列表格式，`MLSD`（RFC 3659）是机器可读
//! 的事实标准但老服务器不支持，`LIST` 输出则依赖服务端实现（UNIX ls 与 DOS
//! 两种主流形态）。转换集中在纯函数层做：解析交给 suppaftp 内建解析器，
//! 这里只负责把解析结果映射为 [`RemoteFile`] 契约、过滤不可信文件名、统一
//! 路径拼接与排序口径，与 SFTP 列表的行为完全一致，面板层无需感知协议差异。

use suppaftp::list::{File, ListParser};
use transom_core::{RemoteFile, is_safe_file_name};

/// 把 MLSD/LIST 的原始行转换为领域条目；不可信文件名整条跳过。
///
/// `dir` 是列表所属目录（以 `/` 结尾与否均可）；`permissions` 来自原始行里的
/// `UNIX.mode` fact（仅 MLSD 提供），POSIX 列表拿不到完整八进制权限时为 `None`。
pub(crate) fn convert_entry(line: &str, dir: &str, permissions: Option<u32>) -> Option<RemoteFile> {
    let file = parse_line(line)?;
    let name = file.name().to_string();
    if !is_safe_file_name(&name) {
        return None;
    }
    let modified = system_time_to_unix(file.modified());
    Some(RemoteFile {
        path: join_remote_path(dir, &name),
        is_dir: file.is_directory(),
        is_symlink: file.is_symlink(),
        size: file.size() as u64,
        modified,
        permissions,
        name,
    })
}

/// 解析一条原始列表行：先按 POSIX `ls -l` 与 DOS 形态匹配，最后尝试 MLSD。
///
/// 设计原理（Why）：POSIX 与 DOS 的格式约束非常严格，不会误匹配 MLSD 行；
/// 反过来 suppaftp 的 MLSD 解析对不含 `; ` 分隔的行有"取尾段当名字"的宽松
/// 兜底，会把 `total 24` 这类人类可读噪音行误收成条目，因此 MLSD 必须放在
/// 最后，且要求行内存在 facts/path 分隔符（RFC 3659 规定）或以空格开头。
fn parse_line(line: &str) -> Option<File> {
    ListParser::parse_posix(line)
        .or_else(|_| ListParser::parse_dos(line))
        .ok()
        .or_else(|| {
            if line.contains("; ") || line.starts_with(' ') {
                ListParser::parse_mlsd(line).ok()
            } else {
                None
            }
        })
}

/// 从 MLSD 行的 facts 中提取 `UNIX.mode` 八进制权限；缺失或非法返回 `None`。
pub(crate) fn mode_from_mlsx_facts(line: &str) -> Option<u32> {
    let facts = line.find("; ").map(|end| &line[..end]).unwrap_or(line);
    let mode = facts
        .split(';')
        .find_map(|fact| fact.strip_prefix("UNIX.mode="))?;
    u32::from_str_radix(mode, 8).ok()
}

/// 拼接远端路径：FTP 路径分隔符恒为 `/`，根目录不产生双斜杠。
pub(crate) fn join_remote_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// 把服务端时间（按 UTC 惯例）换算成 Unix 秒；早于纪元的异常值收敛为 0。
///
/// 采用 days-from-civil 算法（Howard Hinnant）做 civil 日期到相对天数的
/// 换算，避免引入额外的日期库依赖；MDTM/MLSD 的时间按 RFC 3659 一律是
/// UTC，本地时区只影响展示层。
pub(crate) fn civil_to_unix_seconds(
    year: i16,
    month: i8,
    day: i8,
    hour: i8,
    minute: i8,
    second: i8,
) -> u32 {
    let month = i64::from(month);
    let day = i64::from(day);
    // 三月纪年：一、二月算进上一年，月份序号从 3 月起记 0。
    let year = i64::from(if month <= 2 { year - 1 } else { year });
    let month_shift = (month + 9).rem_euclid(12);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * month_shift + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds =
        days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    u32::try_from(seconds.max(0)).unwrap_or(0)
}

/// 把 suppaftp 解析出的服务端时间收敛为 Unix 秒；早于纪元按 0 处理。
fn system_time_to_unix(time: std::time::SystemTime) -> Option<u32> {
    let seconds = time
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    u32::try_from(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::{civil_to_unix_seconds, convert_entry, join_remote_path, mode_from_mlsx_facts};

    /// days-from-civil 的换算必须与已知 UTC 时间点对齐。
    #[test]
    fn civil_time_converts_to_unix_seconds() {
        assert_eq!(civil_to_unix_seconds(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(civil_to_unix_seconds(2024, 11, 5, 9, 26, 0), 1_730_798_760);
        assert_eq!(civil_to_unix_seconds(2100, 1, 1, 0, 0, 0), 4_102_444_800);
        // 闰年 2 月 29 日与跨年边界不能出错。
        assert_eq!(civil_to_unix_seconds(2024, 2, 29, 12, 0, 0), 1_709_208_000);
        // 早于纪元与溢出都收敛为 0，不让异常服务端值变成巨大的回绕数。
        assert_eq!(civil_to_unix_seconds(1969, 12, 31, 23, 59, 59), 0);
        assert_eq!(civil_to_unix_seconds(3000, 1, 1, 0, 0, 0), 0);
    }

    #[test]
    fn joins_remote_paths_without_double_slash() {
        assert_eq!(join_remote_path("/home/u", "a.txt"), "/home/u/a.txt");
        assert_eq!(join_remote_path("/", "a.txt"), "/a.txt");
        assert_eq!(join_remote_path("/home/u/", "a.txt"), "/home/u/a.txt");
    }

    /// MLSD 行携带完整 facts：名字、目录标记、尺寸、修改时间与权限都要落进契约。
    #[test]
    fn converts_mlsd_entry_with_mode() {
        let line =
            "size=1234;type=file;modify=20241105092600;UNIX.mode=0644;UNIX.uid=1000; deploy.tar.gz";
        let entry = convert_entry(line, "/srv", mode_from_mlsx_facts(line)).expect("应可解析");
        assert_eq!(entry.name, "deploy.tar.gz");
        assert_eq!(entry.path, "/srv/deploy.tar.gz");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 1234);
        assert_eq!(entry.permissions, Some(0o644));
        assert_eq!(entry.modified, Some(1_730_798_760));
    }

    #[test]
    fn converts_mlsd_directory_entry() {
        let line = "type=dir;modify=20241105092600;UNIX.mode=0755; pub";
        let entry = convert_entry(line, "/", mode_from_mlsx_facts(line)).expect("应可解析");
        assert!(entry.is_dir);
        assert_eq!(entry.path, "/pub");
        assert_eq!(entry.permissions, Some(0o755));
    }

    /// POSIX `ls -l` 是老服务器的兜底形态；拿不到完整权限时按 None 降级。
    #[test]
    fn converts_posix_entry() {
        let line = "-rw-r--r-- 1 user group 1234 Nov 5 13:46 example.txt";
        let entry = convert_entry(line, "/home/u", None).expect("应可解析");
        assert_eq!(entry.name, "example.txt");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 1234);
        assert_eq!(entry.permissions, None);
        assert_eq!(entry.path, "/home/u/example.txt");
    }

    #[test]
    fn rejects_unsafe_and_unparseable_lines() {
        // 服务端控制的文件名绝不能带路径段，否则拼接后越界。
        assert!(convert_entry("-rw-r--r-- 1 u g 1 Nov 5 13:46 ../escape", "/srv", None).is_none());
        // total 行等非条目行静默跳过。
        assert!(convert_entry("total 24", "/srv", None).is_none());
        assert!(convert_entry("", "/srv", None).is_none());
    }

    #[test]
    fn extracts_mode_only_from_valid_octal_facts() {
        assert_eq!(
            mode_from_mlsx_facts("type=file;UNIX.mode=0755; a"),
            Some(0o755)
        );
        assert_eq!(mode_from_mlsx_facts("type=file;UNIX.mode=zzz; a"), None);
        assert_eq!(mode_from_mlsx_facts("type=file; a"), None);
        // MLST 返回的行 facts 与路径以 "; " 分隔，只统计事实段。
        assert_eq!(
            mode_from_mlsx_facts("type=file;UNIX.mode=0640; /a b;c"),
            Some(0o640)
        );
    }
}
