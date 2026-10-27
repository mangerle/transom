//! MAC 地址新类型：Wake-on-LAN 目标网卡的解析与规范化。
//!
//! 设计原理（Why）：
//! - 实现初衷：MAC 在 WoL 里是唯一的寻址依据，写错一位就永远唤不醒，而字符串
//!   形态允许 `aa:bb:cc:dd:ee:ff`、`aa-bb-...`、`aabbccddeeff` 与大小写混写
//!   四种常见录入形式（用户从 `ip link`、iDRAC、纸质标签上抄下来的样子）。
//!   用新类型把"解析一次、之后必然是 6 字节"变成类型保证，避免组包处再校验；
//! - 核心优势：`Display` 只输出规范形式，会话文件与界面回填都同一口径，
//!   比对"两台机器是否同一网卡"时可以直接比字符串；
//! - 代价与局限：不接受省略形式（如 `aa:bb:*`）与 Wake Pattern 的 IPv6/IBT
//!   扩展报文，这些在运维场景里极少用到。

use std::fmt;
use std::str::FromStr;

/// 48 位以太网地址；内部始终是 6 字节，构造即校验。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    /// 组包用的原始字节（Magic Packet 的后 16 份重复体）。
    pub fn octets(&self) -> [u8; 6] {
        self.0
    }

    /// 规范文本 `aa:bb:cc:dd:ee:ff`（小写），供落盘与界面回填。
    pub fn to_canonical(&self) -> String {
        self.0
            .iter()
            .map(|octet| format!("{octet:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

/// 解析错误：中文描述携带原始输入，便于表单直接展示。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("MAC 地址格式非法，原始输入: {0}，应为 6 组十六进制，如 aa:bb:cc:dd:ee:ff")]
pub struct MacParseError(String);

impl FromStr for MacAddress {
    type Err = MacParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let trimmed = input.trim();
        // 先按分隔符切，无分隔符时按整串 12 位十六进制处理。
        let digits: Vec<u8> = if trimmed.contains([':', '-']) {
            trimmed
                .split([':', '-'])
                .map(|group| {
                    if group.len() != 2 {
                        return Err(MacParseError(input.to_string()));
                    }
                    u8::from_str_radix(group, 16).map_err(|_| MacParseError(input.to_string()))
                })
                .collect::<Result<_, _>>()?
        } else {
            let bytes = trimmed.as_bytes();
            if bytes.len() != 12 {
                return Err(MacParseError(input.to_string()));
            }
            let mut octets = Vec::with_capacity(6);
            for pair in bytes.chunks(2) {
                let text = std::str::from_utf8(pair)
                    .map_err(|_| MacParseError(input.to_string()))
                    .and_then(|text| {
                        u8::from_str_radix(text, 16).map_err(|_| MacParseError(input.to_string()))
                    })?;
                octets.push(text);
            }
            octets
        };
        if digits.len() != 6 {
            return Err(MacParseError(input.to_string()));
        }
        let mut octets = [0u8; 6];
        octets.copy_from_slice(&digits);
        Ok(Self(octets))
    }
}

/// 校验并规范化用户录入的 MAC；空串视为"未配置"。
///
/// 会话文件里存的是文本而不是新类型：`Option<String>` 让单个坏值只影响这一条
/// 会话的唤醒功能（发唤醒时报明确错误），不会因为一次手改就把整个会话库
/// 反序列化打挂。表单录入走本函数立即拒绝，落盘值在发送前再校验一次。
pub fn normalize_mac(input: &str) -> Result<Option<String>, MacParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    MacAddress::from_str(trimmed).map(|mac| Some(mac.to_canonical()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_input_forms() {
        let expected = MacAddress::from_str("aa:bb:cc:dd:ee:ff").unwrap();
        for form in [
            "aa:bb:cc:dd:ee:ff",
            "AA:BB:CC:DD:EE:FF",
            "aa-bb-cc-dd-ee-ff",
            "aabbccddeeff",
            "  aa:bb:cc:dd:ee:ff  ",
        ] {
            assert_eq!(
                MacAddress::from_str(form).ok(),
                Some(expected),
                "应接受录入形式: {form}"
            );
        }
        assert_eq!(expected.octets(), [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        assert_eq!(expected.to_canonical(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn rejects_malformed_values() {
        for form in [
            "",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:ff:00",
            "gg:bb:cc:dd:ee:ff",
            "a:b:c:d:e:f",
            "aabbccddeef",
            "aabbccddeeff0",
            "not a mac",
        ] {
            assert!(
                MacAddress::from_str(form).is_err(),
                "应拒绝非法输入: {form}"
            );
        }
    }

    #[test]
    fn normalize_treats_blank_as_unconfigured() {
        assert_eq!(normalize_mac("   ").unwrap(), None);
        assert_eq!(
            normalize_mac("AA-BB-CC-DD-EE-FF").unwrap().as_deref(),
            Some("aa:bb:cc:dd:ee:ff")
        );
        assert!(normalize_mac("zz:bb:cc:dd:ee:ff").is_err());
    }
}
