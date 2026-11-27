//! 零外部依赖的纯 Rust 轻量 BM25 文本检索引擎（支持中英文混合分词）。
//!
//! 设计原理（Why）：
//! - 当用户的主机长期记忆（`hosts/*.md`）和自定义运维 Runbook 增多时，按需检索最相关片段比全量塞入上下文更省 Token；
//! - 采用标准 Okapi BM25（`k1 = 1.5, b = 0.75`）+ ASCII 词元与 CJK 单字/二元 bigram 混合分词，
//!   无需引入外部向量库或嵌入模型即可实现毫秒级本地知识检索。

use std::collections::{HashMap, HashSet};

const K1: f32 = 1.5;
const B: f32 = 0.75;

/// 参与 BM25 索引的单个知识文档。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bm25Document {
    pub source: String,
    pub title: String,
    pub content: String,
}

/// BM25 检索命中结果。
#[derive(Clone, Debug, PartialEq)]
pub struct Bm25Hit {
    pub source: String,
    pub title: String,
    pub score: f32,
    pub snippet: String,
}

/// 内存态 BM25 倒排索引。
pub struct Bm25Index {
    docs: Vec<IndexedDoc>,
    doc_freq: HashMap<String, usize>,
    avg_len: f32,
}

struct IndexedDoc {
    source: String,
    title: String,
    content: String,
    tf: HashMap<String, usize>,
    len: usize,
}

impl Bm25Index {
    /// 基于文档切片构建 BM25 索引。
    pub fn build(documents: Vec<Bm25Document>) -> Self {
        let mut docs = Vec::with_capacity(documents.len());
        let mut doc_freq: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0usize;

        for doc in documents {
            let combined = format!("{}\n{}", doc.title, doc.content);
            let tokens = tokenize(&combined);
            let len = tokens.len();
            total_len = total_len.saturating_add(len);

            let mut tf: HashMap<String, usize> = HashMap::new();
            let mut seen = HashSet::new();
            for tok in tokens {
                *tf.entry(tok.clone()).or_insert(0) += 1;
                if seen.insert(tok.clone()) {
                    *doc_freq.entry(tok).or_insert(0) += 1;
                }
            }

            docs.push(IndexedDoc {
                source: doc.source,
                title: doc.title,
                content: doc.content,
                tf,
                len,
            });
        }

        let avg_len = if docs.is_empty() {
            1.0
        } else {
            (total_len as f32) / (docs.len() as f32)
        };
        Self {
            docs,
            doc_freq,
            avg_len: avg_len.max(1.0),
        }
    }

    /// 检索与查询最相关的 Top-K 文档片段。
    pub fn search(&self, query: &str, top_k: usize) -> Vec<Bm25Hit> {
        if self.docs.is_empty() || top_k == 0 {
            return Vec::new();
        }
        let q_tokens = tokenize(query);
        if q_tokens.is_empty() {
            return Vec::new();
        }

        let n = self.docs.len() as f32;
        let mut hits = Vec::new();

        for doc in &self.docs {
            if doc.len == 0 {
                continue;
            }
            let mut score = 0.0f32;
            for term in &q_tokens {
                let Some(&tf_raw) = doc.tf.get(term) else {
                    continue;
                };
                let df = *self.doc_freq.get(term).unwrap_or(&0) as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln().max(0.0);
                let tf = tf_raw as f32;
                let norm = 1.0 - B + B * (doc.len as f32) / self.avg_len;
                score += idf * (tf * (K1 + 1.0)) / (tf + K1 * norm);
            }
            if score > 0.0 {
                hits.push(Bm25Hit {
                    source: doc.source.clone(),
                    title: doc.title.clone(),
                    score,
                    snippet: extract_snippet(&doc.content, 400),
                });
            }
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(top_k);
        hits
    }
}

/// 中英文混合分词：ASCII 词元转小写，CJK 字符同时产出单字与相邻二元 bigram。
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii_buf = String::new();
    let mut prev_cjk: Option<char> = None;

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            ascii_buf.push(ch.to_ascii_lowercase());
            prev_cjk = None;
        } else {
            if !ascii_buf.is_empty() {
                tokens.push(std::mem::take(&mut ascii_buf));
            }
            if is_cjk(ch) {
                tokens.push(ch.to_string());
                if let Some(p) = prev_cjk {
                    let mut bi = String::with_capacity(8);
                    bi.push(p);
                    bi.push(ch);
                    tokens.push(bi);
                }
                prev_cjk = Some(ch);
            } else {
                prev_cjk = None;
            }
        }
    }
    if !ascii_buf.is_empty() {
        tokens.push(ascii_buf);
    }
    tokens
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

fn extract_snippet(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    let count = trimmed.chars().count();
    if count <= max_chars {
        trimmed.to_owned()
    } else {
        let head: String = trimmed.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::{Bm25Document, Bm25Index};

    #[test]
    fn ranks_relevant_chinese_and_english_docs_first() {
        let index = Bm25Index::build(vec![
            Bm25Document {
                source: "hosts/db.md".to_owned(),
                title: "MySQL 主库运维笔记".to_owned(),
                content: "InnoDB 缓冲池配置与慢查询日志路径 /var/log/mysql/slow.log".to_owned(),
            },
            Bm25Document {
                source: "hosts/api.md".to_owned(),
                title: "Nginx 网关配置".to_owned(),
                content: "upstream 502 超时排查，检查 php-fpm 与 proxy_read_timeout".to_owned(),
            },
        ]);

        let hits = index.search("nginx 502 超时", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "hosts/api.md");
        assert!(index.search("", 5).is_empty());
    }
}
