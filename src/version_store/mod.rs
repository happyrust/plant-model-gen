//! （specs/027 过渡期）SurrealDB 版本化读副本残留。
//!
//! DuckLake 权威链已随 ADR-0007 退役删除；本模块仅剩 replica 读副本类型，
//! 供 `generation_read::surreal` 编译过渡使用，T006（主表直读适配器）落地后
//! 随 `generation_replica_*` 五表一并删除。

pub mod replica;

pub use replica::{ReplicaApplyBatch, ReplicaSnapshotBinding, SurrealReplicaStore};
