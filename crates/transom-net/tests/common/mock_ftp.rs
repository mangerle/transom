//! 进程内最小 FTP 服务器：为 `ftp_roundtrip` 集成测试提供真实协议对话。
//!
//! 设计原理（Why）：SFTP 回环测试用 russh 进程内服务端，FTP 侧遵循同一
//! 模式——不依赖 Docker 或外部 pyftpdlib，测试在 CI 里零外部依赖可复现。
//! 只实现 FtpClient 用到的命令子集（登录、PASV 数据连接、MLSD/MLST、
//! SIZE/MDTM、REST 断点续传、STOR/RETR、MKD/RNFR+RNTO/DELE/RMD、QUIT），
//! 服务端行为对齐 pyftpdlib 的语义：REST 只作用于下一条传输命令并在传输
//! 后消费，STOR 从偏移处覆写，非空目录 RMD 拒绝。
//!
//! 协议时序约束：`150` 应答必须先于数据连接上的读写写出（suppaftp 在收到
//! 150 后才连接数据端口），因此数据命令先做存在性校验、返回 150 文本与
//! 传输种类，控制循环写出 150 后再接受数据连接并内联执行传输、回 `226`。
//! mock 对每条控制连接顺序处理，与真实客户端一次一条控制连接的行为一致。
//!
//! 代价与局限：不实现主动模式与 TLS——被测客户端走被动模式，FTPS 由平台
//! 证书体系覆盖，不适合 mock。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// 测试凭据：`transom` / `transom`。
pub const MOCK_FTP_USER: &str = "transom";
pub const MOCK_FTP_PASSWORD: &str = "transom";

/// 单条控制连接的会话状态。
struct MockSession {
    root: PathBuf,
    data_listener: Option<TcpListener>,
    rest_offset: u64,
    authenticated: bool,
    pending_rename: Option<PathBuf>,
}

/// 数据连接上要执行的传输种类。
enum TransferKind {
    /// RETR：从偏移处把文件内容写入数据连接。
    SendFile { path: PathBuf, offset: u64 },
    /// STOR：从数据连接读取内容，在偏移处覆写文件。
    ReceiveFile { path: PathBuf, offset: u64 },
    /// MLSD/LIST：把目录列表行写入数据连接。
    SendLines { lines: String },
}

/// 一条命令的处理结果：普通应答，或"先回 150、再执行传输并回 226"。
enum Outcome {
    Reply(String),
    Transfer {
        reply: &'static str,
        kind: TransferKind,
    },
}

/// 启动 mock FTP 服务器，返回监听端口与接收任务句柄。
pub async fn spawn(root: &Path) -> (u16, tokio::task::JoinHandle<()>) {
    spawn_with_command_limit(root, u64::MAX).await
}

/// 启动服务器，并在每条连接处理 `limit` 条命令后静默断开控制连接。
/// 供测试模拟"控制连接被服务端掐断"的断线自愈场景。
pub async fn spawn_with_command_limit(
    root: &Path,
    limit: u64,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = Arc::new(root.to_path_buf());
    let handle = tokio::spawn(async move {
        loop {
            let (socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let root = Arc::clone(&root);
            tokio::spawn(async move {
                serve_connection(root, socket, limit).await;
            });
        }
    });
    (port, handle)
}

/// 服务单个控制连接：逐行处理命令直到 QUIT 或流结束。
async fn serve_connection(root: Arc<PathBuf>, socket: TcpStream, command_limit: u64) {
    let mut commands_served = 0u64;
    let (reader, mut writer) = socket.into_split();
    let mut lines = BufReader::new(reader).lines();
    let session = Arc::new(Mutex::new(MockSession {
        root: root.as_ref().clone(),
        data_listener: None,
        rest_offset: 0,
        authenticated: false,
        pending_rename: None,
    }));

    let _ = writer.write_all(b"220 Mock FTP server ready\r\n").await;
    while let Ok(Some(line)) = lines.next_line().await {
        let (command, argument) = line
            .split_once(' ')
            .map_or((line.as_str(), ""), |(c, a)| (c, a));
        let command = command.to_ascii_uppercase();
        // 整轮持锁：单客户端顺序命令，锁内含数据传输与真实 FTP 语义一致。
        let mut session = session.lock().await;
        let outcome = match command.as_str() {
            "USER" if argument == MOCK_FTP_USER => Outcome::Reply("331 Password required".into()),
            "USER" => Outcome::Reply("530 Login incorrect".into()),
            "PASS" => {
                if argument == MOCK_FTP_PASSWORD {
                    session.authenticated = true;
                    Outcome::Reply("230 Logged in".into())
                } else {
                    Outcome::Reply("530 Login incorrect".into())
                }
            }
            "TYPE" => Outcome::Reply("200 Type set".into()),
            "PWD" => Outcome::Reply("257 \"/\" is the current directory".into()),
            "CWD" => {
                let path = resolve(&session.root, argument);
                if path.is_dir() {
                    Outcome::Reply("250 Directory changed".into())
                } else {
                    Outcome::Reply("550 Directory unavailable".into())
                }
            }
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = listener.local_addr().unwrap().port();
                session.data_listener = Some(listener);
                let (p1, p2) = (port / 256, port % 256);
                Outcome::Reply(format!("227 Entering Passive Mode (127,0,0,1,{p1},{p2})"))
            }
            "QUIT" => {
                let _ = writer.write_all(b"221 Bye\r\n").await;
                return;
            }
            _ => handle_command(&mut session, command.as_str(), argument).await,
        };
        commands_served += 1;
        if commands_served >= command_limit {
            // 模拟服务端掐断：不回应答直接断开，客户端下一次操作感知连接级故障。
            return;
        }
        match outcome {
            Outcome::Reply(text) => {
                if writer
                    .write_all(format!("{text}\r\n").as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Outcome::Transfer { reply, kind } => {
                if writer
                    .write_all(format!("{reply}\r\n").as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                // 150 已写出，客户端此时才连数据端口；5 秒超时防挂死。
                let data = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    accept_data(&mut session),
                )
                .await
                .ok()
                .and_then(|result| result.ok());
                if let Some(mut data) = data {
                    run_transfer(&mut data, kind).await;
                }
                session.rest_offset = 0;
                let _ = writer.write_all(b"226 Transfer complete\r\n").await;
            }
        }
    }
}

/// 接受一条 PASV 数据连接；监听器一次性消费。
async fn accept_data(session: &mut MockSession) -> Result<TcpStream, ()> {
    let listener = session.data_listener.take().ok_or(())?;
    listener
        .accept()
        .await
        .map(|(stream, _)| stream)
        .map_err(|_| ())
}

/// 在数据连接上执行传输；完成后连接随流关闭，控制循环负责回 226。
async fn run_transfer(data: &mut TcpStream, kind: TransferKind) {
    match kind {
        TransferKind::SendFile { path, offset } => {
            let mut file = tokio::fs::File::open(&path).await.unwrap();
            if offset > 0 {
                file.seek(std::io::SeekFrom::Start(offset)).await.unwrap();
            }
            let mut buffer = vec![0u8; 8192];
            loop {
                let count = file.read(&mut buffer).await.unwrap_or(0);
                if count == 0 {
                    break;
                }
                if data.write_all(&buffer[..count]).await.is_err() {
                    return;
                }
            }
        }
        TransferKind::ReceiveFile { path, offset } => {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.unwrap();
            }
            let mut file = if offset > 0 {
                // 续传覆写：显式不截断，偏移之前的前缀必须保留。
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(&path)
                    .await
                    .unwrap()
            } else {
                tokio::fs::File::create(&path).await.unwrap()
            };
            if offset > 0 {
                file.seek(std::io::SeekFrom::Start(offset)).await.unwrap();
            }
            let mut buffer = vec![0u8; 8192];
            loop {
                let count = data.read(&mut buffer).await.unwrap_or(0);
                if count == 0 {
                    break;
                }
                if file.write_all(&buffer[..count]).await.is_err() {
                    return;
                }
            }
        }
        TransferKind::SendLines { lines } => {
            let _ = data.write_all(lines.as_bytes()).await;
        }
    }
}

/// 处理需要会话状态的命令；数据命令只做校验与参数准备，传输由控制循环执行。
async fn handle_command(session: &mut MockSession, command: &str, argument: &str) -> Outcome {
    match command {
        "MLSD" => {
            let directory = resolve(&session.root, argument);
            match tokio::fs::metadata(&directory).await {
                Ok(metadata) if metadata.is_dir() => Outcome::Transfer {
                    reply: "150 Opening data connection",
                    kind: TransferKind::SendLines {
                        lines: mlsd_lines(&directory).await,
                    },
                },
                _ => Outcome::Reply("550 Directory unavailable".into()),
            }
        }
        "RETR" => {
            let path = resolve(&session.root, argument);
            match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() => Outcome::Transfer {
                    reply: "150 Sending file",
                    kind: TransferKind::SendFile {
                        path,
                        offset: session.rest_offset,
                    },
                },
                _ => Outcome::Reply("550 File unavailable".into()),
            }
        }
        "STOR" => Outcome::Transfer {
            reply: "150 Receiving file",
            kind: TransferKind::ReceiveFile {
                path: resolve(&session.root, argument),
                offset: session.rest_offset,
            },
        },
        "SIZE" => {
            let path = resolve(&session.root, argument);
            match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() => {
                    Outcome::Reply(format!("213 {}", metadata.len()))
                }
                _ => Outcome::Reply("550 File unavailable".into()),
            }
        }
        "MDTM" => {
            let path = resolve(&session.root, argument);
            match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() => {
                    let seconds = metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|elapsed| elapsed.as_secs())
                        .unwrap_or(0);
                    Outcome::Reply(format!("213 {}", seconds_to_mdtm(seconds)))
                }
                _ => Outcome::Reply("550 File unavailable".into()),
            }
        }
        "MKD" => {
            let path = resolve(&session.root, argument);
            match tokio::fs::create_dir(&path).await {
                Ok(()) => Outcome::Reply(format!("257 \"{argument}\" created")),
                Err(_) => Outcome::Reply("550 Directory already exists or cannot create".into()),
            }
        }
        "RNFR" => {
            let path = resolve(&session.root, argument);
            if path.exists() {
                session.pending_rename = Some(path);
                Outcome::Reply("350 Ready for destination name".into())
            } else {
                Outcome::Reply("550 File unavailable".into())
            }
        }
        "RNTO" => match session.pending_rename.take() {
            Some(from) => {
                let to = resolve(&session.root, argument);
                match tokio::fs::rename(&from, &to).await {
                    Ok(()) => Outcome::Reply("250 Rename successful".into()),
                    Err(_) => Outcome::Reply("550 Rename failed".into()),
                }
            }
            None => Outcome::Reply("503 Bad sequence (RNFR first)".into()),
        },
        "DELE" => {
            let path = resolve(&session.root, argument);
            match tokio::fs::remove_file(&path).await {
                Ok(()) => Outcome::Reply("250 File deleted".into()),
                Err(_) => Outcome::Reply("550 Delete failed".into()),
            }
        }
        "RMD" => {
            let path = resolve(&session.root, argument);
            match tokio::fs::remove_dir(&path).await {
                Ok(()) => Outcome::Reply("250 Directory removed".into()),
                Err(_) => Outcome::Reply("550 Remove failed (not empty?)".into()),
            }
        }
        "REST" => match argument.parse::<u64>() {
            Ok(offset) => {
                session.rest_offset = offset;
                Outcome::Reply("350 Restarting at offset".into())
            }
            Err(_) => Outcome::Reply("501 Invalid offset".into()),
        },
        _ => Outcome::Reply("502 Command not implemented".into()),
    }
}

/// 把会话根目录下的相对远端路径解析为宿主路径。
fn resolve(root: &Path, remote: &str) -> PathBuf {
    root.join(remote.trim_start_matches('/'))
}

/// 生成目录的 MLSD 行（facts + 空格 + 名称，RFC 3659 形态）。
async fn mlsd_lines(directory: &Path) -> String {
    let mut listing = String::new();
    let Ok(mut children) = tokio::fs::read_dir(directory).await else {
        return String::new();
    };
    while let Ok(Some(child)) = children.next_entry().await {
        let Ok(metadata) = child.metadata().await else {
            continue;
        };
        let name = child.file_name().to_string_lossy().into_owned();
        let (fact_type, size) = if metadata.is_dir() {
            ("dir", 0)
        } else {
            ("file", metadata.len())
        };
        let modify = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs())
            .map(seconds_to_mdtm)
            .unwrap_or_else(|| "19700101000000".to_string());
        listing.push_str(&format!(
            "type={fact_type};size={size};modify={modify};UNIX.mode=0644; {name}\r\n"
        ));
    }
    listing
}

/// Unix 秒转 MDTM 时间戳（UTC，YYYYMMDDHHMMSS）。
///
/// 复用 `transom_net::ftp::listing` 的换算逆运算不便（该函数只做正向），
/// 这里直接由 Timestamp 换算的本地实现：测试内按日历日累加即可，不追求
/// 通用性。
fn seconds_to_mdtm(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (hour, minute, second) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    // civil-from-days（Howard Hinnant 逆算法）。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let day_of_year = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        year, month, day, hour, minute, second
    )
}
