//! RDPSND / DVC 本地音频输出管道。
//!
//! 设计原理（Why）：RDP 远端音频重定向通过 RDPSND 或动态虚拟通道（DVC）传输
//! 压缩或原始 PCM 音频块。通过将收到的波形数据经 cpal 跨平台音频后端送往
//! 本地默认声卡，实现低延迟声音直通，且全程不经 UI 主线程。

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 固定容量环形音频采样缓冲。
///
/// 设计原理（Why）：替代从 `Vec<f32>` 头部 `drain` 的大块内存拷贝操作，
/// 读写两端通过模运算移动游标，操作复杂度为常数时间 O(1)，彻底杜绝音频线程卡顿。
struct AudioRingBuffer {
    data: Vec<f32>,
    read_pos: usize,
    write_pos: usize,
    size: usize,
    capacity: usize,
}

impl AudioRingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            read_pos: 0,
            write_pos: 0,
            size: 0,
            capacity,
        }
    }

    fn push_i16_samples(&mut self, samples: &[i16]) {
        for &sample in samples {
            if self.size == self.capacity {
                // 缓冲区满时丢弃最旧采样，避免积压形成可感知延迟
                self.read_pos = (self.read_pos + 1) % self.capacity;
                self.size -= 1;
            }
            self.data[self.write_pos] = sample as f32 / 32768.0;
            self.write_pos = (self.write_pos + 1) % self.capacity;
            self.size += 1;
        }
    }

    fn read_into(&mut self, output: &mut [f32]) -> usize {
        let to_read = self.size.min(output.len());
        for item in output.iter_mut().take(to_read) {
            *item = self.data[self.read_pos];
            self.read_pos = (self.read_pos + 1) % self.capacity;
        }
        self.size -= to_read;
        to_read
    }
}

/// 音频播放控制器。
pub struct AudioPlayer {
    muted: Arc<AtomicBool>,
    buffer: Arc<Mutex<AudioRingBuffer>>,
    _stream: Option<cpal::Stream>,
}

impl AudioPlayer {
    /// 构造本地音频播放器并启动输出流（默认缓冲 48000 采样，约 1 秒）。
    pub fn new() -> Self {
        let muted = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(AudioRingBuffer::new(48000)));

        let stream = match init_cpal_stream(buffer.clone(), muted.clone()) {
            Ok(s) => {
                if let Err(e) = s.play() {
                    log::warn!("启动音频播放流失败，原因: {e}");
                    None
                } else {
                    Some(s)
                }
            }
            Err(e) => {
                log::warn!("初始化本地音频设备失败，声音将被静音，原因: {e}");
                None
            }
        };

        Self {
            muted,
            buffer,
            _stream: stream,
        }
    }

    /// 设置静音状态。
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// 查询当前是否静音。
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    /// 写入一段来自协议解码的 16-bit PCM 采样数据（双声道/单声道交错）。
    pub fn write_samples_i16(&self, samples: &[i16]) {
        if self.is_muted() || samples.is_empty() {
            return;
        }

        self.buffer.lock().push_i16_samples(samples);
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

fn init_cpal_stream(
    buffer: Arc<Mutex<AudioRingBuffer>>,
    muted: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "未找到默认音频输出设备".to_string())?;

    let config = device
        .default_output_config()
        .map_err(|e| format!("获取默认音频格式失败: {e}"))?;

    let sample_format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let err_fn = |err| log::warn!("音频输出流出现异常: {err}");

    match sample_format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    fill_audio_data(data, &buffer, &muted);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("构建 f32 音频流失败: {e}")),
        _ => Err("当前声卡不支持直接输出 f32 采样".to_string()),
    }
}

fn fill_audio_data(
    output: &mut [f32],
    buffer: &Arc<Mutex<AudioRingBuffer>>,
    muted: &Arc<AtomicBool>,
) {
    if muted.load(Ordering::Relaxed) {
        output.fill(0.0);
        return;
    }

    let mut buf = buffer.lock();
    let read_count = buf.read_into(output);

    if read_count < output.len() {
        output[read_count..].fill(0.0);
    }
}
