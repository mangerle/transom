use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xff) as u8,
            g: ((hex >> 8) & 0xff) as u8,
            b: (hex & 0xff) as u8,
        }
    }

    pub const fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// 解析 `#RRGGBB` 或 `RRGGBB` 形式的十六进制颜色字面量。
    ///
    /// 主题 JSON 里的终端配色用 hex 字符串表达（人可读、与外部主题格式一致），
    /// `Rgb` 自身的 serde 是数值对象形式，字符串解析走这里。
    pub fn parse_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#').unwrap_or(text);
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        Some(Self::from_hex(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let color = Rgb::from_hex(0x1e2127);
        assert_eq!(color, Rgb::new(0x1e, 0x21, 0x27));
        assert_eq!(color.to_hex(), 0x1e2127);
    }
}
