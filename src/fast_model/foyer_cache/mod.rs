//! foyer cache（instance_cache）专用能力集合
//!
//! 本模块用于集中放置 **cache-only（不访问 SurrealDB）** 的模型生成/查询/worker 入口。
//! 其目标是：
//! - 将散落在各处的 `InstanceCacheManager` 使用点收敛到单一“专区”；
//! - 通过统一的运行时上下文（`FoyerCacheContext`）减少重复初始化与路径解析；
//! - 对外提供清晰的 API 与文档注释，便于 orchestrator 编排与后续扩展。
//!
//! 注意：为保持兼容性，原有入口函数可能仍保留在旧模块中，但会转发到此处实现。

pub mod context;
pub mod geos;
pub mod mesh;
pub mod boolean;
pub mod query;
pub mod ptset;

pub use context::FoyerCacheContext;
