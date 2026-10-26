//! 会话外观：专属图标与环境色标记。
//!
//! 两者都是"给会话贴一个可辨识的标签"，让同一批会话在标签栏与侧栏里一眼分得
//! 出生产、测试、数据库还是容器。都用枚举而不是自由字符串：图标集与环境色板
//! 都是产品定义好的有限集合，落盘成枚举才能在改名或删项时保持可解析，也不会
//! 出现"存了一个画不出来的图标名"这种脏数据。

use serde::{Deserialize, Serialize};

/// 会话专属图标；`Auto` 表示按连接类型自动取图标（服务器、终端、网口…）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionIcon {
    /// 跟随连接类型，不额外指定。
    #[default]
    Auto,
    /// 服务器。
    Server,
    /// 数据库。
    Database,
    /// 容器。
    Container,
    /// 云主机。
    Cloud,
    /// 终端。
    Terminal,
    /// 网络设备。
    Globe,
    /// 安全设备（堡垒机、网关）。
    Shield,
    /// 存储与中间件。
    Storage,
}

/// 会话环境色；`None` 表示不染色。
///
/// 语义是"环境标记"而不是"任意配色"：红橙黄绿蓝紫灰七档覆盖生产、预发、
/// 测试、开发等常见环境，一眼能读出含义，也不会因为自由取色导致标签栏变花。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionColor {
    /// 不染色。
    #[default]
    None,
    /// 红：生产。
    Red,
    /// 橙：预发。
    Orange,
    /// 黄：测试。
    Yellow,
    /// 绿：开发。
    Green,
    /// 蓝：基础设施。
    Blue,
    /// 紫：数据与中间件。
    Purple,
    /// 灰：离线与归档。
    Gray,
}

#[cfg(test)]
mod tests {
    use super::{SessionColor, SessionIcon};

    /// 枚举要能稳定往返：老会话文件缺字段时按默认值解析，存回去也仍是合法值。
    #[test]
    fn session_appearance_round_trip_and_defaults() {
        let icon: SessionIcon = serde_json::from_str("\"Database\"").expect("合法图标");
        assert_eq!(icon, SessionIcon::Database);
        let color: SessionColor = serde_json::from_str("\"Green\"").expect("合法颜色");
        assert_eq!(color, SessionColor::Green);

        // 缺字段按默认值解析：老会话文件没有这两个字段时就是"跟随类型、不染色"。
        #[derive(serde::Deserialize)]
        struct Legacy {
            name: String,
        }
        let legacy: Legacy = serde_json::from_str(r#"{"name":"web-01"}"#).expect("老格式");
        assert_eq!(legacy.name, "web-01");
        assert_eq!(SessionIcon::default(), SessionIcon::Auto);
        assert_eq!(SessionColor::default(), SessionColor::None);
    }
}
