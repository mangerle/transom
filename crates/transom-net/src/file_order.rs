//! 远端文件列表的共享展示顺序。
//!
//! 设计原理（Why）：FTP 与 SFTP 的底层条目类型相同，面板又要求目录优先、
//! 名称忽略大小写且同键保持稳定。集中在这里可避免两套协议各自维护比较器，
//! 并用 `sort_by_cached_key` 把大小写键的堆分配从每次比较降为每项一次。

use transom_core::RemoteFile;

/// 按“目录优先、名称 ASCII 不区分大小写”稳定排序远端条目。
pub(crate) fn sort_remote_files(entries: &mut [RemoteFile]) {
    entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_ascii_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::sort_remote_files;
    use transom_core::RemoteFile;

    fn entry(name: &str, is_dir: bool) -> RemoteFile {
        RemoteFile {
            name: name.to_string(),
            path: format!("/{name}"),
            is_dir,
            is_symlink: false,
            size: 0,
            modified: None,
            permissions: None,
        }
    }

    #[test]
    fn sorts_directories_first_and_ignores_ascii_case() {
        let mut entries = vec![
            entry("beta", false),
            entry("Zulu", true),
            entry("Alpha", false),
            entry("alpha", true),
        ];

        sort_remote_files(&mut entries);

        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha", "Zulu", "Alpha", "beta"]);
    }

    #[test]
    fn keeps_input_order_for_equal_case_folded_keys() {
        let mut entries = vec![entry("b.txt", false), entry("B.txt", false)];

        sort_remote_files(&mut entries);

        assert_eq!(entries[0].name, "b.txt");
        assert_eq!(entries[1].name, "B.txt");
    }
}
