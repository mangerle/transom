//! 终端输入补全：词位判定、候选匹配与合并的纯文本算法。
//!
//! 设计原理（Why）：
//! - 实现初衷：输入即补全要"按正在输入的词在命令行里的位置"给候选——命令位整库
//!   模糊搜索（敲「磁盘」能找到 `df -h`），参数位只接得上前文的写法（`git re` →
//!   `git remote add …`），对不上就不弹，把 Tab 留给 shell 自己的路径补全；
//! - 核心优势：全部是纯文本函数，不依赖窗口与实体，可整体单测；候选条目在构造时
//!   预计算小写形式，按键热路径上不再逐条分配堆字符串；参数位用"词对齐"判定，
//!   宁可少提示，也不把候选接到语义对不上的前文后面；
//! - 代价与局限：切词按空白与 `; | & ( )` 分隔符进行，不理解引号内的空格与转义，
//!   引号场景下词位可能判错——表现为这次不弹或提示错位，不会写坏命令行；
//!   选项候选没有静态选项表，靠候选命令里出现过的写法统计，覆盖面随语料增长。

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use super::settings::Snippet;

/// 候选的来源，决定浮层上的徽标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    /// 用户自己保存的命令片段。
    Snippet,
    /// 内置命令表里的命令。
    Command,
    /// 从接得上前文的命令里抽出的选项（参数位输入 `-` 时）。
    Flag,
    /// 执行过的历史命令行。
    History,
}

/// 参与补全的一条候选：片段、内置命令、命令里出现过的选项，或历史命令行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    /// 浮层第一列显示的名称；历史候选就是命令本身。
    pub name: String,
    /// 接受补全时写进终端的文本；选项候选就是选项本身。
    pub command: String,
    /// 补充说明；选项候选用它显示来源命令。
    pub note: String,
    /// 预计算的小写形式：匹配在每次按键时全表扫描，现场 `to_lowercase`
    /// 会对每个条目分配 2-3 个堆字符串，构造时算一次就能省掉。
    pub name_lower: String,
    pub command_lower: String,
    pub note_lower: String,
    pub kind: CompletionKind,
}

impl CompletionItem {
    pub fn new(name: String, command: String, note: String, kind: CompletionKind) -> Self {
        Self {
            name_lower: name.to_lowercase(),
            command_lower: command.to_lowercase(),
            note_lower: note.to_lowercase(),
            name,
            command,
            note,
            kind,
        }
    }
}

/// 当前词达到这个长度才浮出候选：一两个字符就弹会太吵。
pub const MIN_TOKEN_CHARS: usize = 2;
/// 浮层最多展示的候选数。
pub const MAX_MATCHES: usize = 8;
/// 历史命令行在浮层上最多显示多少个字符：整条命令可能长达几千字符，
/// 全铺会把浮层撑出窗格，所以只截显示名，`command` 仍是完整一条。
pub const MAX_NAME_CHARS: usize = 80;

/// 补全候选池：历史命令行与用户片段的预计算集合。
///
/// 历史条目排最前：参数位的续写按池序取前若干条，最近用过的写法要排在
/// 片段前面；命令位命中同样的词时，历史也按"最近使用"排最前。
#[derive(Debug, Default, Clone)]
pub struct CompletionPool {
    items: Vec<CompletionItem>,
}

impl CompletionPool {
    /// 由历史与片段构建候选池；与片段同文的历史条目跳过，浮层里同一条不出现两遍。
    pub fn build(history: &[String], snippets: &[Snippet]) -> Self {
        let presets: Vec<CompletionItem> = snippets
            .iter()
            .map(|snippet| {
                CompletionItem::new(
                    snippet.name.clone(),
                    snippet.command.clone(),
                    String::new(),
                    CompletionKind::Snippet,
                )
            })
            .collect();
        let mut items = history_items(history, &presets);
        items.extend(presets);
        Self { items }
    }

    /// 池内容的指纹：历史与片段任一增删改都会变，调用方据此缓存构建结果，
    /// 避免每次按键重建几百条候选。
    pub fn fingerprint(history: &[String], snippets: &[Snippet]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        history.iter().for_each(|entry| entry.hash(&mut hasher));
        snippets.iter().for_each(|snippet| {
            (snippet.name.as_str(), snippet.command.as_str()).hash(&mut hasher)
        });
        hasher.finish()
    }

    pub fn items(&self) -> &[CompletionItem] {
        &self.items
    }

    /// 按词位给候选；`token` 是正在输入的词，`before_token` 是它之前的文本。
    pub fn complete(&self, token: &str, before_token: &str) -> Vec<CompletionItem> {
        complete(&self.items, token, before_token)
    }
}

/// 历史命令行变成候选：整条就是接受补全时写进终端的文本，最新一条排最前。
///
/// 片段里已有的同一条命令跳过，免得浮层里同一行出现两遍；
/// 名称只留开头若干字符用于显示，`command` 仍是完整一条。
pub fn history_items(entries: &[String], presets: &[CompletionItem]) -> Vec<CompletionItem> {
    let taken: HashSet<&str> = presets.iter().map(|item| item.command.as_str()).collect();
    entries
        .iter()
        .filter(|entry| !taken.contains(entry.as_str()))
        .map(|entry| {
            CompletionItem::new(
                ellipsis(entry, MAX_NAME_CHARS),
                entry.clone(),
                String::new(),
                CompletionKind::History,
            )
        })
        .collect()
}

/// 超过 `max_chars` 就截到该字符数并加省略号（按字符切，不切在多字节中间）。
fn ellipsis(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

/// 文本里最后一个词（按空白 / 分隔符切分）。
pub fn last_token(text: &str) -> &str {
    text.rsplit(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')'))
        .next()
        .unwrap_or("")
}

/// 正在输入的词在命令行里的角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenSlot {
    /// 命令段的第一个词：填的是命令本身。
    Command,
    /// 命令段其后的词：填的是参数，不能再拿它去撞命令名。
    Argument,
}

/// 不改命令语义的包装词：`sudo ` 之后用户要的仍然是「哪条命令」，不是
/// 「喂给 sudo 的参数」。这类词单独占段首时，正在输入的词按命令位对待——
/// 否则 `sudo dock` 只能匹配库里以 sudo 开头的片段，提示会完全落空。
const COMMAND_WRAPPERS: &[&str] = &["sudo", "doas", "nohup", "time", "nice"];

/// 从段首剥掉连续的包装词，剩下的词才决定「这条命令段有没有前文」。
fn without_wrappers<'a>(words: &'a [&'a str]) -> &'a [&'a str] {
    let mut kept = words;
    while let [head, rest @ ..] = kept
        && COMMAND_WRAPPERS
            .iter()
            .any(|wrapper| head.eq_ignore_ascii_case(wrapper))
    {
        kept = rest;
    }
    kept
}

/// 正在输入的词落在命令位还是参数位。
///
/// `before_token` 是光标前、不含正在输入那个词的文本：按 `; | & ( )` 切成命令段，
/// 词落在段首就是命令位。`cd find` 里的 `find` 在 `cd` 之后，是参数位，
/// 不该再因为片段库里有 `sudo find / -name …` 就把它当命令提示出来。
/// 包装词（`sudo` 等）不算前文：`sudo dock` 里的 `dock` 仍是命令位。
fn token_slot(before_token: &str) -> TokenSlot {
    if without_wrappers(&segment_words(before_token)).is_empty() {
        TokenSlot::Command
    } else {
        TokenSlot::Argument
    }
}

/// `before_token` 里当前命令段已经写下的词（不含正在输入的词）。
fn segment_words(before_token: &str) -> Vec<&str> {
    before_token
        .rsplit([';', '|', '&', '(', ')'])
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect()
}

/// 命令开头的若干词是否与段末尾的词一一对齐，返回对齐的词数。
///
/// 只认「对齐到段尾」的情况，也就是候选命令正好接在正在输入的词前面：
/// 段末尾是 `cd` 时，`sudo find / -name …` 对不上（开头是 `sudo`），
/// 段末尾是 `sudo apt` 时 `sudo apt install …` 对得上（2 个词）。
fn aligned_word_count(command: &str, segment: &[&str]) -> Option<usize> {
    let words: Vec<&str> = command.split_whitespace().collect();
    (1..=segment.len().min(words.len())).rev().find(|&k| {
        words[..k]
            .iter()
            .zip(&segment[segment.len() - k..])
            .all(|(head, typed)| head.eq_ignore_ascii_case(typed))
    })
}

/// 参数位候选：命令接得上前文，且紧接正在输入的词的那个词以它开头。
///
/// 词为空（刚敲完空格）时列出这条命令后面的所有写法，等于「接着能填什么」的
/// 速查；片段本身的顺序保留（历史在池序更前）。
fn continuations(items: &[CompletionItem], token: &str, segment: &[&str]) -> Vec<usize> {
    let needle = token.to_lowercase();
    items
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| {
            let words: Vec<&str> = item.command.split_whitespace().collect();
            let aligned = aligned_word_count(&item.command, segment)?;
            let next = words.get(aligned)?.to_lowercase();
            next.starts_with(&needle).then_some(slot)
        })
        .take(MAX_MATCHES)
        .collect()
}

/// `-y` / `--all` / `-k5` 这样以 `-` 开头、紧跟字母数字的词才算选项；
/// 单独的 `-` 与 `--`（读 stdin / 选项结束标记）不算。
fn is_flag(word: &str) -> bool {
    let body = word.trim_start_matches('-');
    word.starts_with('-')
        && word.len() > body.len()
        && body.chars().next().is_some_and(|c| c.is_alphanumeric())
}

/// 参数位输入 `-` 时的候选：从接得上前文的命令里，取接上位置之后出现的选项。
///
/// 候选没有选项表可依，就按片段与历史里出现的次数排；小写用于比较与去重，
/// 写回终端的仍是候选里的原大小写。
fn flag_options(items: &[CompletionItem], token: &str, segment: &[&str]) -> Vec<CompletionItem> {
    let needle = token.to_lowercase();
    // (原文, 小写, 次数, 来源命令)。小写只算一次存下来：这里每个按键都跑一遍，
    // 逐对比较各 to_lowercase() 是 O(n^2) 次堆分配（CompletionItem 已为此
    // 预计算过小写字段，同一原则）。
    let mut found: Vec<(String, String, u32, String)> = Vec::new();
    for item in items {
        let words: Vec<&str> = item.command.split_whitespace().collect();
        let Some(aligned) = aligned_word_count(&item.command, segment) else {
            continue;
        };
        for word in &words[aligned..] {
            if !is_flag(word) {
                continue;
            }
            let lower = word.to_lowercase();
            if !lower.starts_with(&needle) {
                continue;
            }
            match found
                .iter_mut()
                .find(|(flag, _, _, _)| flag.eq_ignore_ascii_case(word))
            {
                Some((_, _, count, _)) => *count += 1,
                None => found.push((word.to_string(), lower, 1, item.command.clone())),
            }
        }
    }
    found.sort_by(
        |(a_flag, a_lower, a_count, _), (b_flag, b_lower, b_count, _)| {
            b_count
                .cmp(a_count)
                .then_with(|| a_lower.cmp(b_lower))
                .then_with(|| a_flag.cmp(b_flag))
        },
    );
    found
        .into_iter()
        .take(MAX_MATCHES)
        .map(|(flag, _, _, origin)| {
            CompletionItem::new(flag.clone(), flag, origin, CompletionKind::Flag)
        })
        .collect()
}

/// 按词在命令行里的位置给候选：命令位整库模糊搜索，参数位只接受接得上前文的
/// 续写与选项，参数填的是路径 / 包名之类的东西，命中不了就完全不弹（Tab 交回
/// shell 自己的补全）。
///
/// 候选池里除了片段还有历史命令行：命中同样的词时，最近用过的排最前。
pub fn complete(items: &[CompletionItem], token: &str, before_token: &str) -> Vec<CompletionItem> {
    let token = token.trim();
    let segment = segment_words(before_token);
    let slots = match token_slot(before_token) {
        TokenSlot::Command => recent_first(fuzzy_matches(items, token), items),
        TokenSlot::Argument if token.starts_with('-') => {
            return flag_options(items, token, &segment);
        }
        TokenSlot::Argument => recent_first(continuations(items, token, &segment), items),
    };
    slots.into_iter().map(|slot| items[slot].clone()).collect()
}

/// 历史候选提到最前：最近用过的写法往往就是这次要敲的，剩下的候选保持匹配度顺序。
///
/// 候选池里历史条目本身按新→旧排列，所以组内只按下标排即可。
fn recent_first(slots: Vec<usize>, items: &[CompletionItem]) -> Vec<usize> {
    let (mut recent, rest): (Vec<usize>, Vec<usize>) = slots
        .into_iter()
        .partition(|slot| items[*slot].kind == CompletionKind::History);
    recent.sort_unstable();
    recent.into_iter().chain(rest).collect()
}

/// 命令位候选：整库模糊匹配，返回 items 下标（已按匹配度排序）。
///
/// 打分顺序：名称前缀 → 名称包含 → 命令里某个词的前缀 → 命令包含 → 备注包含。
/// 命令词前面可能带管道/引号之类的符号，比较前先剥掉。只有命令位走这里——
/// 「敲『磁盘』能找到 `df -h`」靠的就是这种宽松匹配，参数位不能用。
fn fuzzy_matches(items: &[CompletionItem], token: &str) -> Vec<usize> {
    let token = token.trim();
    if token.chars().count() < MIN_TOKEN_CHARS {
        return Vec::new();
    }
    let needle = token.to_lowercase();
    let mut scored: Vec<(i16, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| item_score(item, &needle).map(|score| (i16::from(score), slot)))
        .collect();
    scored.sort_by(|(a_score, a_slot), (b_score, b_slot)| {
        a_score
            .cmp(b_score)
            .then_with(|| items[*a_slot].name.cmp(&items[*b_slot].name))
    });
    scored
        .into_iter()
        .take(MAX_MATCHES)
        .map(|(_, slot)| slot)
        .collect()
}

fn item_score(item: &CompletionItem, needle: &str) -> Option<u8> {
    // 小写形式在构造时算好（见 `CompletionItem::new`），这里直接用。
    if item.name_lower.starts_with(needle) {
        return Some(0);
    }
    if item.name_lower.contains(needle) {
        return Some(1);
    }
    if item.command_lower.split_whitespace().any(|word| {
        word.trim_start_matches(|c: char| !c.is_alphanumeric())
            .starts_with(needle)
    }) {
        return Some(2);
    }
    if item.command_lower.contains(needle) {
        return Some(3);
    }
    if item.note_lower.contains(needle) {
        Some(4)
    } else {
        None
    }
}

/// 网格回显落后于按键时，行尾可能只有正在输入的词的前半截。
///
/// `before` 是终端网格里光标前的字符，`tracked` 是按按键累计的整行。远程 shell
/// 的回显要绕一圈才回来，刚敲完就按 Tab / Enter 时网格还没跟上；此时网格行尾正好
/// 是 `tracked` 的前缀（缺一个词以内的尾部）。终端输入按先后顺序入队，补全写入的
/// 退格仍在这些按键之后，所以这种"只差回显"的情况可以照常替换。
pub fn echo_lagging(before: &[char], tracked: &[char], token_chars: usize) -> bool {
    if before.is_empty() || tracked.is_empty() || token_chars == 0 || tracked.len() < token_chars {
        return false;
    }
    // 落后不超过一个词；整行都没回显时没有锚点，不冒险。
    let max_lag = token_chars.min(tracked.len());
    (1..=max_lag)
        .any(|lag| lag < tracked.len() && before.ends_with(&tracked[..tracked.len() - lag]))
}

/// 把候选命令接到已有前缀后面：前缀的后缀已经写过命令开头的若干词时（如
/// `sudo apt ` + `sudo apt install …`）不重复，否则会拼出 `sudo apt sudo apt …`。
pub fn merged_command(prefix: &str, command: &str) -> String {
    let prefix_words: Vec<&str> = prefix.split_whitespace().collect();
    let words: Vec<&str> = command.split_whitespace().collect();
    let max = prefix_words.len().min(words.len());
    for common in (1..=max).rev() {
        if prefix_words[prefix_words.len() - common..] == words[..common] {
            let rest = words[common..].join(" ");
            return if rest.is_empty() {
                command.trim().to_string()
            } else {
                rest
            };
        }
    }
    command.trim().to_string()
}

#[cfg(test)]
mod tests;
