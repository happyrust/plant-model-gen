# [已归档] SurrealKV Model Write Separation Implementation Plan

> 状态：2026-03-02 起，本计划归档，不再作为现行实现指南。
> 更新（2026-07-17）：SurrealKV/MODEL_KV 双库分离机制已从 rs-core 与本仓库整体移除，
> 模型数据与 PE/属性固定写同一 SurrealDB（SUL_DB），`model_primary_db()` 仅作兼容别名。

说明：
- 历史方案中的写入模式切换设计已下线。
- 本文保留为归档占位，避免继续误用旧流程。
