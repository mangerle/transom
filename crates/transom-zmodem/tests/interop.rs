//! ZMODEM 与真实 lrzsz 的端到端互操作测试。
//!
//! 为什么必须与真机对跑：库内合成用例的字节流由同一套编解码函数生成，只能证明
//! 自洽，证明不了与参考实现一致——CRC 取反层次、帧头字节序、收尾时序这类细节
//! 只要编解码两侧同时写错就会一路全绿，而真机 sz/rz 立刻判为坏帧。这里用工作区
//! 既有的 portable-pty 起 `sz`/`rz` 子进程，把伪终端主端接进 `attach_zmodem`
//! 的公开 API，链路与产品完全同构（含下载目录自动创建、多文件批次、中途取消）。
//!
//! 未安装 lrzsz 的环境整组跳过：属可选的外部工具缺失，不让 `cargo test` 失败。

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, PtyPair, PtySize, native_pty_system};
use transom_core::{
    TerminalControlHandle, TerminalIo, ZmodemDirection, ZmodemEvent, ZmodemFinished,
};
use transom_zmodem::{ZmodemConfig, ZmodemControl, attach_zmodem};

/// 单场会话的看门狗：lrzsz 在对端不应答时会长时间轮询，测试不能跟着挂死。
const WATCHDOG: Duration = Duration::from_secs(90);
/// 事件轮询间隔。
const POLL: Duration = Duration::from_millis(100);
/// 子进程收尾等待：正常退出优先，超时强杀，杜绝僵尸与孤儿进程。
const REAP_WAIT: Duration = Duration::from_secs(5);

/// lrzsz 是否可用；缺失时相关用例直接返回。
fn lrzsz_available(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 伪终端两端的持有者：主端读写已 dup 独立 fd，从端句柄需在收尾时显式关闭，
/// 否则主端读永远不会返回 EOF，抽字节的线程也就收不住。
struct Peer {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    slave: Box<dyn portable_pty::SlavePty + Send>,
}

/// 在指定目录里起一个 lrzsz 子进程，接到伪终端从端。
fn spawn_lrzsz(program: &str, args: &[&str], cwd: &Path) -> Peer {
    let pty_system = native_pty_system();
    let PtyPair { slave, master } = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("伪终端创建失败");
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(cwd.as_os_str());
    let child = slave.spawn_command(cmd).expect("lrzsz 子进程启动失败");
    // 主端句柄本身可以立刻释放：读写端各自 dup 了文件描述符。
    let reader = master.try_clone_reader().expect("主端读句柄获取失败");
    let writer = master.take_writer().expect("主端写句柄获取失败");
    drop(master);
    Peer {
        reader,
        writer,
        child,
        slave,
    }
}

/// 收割子进程：先看是否已退出，等待超时则强杀并回收状态。
fn reap(child: &mut Box<dyn Child + Send + Sync>, slave: Box<dyn portable_pty::SlavePty + Send>) {
    drop(slave);
    let deadline = Instant::now() + REAP_WAIT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                log::warn!("lrzsz 子进程状态查询失败，按强杀处理: {error}");
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// 把 lrzsz 子进程接进 ZMODEM 监督层并驱动到会话收尾，返回全部事件。
///
/// `on_event` 在每条事件上报时获得一次决策机会（提交上传文件、中途取消等），
/// 让不同场景共用同一套驱动与收尾逻辑。
fn drive(
    peer: Peer,
    download_dir: PathBuf,
    mut on_event: impl FnMut(&ZmodemEvent, &ZmodemControl),
) -> Vec<ZmodemEvent> {
    let Peer {
        reader,
        writer,
        mut child,
        slave,
    } = peer;
    let (event_tx, event_rx) = flume::bounded::<ZmodemEvent>(64);
    let (control, _control_rx) = TerminalControlHandle::channel(4);
    let io = TerminalIo::new(reader, writer, control);
    let config = ZmodemConfig {
        download_dir,
        auto_receive: true,
    };
    let (mut io, zmodem) = attach_zmodem(io, config, event_tx);

    // 终端侧抽取线程：协议字节由驱动线程直接写回主端，这里只负责持续读掉
    // "终端可见字节"，模拟真实界面消费，读过滤层的通道才不会积压。
    let pump = std::thread::spawn(move || {
        let mut sink = [0u8; 4096];
        loop {
            match io.reader.read(&mut sink) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    let mut events = Vec::new();
    let deadline = Instant::now() + WATCHDOG;
    while Instant::now() < deadline {
        match event_rx.recv_timeout(POLL) {
            Ok(event) => {
                let finished = matches!(event, ZmodemEvent::Finished(_));
                on_event(&event, &zmodem);
                events.push(event);
                if finished {
                    break;
                }
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(flume::RecvTimeoutError::Disconnected) => break,
        }
    }

    reap(&mut child, slave);
    let _ = zmodem;
    if pump.join().is_err() {
        log::warn!("终端抽取线程异常结束");
    }
    events
}

/// 断言会话以"完成 n 个文件"收尾。
fn assert_completed(events: &[ZmodemEvent], files: usize) {
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ZmodemEvent::Started { .. })),
        "应上报会话开始事件，实际事件: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            ZmodemEvent::Finished(ZmodemFinished::Completed { files: done }) if *done == files
        )),
        "应以完成 {files} 个文件收尾，实际事件: {events:?}"
    );
}

/// 构造含转义敏感字节的负载：ZDLE(0x18)、0xFF、`@` 后接 CR 都必须能穿过
/// ZDLE 转义与终端通路而不失真。
fn tricky_payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| match index % 17 {
            0 => 0x18,
            3 => 0xff,
            5 => b'@',
            6 => b'\r',
            9 => b'\n',
            _ => (index % 251) as u8,
        })
        .collect()
}

/// 真机 `sz` 下载：多文件批次 + 不存在的下载目录 + 超过单个子包的数据量。
/// 这是最能暴露帧格式分歧的一条：CRC-32 取反或帧头字节序任一有误都会失败。
#[test]
fn real_sz_download_lands_multiple_files_byte_identical() {
    if !lrzsz_available("sz") {
        eprintln!("未安装 lrzsz，跳过真机下载互操作测试");
        return;
    }
    let root = tempfile::tempdir().expect("临时目录创建");
    let payload_a = tricky_payload(9000);
    let payload_b = b"tiny zmodem file\n".to_vec();
    std::fs::write(root.path().join("a.bin"), &payload_a).expect("写入源文件 a");
    std::fs::write(root.path().join("b.txt"), &payload_b).expect("写入源文件 b");
    // 故意指向尚不存在的多级目录，顺带验证下载目录自动创建。
    let incoming = root.path().join("incoming/deep");
    let peer = spawn_lrzsz("sz", &["--zmodem", "-q", "a.bin", "b.txt"], root.path());

    let events = drive(peer, incoming.clone(), |_, _| {});

    assert_completed(&events, 2);
    assert_eq!(
        std::fs::read(incoming.join("a.bin")).expect("落盘文件 a"),
        payload_a
    );
    assert_eq!(
        std::fs::read(incoming.join("b.txt")).expect("落盘文件 b"),
        payload_b
    );
}

/// 真机 `rz` 上传：界面侧决策经命令通道送回，对端落盘内容必须一致。
#[test]
fn real_rz_upload_delivers_file_to_peer() {
    if !lrzsz_available("rz") {
        eprintln!("未安装 lrzsz，跳过真机上传互操作测试");
        return;
    }
    let root = tempfile::tempdir().expect("临时目录创建");
    let payload = tricky_payload(5000);
    let local = root.path().join("upload.bin");
    std::fs::write(&local, &payload).expect("写入待上传文件");
    let peer_dir = root.path().join("peer");
    std::fs::create_dir_all(&peer_dir).expect("对端目录创建");

    let peer = spawn_lrzsz("rz", &["--zmodem", "-q"], &peer_dir);
    let mut handed = false;
    let events = drive(peer, root.path().join("incoming"), |event, control| {
        if matches!(event, ZmodemEvent::AwaitUpload) && !handed {
            handed = true;
            assert!(
                control.upload(vec![local.clone()]),
                "等待决策窗口内提交上传应当被受理"
            );
        }
    });

    assert!(handed, "应收到对端 rz 的上传决策请求");
    assert_completed(&events, 1);
    assert_eq!(
        std::fs::read(peer_dir.join("upload.bin")).expect("对端落盘文件"),
        payload
    );
}

/// 真机中途取消：发出取消命令后必须以 Cancelled 收尾，且 `sz` 自身退出，
/// 不给对端留下挂死的传输进程。
#[test]
fn real_sz_download_can_be_cancelled_midway() {
    if !lrzsz_available("sz") {
        eprintln!("未安装 lrzsz，跳过真机取消互操作测试");
        return;
    }
    let root = tempfile::tempdir().expect("临时目录创建");
    std::fs::write(root.path().join("big.bin"), tricky_payload(60_000)).expect("写入源文件");
    let incoming = root.path().join("incoming");
    let peer = spawn_lrzsz("sz", &["--zmodem", "-q", "big.bin"], root.path());

    let mut cancelled = false;
    let events = drive(peer, incoming.clone(), |event, control| {
        if !cancelled
            && matches!(
                event,
                ZmodemEvent::Progress {
                    direction: ZmodemDirection::Download,
                    transferred,
                    ..
                } if *transferred > 2048
            )
        {
            cancelled = true;
            assert!(control.cancel(), "传输中取消命令应被受理");
        }
    });

    assert!(cancelled, "应在数据流动后触发取消");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ZmodemEvent::Finished(ZmodemFinished::Cancelled))),
        "应以取消收尾，实际事件: {events:?}"
    );
}
