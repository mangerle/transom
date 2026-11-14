use super::{HostSampler, ProcessSampler, is_userland_thread};
use sysinfo::{System, ThreadKind};

#[test]
fn userland_threads_are_not_processes() {
    assert!(is_userland_thread(Some(ThreadKind::Userland)));
    assert!(!is_userland_thread(Some(ThreadKind::Kernel)));
    assert!(!is_userland_thread(None));
}

#[test]
fn host_snapshot_has_cores_memory_and_system_disk() {
    if System::new().is_err() {
        return;
    }
    let mut sampler = HostSampler::new();
    sampler.sample_network();
    let snapshot = sampler.sample_system();
    assert!(!snapshot.cpu_cores.is_empty());
    assert!(snapshot.mem_total_bytes > 0);
    assert!(snapshot.disk_total_bytes > 0);
    assert!(!snapshot.os_name.is_empty());
}

#[test]
fn process_snapshot_contains_current_process() {
    if System::new().is_err() {
        return;
    }
    let mut sampler = ProcessSampler::new();
    let processes = sampler.sample();
    assert!(
        processes
            .iter()
            .any(|process| process.pid == std::process::id())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn process_snapshot_excludes_userland_threads() {
    if System::new().is_err() {
        return;
    }
    let mut sampler = ProcessSampler::new();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(0);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("transom-thread-filter-test".to_string())
            .spawn_scoped(scope, move || {
                started_tx.send(()).expect("线程启动通知应当发送成功");
                release_rx.recv().expect("线程释放信号应当接收成功");
            })
            .expect("测试线程应当创建成功");
        started_rx.recv().expect("测试线程启动信号应当接收成功");

        let processes = sampler.sample();
        assert!(
            processes
                .iter()
                .all(|process| !process.cmdline.starts_with("transom-thread")),
            "用户线程不应作为独立进程返回"
        );

        release_tx.send(()).expect("测试线程释放信号应当发送成功");
    });
}
