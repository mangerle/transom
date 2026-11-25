//! 源码结构守卫：把架构约定固化成对源码的静态检查。
//!
//! # 设计原理（Why）
//! 这些检查与主题无关，放在 `theme` 单元测试里会让主题模块承担全 crate 的
//! 架构守卫职责；移到集成测试后，主题测试只保留主题契约，源码扫描集中在一处。
//!
//! # 代价与局限
//! 文本扫描只能防住“写法恰好被匹配”的回归，不能替代类型约束。字段可见性
//! 与实体边界最终应由类型系统和模块私有性保证；本文件是迁移期的防线。

mod api;
mod common;
mod container_git;
mod entities;
mod i18n;
mod icons_a11y;
mod tab_model;
mod tokens;
mod workspace;
