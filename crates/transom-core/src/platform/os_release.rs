//! Linux 系统发行版标识（/etc/os-release）文本解析。

/// 解析 `/etc/os-release` 内容并提取友好的操作系统名称。
///
/// 优先取 `PRETTY_NAME`，若无则回退到 `NAME`；均无时返回空字符串。
pub fn parse_os_name(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("PRETTY_NAME=") {
            return rest.trim_matches('"').trim_matches('\'').to_string();
        }
    }
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("NAME=") {
            return rest.trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pretty_name_preferentially() {
        let sample = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\nVERSION_ID=\"24.04\"";
        assert_eq!(parse_os_name(sample), "Ubuntu 24.04 LTS");
    }

    #[test]
    fn falls_back_to_name_when_pretty_name_missing() {
        let sample = "NAME=Debian\nVERSION_ID=12";
        assert_eq!(parse_os_name(sample), "Debian");
    }
}
