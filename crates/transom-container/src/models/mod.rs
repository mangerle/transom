//! Docker/Podman REST API 的线格式结构体与到领域模型的转换。
//!
//! 原则（Why）：外部 JSON 只在本模块出现一次，立即收敛为
//! `transom-core` 的强类型；引擎字段增删或大小写差异不会扩散到控制器与界面。

use serde::Deserialize;
use serde::de::Deserializer;
use std::collections::HashMap;

mod containers;
mod exec;
mod images;
mod networks;
mod stats;
mod system;
mod volumes;

pub use self::containers::*;
pub use self::exec::*;
pub use self::images::*;
pub use self::networks::*;
pub use self::stats::*;
pub use self::system::*;
pub use self::volumes::*;

/// 把线格式里的 `null` 收敛成空集合。
///
/// 设计与原因（Why）：引擎对「无标签」的数据卷返回 `"Labels": null` 而不是 `{}`，
/// 而 `#[serde(default)]` 只兜底「键缺失」，键存在但为 null 会直接报类型错误。
/// 这类字段只用于展示，空集合就是合法语义，不值得为它新增错误分支。
fn null_as_empty<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// 取非空白文本；键存在但为空串时按缺失处理。
fn non_blank(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// RFC3339 时间转 Unix 秒；解析失败返回 `None`。
pub fn parse_rfc3339_secs(value: Option<&str>) -> Option<i64> {
    let value = value?;
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|datetime| datetime.timestamp())
}

/// 把引擎的 map 字段摊平成按 key 排序的列表：界面渲染与差分都要求稳定顺序。
fn sorted_pairs(map: HashMap<String, String>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = map.into_iter().collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    pairs
}

#[cfg(test)]
mod tests;
