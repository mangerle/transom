//! 提权代理进程内部：创建 PTY、泵送字节并回收整棵 Shell 进程树。

use super::{BrokerConfig, BrokerReady, PipeNames};
use crate::local_pty::{self, LocalPtySession};
use crate::pty_broker::{read_frame, write_frame};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::time::Duration;
use transom_core::LocalConnection;
use transom_platform::named_pipe::connect_private_client;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::core::PCWSTR;

pub(crate) fn run(token: &str) -> anyhow::Result<()> {
    uuid::Uuid::parse_str(token).map_err(|error| anyhow::anyhow!("非法代理令牌: {error}"))?;
    let names = PipeNames::new(token);
    let mut config_channel = connect_private_client(&names.config)?;
    let config: BrokerConfig = read_frame(&mut config_channel)?;
    let local = LocalConnection {
        shell: config.shell,
        shell_args: config.shell_args,
        run_as_admin: false,
        working_dir: config.working_dir,
        command: config.command,
    };
    let LocalPtySession {
        master,
        mut child,
        shell,
        exec_argv,
        preload,
        integration_disabled,
    } = local_pty::spawn(&local, config.size, config.shell_integration)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let _job = match ElevatedJob::assign(child.as_raw_handle()) {
        Ok(job) => Some(job),
        Err(error) => {
            log::warn!("提权 Shell 未能加入 Job Object，关闭时将依赖控制通道回收进程树: {error}");
            None
        }
    };
    write_frame(
        &mut config_channel,
        &BrokerReady {
            integration: preload.kind,
            integration_disabled,
        },
    )?;

    let input = connect_private_client(&names.input)?;
    let output = connect_private_client(&names.output)?;
    let control = connect_private_client(&names.control)?;
    let input_handle = input.as_raw_handle() as isize;
    let output_handle = output.as_raw_handle() as isize;
    let control_handle = control.as_raw_handle() as isize;

    log::info!(
        "提权本地 PTY 代理已就绪，Shell: {shell}，尺寸: {}x{}，工作目录: {:?}，执行命令: {exec_argv:?}",
        config.size.cols,
        config.size.rows,
        local.working_dir
    );

    let input_thread = spawn_input_pump(input, master.take_writer()?, child.clone_killer())?;
    let output_thread =
        spawn_output_pump(master.try_clone_reader()?, output, child.clone_killer())?;
    let control_thread = spawn_control_reader(master, control, child.clone_killer())?;

    let status = child.wait()?;
    log::info!("提权本地 Shell 已退出，状态: {status:?}");
    drop(config_channel);
    cancel_pipe(input_handle);
    cancel_pipe(control_handle);
    let _ = input_thread.join();
    let _ = control_thread.join();
    // Shell 已退出时输出泵通常马上收到 PTY EOF；给最终几帧一个短暂排空窗口，
    // 只有父窗口已经关闭或管道卡死时才取消，避免截断退出前的最后输出。
    for _ in 0..100 {
        if output_thread.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !output_thread.is_finished() {
        cancel_pipe(output_handle);
    }
    let _ = output_thread.join();
    drop(preload);
    Ok(())
}

fn spawn_input_pump(
    mut input: File,
    mut writer: Box<dyn Write + Send>,
    mut killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
) -> io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("transom-elevated-pty-input".to_string())
        .spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if writer.write_all(&buffer[..count]).is_err() {
                            break;
                        }
                        let _ = writer.flush();
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = killer.kill();
        })
}

fn spawn_output_pump(
    mut reader: Box<dyn Read + Send>,
    mut output: File,
    mut killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
) -> io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("transom-elevated-pty-output".to_string())
        .spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if output.write_all(&buffer[..count]).is_err() {
                            break;
                        }
                        let _ = output.flush();
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = killer.kill();
        })
}

fn spawn_control_reader(
    master: Box<dyn portable_pty::MasterPty + Send>,
    mut control: File,
    mut killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
) -> io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("transom-elevated-pty-control".to_string())
        .spawn(move || {
            loop {
                let mut kind = [0u8; 1];
                if control.read_exact(&mut kind).is_err() {
                    break;
                }
                match kind[0] {
                    1 => {
                        let mut size = [0u8; 4];
                        if control.read_exact(&mut size).is_err() {
                            break;
                        }
                        let cols = u16::from_le_bytes([size[0], size[1]]);
                        let rows = u16::from_le_bytes([size[2], size[3]]);
                        let _ = master.resize(portable_pty::PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    2 => break,
                    _ => break,
                }
            }
            let _ = killer.kill();
        })
}

fn cancel_pipe(raw_handle: isize) {
    // 安全性：只取消当前进程拥有的命名管道文件对象上的 I/O。
    unsafe {
        let _ = CancelIoEx(HANDLE(raw_handle as *mut _), None);
    }
}

struct ElevatedJob(HANDLE);

impl ElevatedJob {
    fn assign(process: Option<std::os::windows::io::RawHandle>) -> io::Result<Self> {
        let process = process.ok_or_else(|| io::Error::other("提权 Shell 缺少进程句柄"))?;
        // 安全性：创建当前代理私有的 Job Object，并把刚启动的 Shell 句柄加入。
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null()).map_err(io::Error::other)?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(io::Error::other)?;
            AssignProcessToJobObject(job, HANDLE(process as *mut _)).map_err(io::Error::other)?;
            Ok(Self(job))
        }
    }
}

impl Drop for ElevatedJob {
    fn drop(&mut self) {
        // 安全性：句柄由本结构体独占，关闭即按 KILL_ON_JOB_CLOSE 回收整棵进程树。
        unsafe {
            let _ = TerminateJobObject(self.0, 1);
            let _ = CloseHandle(self.0);
        }
    }
}
