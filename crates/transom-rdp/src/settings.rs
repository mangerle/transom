//! RDP 连接配置构建与参数转换。
//!
//! 设计原理（Why）：将 `transom-core` 领域模型中的 `RdpConnection` 规范化为
//! IronRDP 协议连接序列所需的强类型凭据与客户端能力标志，并在类型边界进行
//! 默认值兜底与合法性归一化。

use ironrdp_connector::{BitmapConfig, Credentials, DesktopSize};
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::rdp::capability_sets::{
    MajorPlatformType, RailSupportLevel, client_codecs_capabilities,
};
use ironrdp_pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};

/// RDP 客户端与会话运行时统一使用的协议压缩算法。
pub(crate) const DEFAULT_COMPRESSION_TYPE: CompressionType = CompressionType::K64;
use std::fmt;
use std::sync::Arc;
use transom_core::{RdpColorDepth, RdpConnection, RdpPerformance, RdpSecurity};
use zeroize::Zeroize;

/// 远端 TLS 服务器证书指纹信息（用于 TOFU 信任校验与连接审计）。
#[derive(Debug, Clone)]
pub struct TlsCertificateInfo {
    /// 目标主机名或 IP。
    pub host: String,
    /// 公钥 SHA-256 十六进制指纹（冒号分隔）。
    pub sha256_fingerprint: String,
}

/// TLS 证书校验结果。
#[derive(Debug, Clone)]
pub enum TlsVerifyOutcome {
    /// 校验通过。
    Trusted,
    /// 指纹与已知主机记录不匹配，需由用户确认是否信任并更新。
    Mismatch { previous_fingerprint: String },
    /// 其他校验失败原因。
    Rejected(String),
}

/// TLS 证书校验回调类型；非 [`TlsVerifyOutcome::Trusted`] 时在发送任何 NLA 凭据前中止连接。
pub type TlsCertVerifier = Arc<dyn Fn(&TlsCertificateInfo) -> TlsVerifyOutcome + Send + Sync>;

/// RDP 客户端会话启动运行期配置。
///
/// 设计原理（Why）：手动实现脱敏 `Debug` 并在 `Drop` 时自动擦除内存中的密码，
/// 即使发生提前返回或 panic 也能确保明文凭据不会泄漏到日志或残留在堆内存中。
#[derive(Clone)]
pub struct SessionConfig {
    /// 目标主机地址（IP 或域名）。
    pub host: String,
    /// 目标端口。
    pub port: u16,
    /// 用户名。
    pub username: String,
    /// 认证密码。
    pub password: String,
    /// Windows 认证域名。
    pub domain: Option<String>,
    /// 安全协商档位。
    pub security: RdpSecurity,
    /// 协商色深。
    pub color_depth: RdpColorDepth,
    /// 体验与性能参数。
    pub performance: RdpPerformance,
    /// 是否开启声音重定向。
    pub audio: bool,
    /// 是否开启双向文本剪贴板。
    pub clipboard: bool,
    /// 是否启用视口动态自适应重设分辨率。
    pub dynamic_resolution: bool,
    /// 初始视口尺寸（宽、高）。
    pub initial_size: (u16, u16),
    /// 客户端 DPI 缩放百分比（100..=500，例如 200 表示 2x HiDPI）。
    pub scale_factor_percent: u32,
    /// 可选的底层 TCP 实际拨号端点（用于通过 SSH 隧道转发至本地 `127.0.0.1:port` 时，
    /// 保持 `host`/`port` 为真实目标主机以进行 TLS SNI、CredSSP 与 known_hosts 校验）。
    pub connect_endpoint: Option<(String, u16)>,
    /// 可选的本地共享目录路径（启用 RDPDR 磁盘重定向）。
    pub shared_dir: Option<String>,
    /// 可选的 TLS 证书校验回调（用于 TOFU 或指纹钉扎）。
    pub cert_verifier: Option<TlsCertVerifier>,
}

impl SessionConfig {
    /// 基于领域连接模型与口令构造运行期配置。
    pub fn new(connection: &RdpConnection, password: String, initial_size: (u16, u16)) -> Self {
        let (username, domain) =
            parse_user_and_domain(&connection.username, connection.domain.as_deref());

        Self {
            host: connection.host.clone(),
            port: connection.port,
            username,
            password,
            domain,
            security: connection.security,
            color_depth: connection.color_depth,
            performance: connection.performance,
            audio: connection.audio,
            clipboard: connection.clipboard,
            dynamic_resolution: connection.dynamic_resolution,
            initial_size: connection
                .desktop_size
                .map(|size| (size.width, size.height))
                .unwrap_or(initial_size),
            scale_factor_percent: 100,
            connect_endpoint: None,
            shared_dir: connection.shared_dir.clone(),
            cert_verifier: None,
        }
    }

    /// 安全擦除内存中的密码字符串。
    pub fn clear_password(&mut self) {
        self.password.zeroize();
    }

    /// 构造 IronRDP 连接器配置。
    pub fn to_ironrdp_config(&self) -> ironrdp_connector::Config {
        let codecs = client_codecs_capabilities(&["remotefx"]).unwrap_or_default();
        let bitmap = BitmapConfig {
            lossy_compression: true,
            color_depth: match self.color_depth {
                RdpColorDepth::Depth32 => 32,
                RdpColorDepth::Depth24 => 24,
                RdpColorDepth::Depth16 => 16,
            },
            codecs,
        };

        ironrdp_connector::Config {
            credentials: Credentials::UsernamePassword {
                username: self.username.clone(),
                password: self.password.clone(),
            },
            domain: self.domain.clone(),
            enable_tls: match self.security {
                RdpSecurity::Negotiate | RdpSecurity::Nla | RdpSecurity::Tls => true,
                RdpSecurity::Rdp => false,
            },
            enable_credssp: match self.security {
                RdpSecurity::Negotiate | RdpSecurity::Nla => true,
                RdpSecurity::Tls | RdpSecurity::Rdp => false,
            },
            enable_standard_rdp_security: matches!(self.security, RdpSecurity::Rdp),
            keyboard_type: KeyboardType::IBM_ENHANCED,
            keyboard_subtype: 0,
            keyboard_layout: 0,
            keyboard_functional_keys_count: 12,
            ime_file_name: String::new(),
            dig_product_id: "Transom RDP".to_string(),
            desktop_size: DesktopSize {
                width: self.initial_size.0,
                height: self.initial_size.1,
            },
            desktop_scale_factor: self.scale_factor_percent.clamp(100, 500),
            monitor_layout: None,
            desktop_physical_size: None,
            desktop_orientation: None,
            device_scale_factor: None,
            monitor_extended_layout: None,
            relative_mouse_input: false,
            connection_type: ironrdp_pdu::gcc::ConnectionType::NotUsed,
            bitmap: Some(bitmap),
            client_build: 0,
            client_name: "Transom-RDP".to_string(),
            client_dir: String::new(),
            platform: platform(),
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: self.audio,
            enable_audio_capture: false,
            performance_flags: self.performance_flags(),
            license_cache: None,
            timezone_info: TimezoneInfo::default(),
            compression_type: Some(DEFAULT_COMPRESSION_TYPE),
            // GPUI 当前未提供隐藏宿主原生光标（CursorStyle::None）或注册自定义位图光标的能力；
            // 若开启服务端指针位图，客户端自绘光标会与系统箭头光标双重叠加。此处关闭服务端指针，
            // 统一使用零延迟本地原生光标；待未来 GUI 框架支持隐藏原生光标后可重新开启。
            enable_server_pointer: false,
            pointer_software_rendering: false,
            multitransport_flags: None,
            alternate_shell: String::new(),
            work_dir: String::new(),
            remote_application_mode: false,
            rail_support_level: RailSupportLevel::empty(),
            support_dyn_vc_gfx_protocol: false,
        }
    }

    /// 转换为 RDP 客户端性能标志位（MS-RDPBCGR 性能优化）。
    pub fn performance_flags(&self) -> PerformanceFlags {
        match self.performance {
            RdpPerformance::Lan => PerformanceFlags::empty(),
            RdpPerformance::Broadband => {
                PerformanceFlags::DISABLE_WALLPAPER
                    | PerformanceFlags::DISABLE_FULLWINDOWDRAG
                    | PerformanceFlags::DISABLE_MENUANIMATIONS
            }
            RdpPerformance::Wan => {
                PerformanceFlags::DISABLE_WALLPAPER
                    | PerformanceFlags::DISABLE_FULLWINDOWDRAG
                    | PerformanceFlags::DISABLE_MENUANIMATIONS
                    | PerformanceFlags::DISABLE_THEMING
                    | PerformanceFlags::DISABLE_CURSOR_SHADOW
                    | PerformanceFlags::DISABLE_CURSORSETTINGS
            }
        }
    }
}

impl fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("security", &self.security)
            .field("color_depth", &self.color_depth)
            .field("performance", &self.performance)
            .field("audio", &self.audio)
            .field("clipboard", &self.clipboard)
            .field("dynamic_resolution", &self.dynamic_resolution)
            .field("initial_size", &self.initial_size)
            .field(
                "cert_verifier",
                &self.cert_verifier.as_ref().map(|_| "[CALLBACK]"),
            )
            .finish()
    }
}

impl Drop for SessionConfig {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

fn platform() -> MajorPlatformType {
    if cfg!(windows) {
        MajorPlatformType::WINDOWS
    } else if cfg!(target_os = "macos") {
        MajorPlatformType::MACINTOSH
    } else {
        MajorPlatformType::UNIX
    }
}

/// 解析可能包含在用户名中的 Windows 域（如 `DOMAIN\user` 或 `user@domain`）。
fn parse_user_and_domain(
    raw_username: &str,
    explicit_domain: Option<&str>,
) -> (String, Option<String>) {
    if let Some(domain) = explicit_domain
        && !domain.trim().is_empty()
    {
        return (raw_username.to_string(), Some(domain.trim().to_string()));
    }

    if let Some((domain, user)) = raw_username.split_once('\\') {
        (user.to_string(), Some(domain.to_string()))
    } else if let Some((user, domain)) = raw_username.split_once('@') {
        (user.to_string(), Some(domain.to_string()))
    } else {
        (raw_username.to_string(), None)
    }
}
