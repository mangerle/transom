use super::archive::metadata_from_archive;
use super::listing::{parse_listing, relativize};
use super::validate_container_path;
use std::io::Cursor;
use transom_core::ContainerFileKind;

#[test]
fn path_validation_rejects_relative_and_parent_segments() {
    assert!(validate_container_path("etc/passwd").is_err());
    assert!(validate_container_path("/etc/../root").is_err());
    assert!(validate_container_path("/etc/\npasswd").is_err());
    assert_eq!(
        validate_container_path("/var//log/./").expect("合法路径"),
        "/var/log"
    );
    assert_eq!(validate_container_path("/").expect("根路径"), "/");
}

#[test]
fn relativize_strips_base_directory_name() {
    assert_eq!(
        relativize("etc/hostname", "etc").as_deref(),
        Some("hostname")
    );
    assert_eq!(relativize("etc/", "etc"), None);
    assert_eq!(relativize("bin/", "").as_deref(), Some("bin"));
    assert_eq!(
        relativize("etc/ssl/certs/ca.pem", "etc").as_deref(),
        Some("ssl/certs/ca.pem")
    );
}

#[test]
fn directory_upload_builds_tar_with_nested_files() {
    let directory = tempfile::tempdir().expect("临时目录");
    std::fs::create_dir(directory.path().join("sub")).expect("创建子目录");
    std::fs::write(directory.path().join("sub").join("app.txt"), b"hello").expect("写入测试文件");
    let (sender, receiver) = flume::bounded(64);
    let (progress, _progress_rx) = flume::bounded(8);
    let root = directory.path().to_path_buf();
    let builder = std::thread::spawn(move || {
        super::archive::build_dir_tar_stream(
            &root,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            &progress,
            sender,
        )
    });
    let mut bytes = Vec::new();
    while let Ok(chunk) = receiver.recv() {
        bytes.extend(chunk.expect("tar 块必须成功"));
    }
    builder
        .join()
        .expect("tar 构建线程")
        .expect("目录 tar 构建必须成功");
    let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
    let names: Vec<String> = archive
        .entries()
        .expect("tar 条目")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().expect("路径").display().to_string())
        .collect();
    assert!(names.iter().any(|name| name.ends_with("sub/app.txt")));
}

#[test]
fn file_mode_and_owner_validation_rejects_bad_input() {
    assert!(super::write::valid_file_mode("755"));
    assert!(super::write::valid_file_mode("0644"));
    assert!(!super::write::valid_file_mode("888"));
    assert!(!super::write::valid_file_mode("75"));
    assert!(super::write::valid_file_owner("root"));
    assert!(super::write::valid_file_owner("1000:1000"));
    assert!(!super::write::valid_file_owner(""));
    assert!(!super::write::valid_file_owner("root:"));
    assert!(!super::write::valid_file_owner("a:b:c"));
}

#[test]
fn archive_export_writes_raw_tar_and_cleans_temp_on_cancel() {
    let directory = tempfile::tempdir().expect("临时目录");
    let destination = directory.path().join("export.tar");
    let (sender, reader) = crate::bridge::output_channel(4);
    sender
        .send(Ok(b"tar-bytes".to_vec()))
        .expect("发送必须成功");
    drop(sender);
    let (progress, _receiver) = flume::bounded(4);
    let written = super::archive::write_archive_to_file(
        reader,
        "/data",
        &destination,
        &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        &progress,
    )
    .expect("导出必须成功");
    assert_eq!(written, 9);
    assert_eq!(std::fs::read(&destination).expect("读取导出"), b"tar-bytes");
}

#[test]
fn archive_metadata_keeps_mode_owner_and_mtime() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o750);
    header.set_uid(1000);
    header.set_gid(1001);
    header.set_mtime(1_700_000_000);
    header.set_cksum();
    builder
        .append_data(&mut header, "config", Cursor::new(b"data"))
        .expect("写入 tar 条目");
    let bytes = builder.into_inner().expect("结束 tar");
    let metadata = metadata_from_archive(Cursor::new(bytes)).expect("必须读到元数据");
    assert_eq!(metadata.mode, 0o750);
    assert_eq!(metadata.uid, 1000);
    assert_eq!(metadata.gid, 1001);
    assert_eq!(metadata.mtime, 1_700_000_000);
}

#[test]
fn listing_aggregates_direct_children_from_tar() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_mode(0o755);
    header.set_size(0);
    header.set_cksum();
    builder
        .append_data(&mut header, "etc/ssl/", Cursor::new(Vec::new()))
        .expect("写入目录条目");

    let mut file_header = tar::Header::new_gnu();
    file_header.set_entry_type(tar::EntryType::Regular);
    file_header.set_mode(0o644);
    file_header.set_size(5);
    file_header.set_cksum();
    builder
        .append_data(
            &mut file_header,
            "etc/hostname",
            Cursor::new(b"host\n".to_vec()),
        )
        .expect("写入文件条目");
    let bytes = builder.into_inner().expect("结束 tar");

    let entries = parse_listing(Cursor::new(bytes), "/etc").expect("解析目录");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "ssl");
    assert_eq!(entries[0].kind, ContainerFileKind::Directory);
    assert_eq!(entries[1].name, "hostname");
    assert_eq!(entries[1].kind, ContainerFileKind::File);
    assert_eq!(entries[1].size, 5);
    assert_eq!(entries[1].path, "/etc/hostname");
}
