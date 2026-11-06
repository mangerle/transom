//! porcelain 输出的字节级切分工具。
//!
//! 设计原理（Why）：git 的路径与正文段可能含空格、换行乃至分隔符本身，
//! 因此切分一律在字节层做，并且提供「限定段数、余量并入最后一段」的形态，
//! 避免 `split_whitespace` 把 `dir with space/a b.txt` 切成三截。

/// 按 `sep` 最多切出 `n` 段，最后一段保留剩余全部内容。
///
/// 段数不足时返回实际段数，由调用方判定为格式异常。
pub(crate) fn split_limited(input: &[u8], sep: u8, n: usize) -> Vec<&[u8]> {
    let mut fields = Vec::with_capacity(n);
    let mut rest = input;
    for _ in 1..n {
        match rest.iter().position(|b| *b == sep) {
            Some(at) => {
                fields.push(&rest[..at]);
                rest = &rest[at + 1..];
            }
            None => {
                fields.push(rest);
                return fields;
            }
        }
    }
    fields.push(rest);
    fields
}

/// 宽松解码：非 UTF-8 路径以替换字符呈现，不中断整批解析。
pub(crate) fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::split_limited;

    #[test]
    fn keeps_the_remainder_in_the_last_field() {
        let bytes = b"1 AM N... 100644 100644 100644 h1 h2 dir with space/a b.txt";
        let fields = split_limited(bytes, b' ', 9);
        assert_eq!(fields.len(), 9);
        assert_eq!(fields[8], b"dir with space/a b.txt");
    }

    #[test]
    fn returns_actual_count_when_short() {
        let fields = split_limited(b"a b", b' ', 5);
        assert_eq!(fields.len(), 2);
    }
}
