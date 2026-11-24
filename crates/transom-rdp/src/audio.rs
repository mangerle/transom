//! RDPSND 本地音频输出管道与静音控制器。
//!
//! 设计原理（Why）：通过装饰器模式（Decorator Pattern）包裹底层原生的
//! `ironrdp_rdpsnd_native::cpal::RdpsndBackend`，持有一个由原子布尔值（`AtomicBool`）
//! 控制的静音开关。当处于静音状态时安全拦截采样回放，彻底杜绝自建额外 CPAL 输出流造成的
//! 声卡多重占用与硬件资源竞争，实现低延迟声音直通与 UI 静音按键的即时联动。

use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu};
use ironrdp_rdpsnd_native::cpal::RdpsndBackend;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 受控的 RDPSND 音频直通后端。
#[derive(Debug)]
pub struct TransomAudioBackend {
    inner: RdpsndBackend,
    muted: Arc<AtomicBool>,
}

impl TransomAudioBackend {
    /// 构造新的受控音频后端，绑定指定的原子静音标志。
    pub fn new(muted: Arc<AtomicBool>) -> Self {
        Self {
            inner: RdpsndBackend::new(),
            muted,
        }
    }
}

impl RdpsndClientHandler for TransomAudioBackend {
    fn get_flags(&self) -> AudioFormatFlags {
        self.inner.get_flags()
    }

    fn get_formats(&self) -> &[AudioFormat] {
        self.inner.get_formats()
    }

    fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        self.inner.choose_formats(server_formats)
    }

    fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>) {
        if self.muted.load(Ordering::Relaxed) {
            // 静音状态下丢弃波形数据，不占用声卡硬件通道
            return;
        }
        self.inner.wave(format, ts, data);
    }

    fn playback_latency(&self) -> Duration {
        if self.muted.load(Ordering::Relaxed) {
            Duration::ZERO
        } else {
            self.inner.playback_latency()
        }
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        self.inner.set_volume(volume);
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        self.inner.set_pitch(pitch);
    }

    fn close(&mut self) {
        self.inner.close();
    }
}
