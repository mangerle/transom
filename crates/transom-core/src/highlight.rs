//! 终端输出关键词规则集高亮：一行文本一次遍历，产出所有该上色的列区间。
//!
//! 为什么不用正则表达式引擎：这条特性是**常驻**的（不像检索只在搜索栏展开时工作），
//! 每帧要为可见区的每一行做匹配。用 regex 就是"行数 × 规则数 × 每行长度"的代价，
//! 落在渲染热点上会把刷屏变成幻灯片；而这里需要的模式（有限的日志级别词、IPv4、UUID）
//! 都能在一趟字符扫描里用字节判定完成，规则数只影响"同一位置试几条"的常数，
//! 总代价仍是每行一次遍历。
//!
//! 代价与局限：自定义关键词只做字面量匹配（不支持正则、不支持通配），
//! 想要"命中正则就上色"属于路线图里的触发器系统（#25），那条才有回调与动作语义。

use serde::{Deserialize, Serialize};

/// 内建的日志级别关键词及其色相归属。
///
/// 只匹配大写形式是刻意的：`ERROR`/`WARN` 在日志里是级别字段，而 `error` 大量出现在
/// 英文正文与 `error=` 之类的键名里；大小写不敏感会把整屏英文单词都染上色，
/// 信号立刻变成噪声。需要照顾小写口径的用户，把词加进 `keywords` 即可（自定义词不分大小写）。
const PRESET_KEYWORDS: [(&[u8], HighlightKind); 6] = [
    (b"ERROR", HighlightKind::Error),
    (b"FATAL", HighlightKind::Error),
    (b"WARN", HighlightKind::Warning),
    (b"WARNING", HighlightKind::Warning),
    (b"INFO", HighlightKind::Info),
    (b"DEBUG", HighlightKind::Debug),
];

/// 自定义关键词数量上限：每条都要在每个字符位置上试一次，超过这个量就该换成词典树。
const MAX_KEYWORDS: usize = 32;
/// 单条自定义关键词的长度上限。
const MAX_KEYWORD_CHARS: usize = 40;

/// 高亮类别。颜色不在这里定死，而是映射到会话调色板的 ANSI 槽位：
/// 换主题（One Dark / Dracula / 浅色）时自动跟着走，比固定色值更可读。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// 错误级别（ERROR/FATAL）。
    Error,
    /// 告警级别（WARN/WARNING）。
    Warning,
    /// 信息级别（INFO）。
    Info,
    /// 调试级别（DEBUG）。
    Debug,
    /// IPv4 地址。
    Ipv4,
    /// UUID。
    Uuid,
    /// 用户自定义关键词。
    Custom,
}

impl HighlightKind {
    /// 取会话调色板里对应 ANSI 槽位的颜色（0..16）。
    ///
    /// 一律用 bright 段（8..15）：常规段与日志本身常用的着色撞车，
    /// 例如程序自己就用红字打 ERROR 时，再叠一层同色等于没有强调。
    pub fn ansi_index(self) -> usize {
        match self {
            HighlightKind::Error => 9,
            HighlightKind::Warning => 11,
            HighlightKind::Info => 12,
            HighlightKind::Debug => 7,
            HighlightKind::Ipv4 => 14,
            HighlightKind::Uuid => 13,
            HighlightKind::Custom => 10,
        }
    }
}

/// 一行内需要上色的区间，下标是**字符序号**（不是字节），区间为 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}

/// 关键词高亮配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHighlight {
    /// 总开关。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 自定义关键词，匹配时不区分大小写；保存前经 [`TerminalHighlight::sanitized`] 清洗。
    #[serde(default)]
    pub keywords: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

impl Default for TerminalHighlight {
    fn default() -> Self {
        Self {
            enabled: true,
            keywords: Vec::new(),
        }
    }
}

impl TerminalHighlight {
    /// 清洗用户输入：去首尾空白、丢空项、不区分大小写去重、限长限条数。
    ///
    /// 设置文件属外部输入（可手工编辑、可被导入），不能假设条目已经合法。
    pub fn sanitized(self) -> Self {
        let mut keywords: Vec<String> = Vec::with_capacity(self.keywords.len().min(MAX_KEYWORDS));
        for keyword in self.keywords {
            let trimmed = keyword.trim();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_KEYWORD_CHARS {
                continue;
            }
            let lowered = trimmed.to_lowercase();
            if keywords
                .iter()
                .any(|existing| existing.to_lowercase() == lowered)
            {
                continue;
            }
            if keywords.len() >= MAX_KEYWORDS {
                break;
            }
            keywords.push(trimmed.to_string());
        }
        Self {
            enabled: self.enabled,
            keywords,
        }
    }

    pub fn is_empty_keywords(&self) -> bool {
        self.keywords.is_empty()
    }
}

/// 编译后的规则集：自定义关键词预先转成小写字符向量。
///
/// 为什么不每次扫描现场转：渲染层每帧要为可见区每一行调用一次，逐行 `to_lowercase + collect`
/// 会把"每行一趟线性扫描"变成"每行按关键词数分配"，规则一多成本模型就崩了。
/// 编译一次（每帧一次）后，扫描路径里只剩字符比较。
#[derive(Debug, Default, Clone)]
pub struct CompiledHighlight {
    keywords: Vec<Vec<char>>,
}

impl CompiledHighlight {
    /// 按当前配置编译规则集；关闭状态或无自定义关键词时得到空表。
    pub fn compile(config: &TerminalHighlight) -> Self {
        if !config.enabled {
            return Self::default();
        }
        Self {
            keywords: config
                .keywords
                .iter()
                .map(|keyword| keyword.to_lowercase().chars().collect())
                .collect(),
        }
    }

    /// 扫描一行文本，返回按起点升序、互不重叠的高亮区间。
    ///
    /// 位置冲突时先到先得，匹配成功后游标跳到区间末尾：这样 `192.168.0.1` 里的
    /// `0.1` 不会被再次当成别的命中重复上色。
    pub fn scan(&self, text: &str, config: &TerminalHighlight) -> Vec<HighlightSpan> {
        if !config.enabled || text.is_empty() {
            return Vec::new();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut spans = Vec::new();
        let mut index = 0usize;
        while index < chars.len() {
            if let Some((end, kind)) = self.match_at(&chars, index) {
                spans.push(HighlightSpan {
                    start: index,
                    end,
                    kind,
                });
                index = end;
                continue;
            }
            index += 1;
        }
        spans
    }

    /// 在 `index` 处尝试所有规则；返回匹配结束的下一个下标。
    ///
    /// 顺序是内建级别词 → 自定义关键词 → IPv4 → UUID：同一位置先到先得，
    /// 用户把 `ERROR` 误加进自定义列表也不会改变它的颜色归属。
    fn match_at(&self, chars: &[char], index: usize) -> Option<(usize, HighlightKind)> {
        if !boundary_ok(chars, index) {
            return None;
        }
        if let Some(found) =
            match_preset(chars, index).or_else(|| match_keyword(chars, index, &self.keywords))
        {
            return Some(found);
        }
        if let Some(end) = match_ipv4(chars, index) {
            return Some((end, HighlightKind::Ipv4));
        }
        match_uuid(chars, index).map(|end| (end, HighlightKind::Uuid))
    }
}

/// 匹配点左侧必须是词边界，否则 `INFORMATION` 会被切成 `INFO`。
fn boundary_ok(chars: &[char], index: usize) -> bool {
    index == 0 || !is_word_char(chars[index - 1])
}

fn boundary_end(chars: &[char], end: usize) -> bool {
    end >= chars.len() || !is_word_char(chars[end])
}

/// 词延续字符：只把 ASCII 字母数字与 `_ . -` 当作"同一个词"。
///
/// 汉字刻意排除在外：CJK 之间没有词边界概念，若把汉字算作延续字符，
/// `磁盘空间不足` 里的自定义词"空间不足"就永远命中不了——而这正是中文日志最常见的形态。
/// 代价是 `1.2.3.4版本` 这类紧贴汉字的情况也会命中地址，这是可接受的宽松。
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

fn match_preset(chars: &[char], index: usize) -> Option<(usize, HighlightKind)> {
    for (bytes, kind) in PRESET_KEYWORDS {
        let len = bytes.len();
        if !fits(chars, index, len) {
            continue;
        }
        let matched = bytes
            .iter()
            .enumerate()
            .all(|(offset, expect)| chars[index + offset] == char::from(*expect));
        if matched && boundary_end(chars, index + len) {
            return Some((index + len, kind));
        }
    }
    None
}

/// 自定义关键词：不区分大小写，同样要求两侧词边界。
fn match_keyword(
    chars: &[char],
    index: usize,
    keywords: &[Vec<char>],
) -> Option<(usize, HighlightKind)> {
    for letters in keywords {
        if !fits(chars, index, letters.len()) {
            continue;
        }
        let matched = letters.iter().enumerate().all(|(offset, expect)| {
            let actual = chars[index + offset];
            // 关键词已预转小写：ASCII 走 to_ascii_lowercase，非 ASCII（中文等）本来就等价比较。
            actual == *expect || actual.to_ascii_lowercase() == *expect
        });
        if matched && boundary_end(chars, index + letters.len()) {
            return Some((index + letters.len(), HighlightKind::Custom));
        }
    }
    None
}

/// IPv4：四段 1~3 位十进制、每段 ≤255，且两侧不能紧贴数字、点、横线或字母。
///
/// 尾边界要求"下一个字符不是点"，为的是不把 `1.2.3.4.5` 这种版本串的前四段当成地址。
fn match_ipv4(chars: &[char], index: usize) -> Option<usize> {
    let mut cursor = index;
    for group in 0..4 {
        cursor = parse_octet(chars, cursor)?;
        if group < 3 {
            if chars.get(cursor) != Some(&'.') {
                return None;
            }
            cursor += 1;
        }
    }
    // 首字符必须是数字（前面已由 boundary_ok 保证不是词字符），这里再排除前导点之类。
    if !chars[index].is_ascii_digit() {
        return None;
    }
    let next = chars.get(cursor);
    (next.is_none_or(|c| !c.is_ascii_digit() && *c != '.' && !c.is_alphabetic() && *c != '-'))
        .then_some(cursor)
}

fn parse_octet(chars: &[char], start: usize) -> Option<usize> {
    let mut cursor = start;
    let mut value = 0u16;
    let mut digits = 0usize;
    while digits < 3 && chars.get(cursor).is_some_and(char::is_ascii_digit) {
        value = value * 10 + u16::from(chars[cursor].digit_value());
        cursor += 1;
        digits += 1;
    }
    if digits == 0 || value > 255 {
        return None;
    }
    Some(cursor)
}

/// UUID：8-4-4-4-12 的十六进制段，横线分隔，两侧不能紧贴十六进制字符或横线。
fn match_uuid(chars: &[char], index: usize) -> Option<usize> {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut cursor = index;
    for (position, length) in GROUPS.iter().enumerate() {
        if !fits(chars, cursor, *length) {
            return None;
        }
        if !chars[cursor..cursor + length]
            .iter()
            .all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        cursor += length;
        if position < GROUPS.len() - 1 {
            if chars.get(cursor) != Some(&'-') {
                return None;
            }
            cursor += 1;
        }
    }
    let next = chars.get(cursor);
    (next.is_none_or(|c| !c.is_ascii_hexdigit() && *c != '-')).then_some(cursor)
}

fn fits(chars: &[char], index: usize, length: usize) -> bool {
    index + length <= chars.len()
}

/// 解析一段八位组：最多 3 位，且数值不超过 255；溢出或位数不足即失败。
trait DigitValue {
    /// ASCII 数字转数值；调用前已用 `is_ascii_digit` 判定，非数字不会走到这里。
    fn digit_value(self) -> u8;
}

impl DigitValue for char {
    fn digit_value(self) -> u8 {
        self as u8 - b'0'
    }
}

/// 分隔符集合：半角/全角逗号、顿号、分号与任意空白都能分隔一个关键词。
const KEYWORD_SEPARATORS: [char; 5] = [',', '，', '、', ';', '；'];

impl TerminalHighlight {
    /// 把设置里的"关键词列表"文本切成条目。
    ///
    /// 中文习惯里用顿号与全角逗号列举，只按半角逗号拆分会让用户以为输入没生效。
    pub fn parse_keywords(raw: &str) -> Vec<String> {
        raw.split(|c: char| c.is_whitespace() || KEYWORD_SEPARATORS.contains(&c))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// 渲染回输入框文本（保存后回填用）。
    pub fn keywords_joined(&self) -> String {
        self.keywords.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::{CompiledHighlight, HighlightKind, HighlightSpan, TerminalHighlight};

    fn config(keywords: &[&str]) -> TerminalHighlight {
        TerminalHighlight {
            enabled: true,
            keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
        }
    }

    /// 走完整入口：编译规则集再扫描一行。
    fn scan(text: &str, keywords: &[&str]) -> Vec<HighlightSpan> {
        let config = config(keywords);
        CompiledHighlight::compile(&config).scan(text, &config)
    }

    fn kinds(text: &str, keywords: &[&str]) -> Vec<HighlightKind> {
        scan(text, keywords)
            .into_iter()
            .map(|span| span.kind)
            .collect()
    }

    fn spans_of(text: &str, keywords: &[&str]) -> Vec<(usize, usize, HighlightKind)> {
        scan(text, keywords)
            .into_iter()
            .map(|span| (span.start, span.end, span.kind))
            .collect()
    }

    #[test]
    fn highlights_log_levels() {
        assert_eq!(kinds("ERROR boom", &[]), vec![HighlightKind::Error]);
        assert_eq!(kinds("[WARN] slow", &[]), vec![HighlightKind::Warning]);
        assert_eq!(kinds("WARNING: low", &[]), vec![HighlightKind::Warning]);
        assert_eq!(
            kinds("info started", &[]),
            Vec::<HighlightKind>::new(),
            "小写 info 属于正文"
        );
        assert_eq!(
            kinds("DEBUG=1 INFO=2 FATAL", &[]),
            vec![
                HighlightKind::Debug,
                HighlightKind::Info,
                HighlightKind::Error
            ]
        );
    }

    /// `INFORMATION` 不该被切成 `INFO`+剩下；`ERRORS` 同理。词边界是这套规则的底线。
    #[test]
    fn respects_word_boundaries() {
        assert_eq!(kinds("INFORMATION", &[]), Vec::<HighlightKind>::new());
        assert_eq!(kinds("ERRORS happen", &[]), Vec::<HighlightKind>::new());
        assert_eq!(kinds("MY_ERROR=1", &[]), Vec::<HighlightKind>::new());
        // 但标点与括号分隔的级别词要能命中。
        assert_eq!(kinds("(ERROR)", &[]), vec![HighlightKind::Error]);
        assert_eq!(kinds("level:ERROR", &[]), vec![HighlightKind::Error]);
    }

    #[test]
    fn highlights_ipv4_only_when_valid() {
        assert_eq!(kinds("from 192.168.1.1 ok", &[]), vec![HighlightKind::Ipv4]);
        assert_eq!(kinds("0.0.0.0", &[]), vec![HighlightKind::Ipv4]);
        assert_eq!(kinds("255.255.255.255", &[]), vec![HighlightKind::Ipv4]);
        assert_eq!(
            kinds("256.1.1.1", &[]),
            Vec::<HighlightKind>::new(),
            "段值越界不算地址"
        );
        assert_eq!(
            kinds("1.2.3", &[]),
            Vec::<HighlightKind>::new(),
            "三段不算地址"
        );
        assert_eq!(
            kinds("1.2.3.4.5", &[]),
            Vec::<HighlightKind>::new(),
            "五段版本串不能只染前四段"
        );
        assert_eq!(
            kinds("v1.2.3.4", &[]),
            Vec::<HighlightKind>::new(),
            "前面紧贴版本号前缀时不算地址"
        );
    }

    #[test]
    fn highlights_uuid() {
        assert_eq!(
            kinds("id=6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4b", &[]),
            vec![HighlightKind::Uuid]
        );
        assert_eq!(
            kinds("6F1C2B3A-4D5E-4F60-7A8B-9C0D1E2F3A4B", &[]),
            vec![HighlightKind::Uuid],
            "大写十六进制同样命中"
        );
        assert_eq!(
            kinds("6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4", &[]),
            Vec::<HighlightKind>::new(),
            "最后一段少一位不算 UUID"
        );
    }

    /// 区间是字符下标且互不重叠：渲染层要按它切分文本 run。
    #[test]
    fn spans_are_ordered_and_non_overlapping() {
        let found = spans_of(
            "ERROR at 10.0.0.1 id 6f1c2b3a-4d5e-4f60-7a8b-9c0d1e2f3a4b",
            &[],
        );
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].1, 5, "ERROR 占 0..5");
        assert!(found[0].1 <= found[1].0, "区间必须升序且不重叠");
        assert!(found[1].1 <= found[2].0);
    }

    /// 中文等多字节字符按字符而不是字节计数，否则列位会整体错位。
    #[test]
    fn counts_characters_not_bytes() {
        let found = spans_of("服务 ERROR 已发", &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 3, "三个汉字后是第 3 个字符");
        assert_eq!(found[0].1, 8);
    }

    #[test]
    fn custom_keywords_are_case_insensitive() {
        assert_eq!(
            kinds("connection refused", &["refused"]),
            vec![HighlightKind::Custom]
        );
        assert_eq!(kinds("REFUSED", &["refused"]), vec![HighlightKind::Custom]);
        assert_eq!(
            kinds("refusedly", &["refused"]),
            Vec::<HighlightKind>::new()
        );
        // 中文关键词也能命中。
        assert_eq!(
            kinds("磁盘空间不足", &["空间不足"]),
            vec![HighlightKind::Custom]
        );
    }

    #[test]
    fn preset_wins_over_custom_at_same_position() {
        // 同一位置上先试内建级别词，避免用户误把 ERROR 加进自定义列表后颜色被覆盖。
        assert_eq!(kinds("ERROR", &["error"]), vec![HighlightKind::Error]);
    }

    #[test]
    fn disabled_config_scans_nothing() {
        let config = TerminalHighlight {
            enabled: false,
            keywords: vec!["ERROR".into()],
        };
        let compiled = CompiledHighlight::compile(&config);
        assert!(compiled.scan("ERROR 1.2.3.4", &config).is_empty());
        assert!(
            compiled.keywords.is_empty(),
            "关闭状态下不该编译出任何关键词"
        );
        assert!(scan("", &[]).is_empty());
    }

    #[test]
    fn parses_keywords_across_chinese_separators() {
        assert_eq!(
            TerminalHighlight::parse_keywords("timeout, refused、连接失败;oom  killed"),
            vec!["timeout", "refused", "连接失败", "oom", "killed"]
        );
        assert!(TerminalHighlight::parse_keywords("  , ， 、 ;;  ").is_empty());
        let highlight = TerminalHighlight {
            enabled: true,
            keywords: vec!["oom".into(), "killed".into()],
        };
        assert_eq!(highlight.keywords_joined(), "oom, killed");
    }

    #[test]
    fn sanitize_drops_blank_dedupes_and_caps() {
        let raw = TerminalHighlight {
            enabled: true,
            keywords: vec![
                "  warn ".into(),
                "WARN".into(),
                "".into(),
                "   ".into(),
                "x".repeat(41),
                "disk".into(),
            ],
        };
        let cleaned = raw.sanitized();
        assert_eq!(
            cleaned.keywords,
            vec!["warn", "disk"],
            "去空白、不分大小写去重、丢超长项"
        );
        let many = TerminalHighlight {
            enabled: true,
            keywords: (0..60).map(|index| format!("k{index}")).collect(),
        };
        assert_eq!(many.sanitized().keywords.len(), 32, "条数上限必须生效");
    }

    /// 颜色映射到调色板槽位，越界就会画坏画面，因此逐个钉住。
    #[test]
    fn ansi_indices_stay_inside_palette() {
        for kind in [
            HighlightKind::Error,
            HighlightKind::Warning,
            HighlightKind::Info,
            HighlightKind::Debug,
            HighlightKind::Ipv4,
            HighlightKind::Uuid,
            HighlightKind::Custom,
        ] {
            assert!((0..16).contains(&kind.ansi_index()), "{kind:?}");
        }
    }
}
