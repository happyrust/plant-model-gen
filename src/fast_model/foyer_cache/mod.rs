//! [foyer-removal] 桩模块：foyer_cache 已移除，此处仅提供编译兼容。
//!
//! 原计划第 7/8 项（基于 foyer_cache 的缓存持久化）已作废。
//! 若未来需要缓存层，应基于新架构重新设计（不再基于已移除的 foyer）。

#[deprecated(note = "foyer_cache 已移除，此模块为桩。若需缓存层，请基于新架构重新设计")]
pub mod cata_resolve_cache;
