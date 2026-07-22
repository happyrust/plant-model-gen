---
status: superseded by ADR-0008
date: 2026-07-20
depends_on: ADR-0002
---

# 模型生成通过固定版本读取会话访问双后端

模型生成输入统一抽象为 `VersionedReadSession`。一次会话绑定一份输入版本清单、一个真实存在的 DuckLake 全局 `snapshot_id` 和单一读取后端；会话内不允许按 dbnum 或查询失败在 DuckLake 与 SurrealDB 之间混读。版本覆盖不足、必需元素/属性/引用/变换缺失或副本水位落后时必须失败关闭。

DuckLake adapter 使用 DuckDB SQL 查询权威 snapshot；SurrealDB adapter 只查询已经原子复制并建立 `snapshot_id -> replica_version_time` 绑定的版本化读副本。两者只实现批量事实读取。层级遍历、CATA 引用闭包、排序、缺失语义和规范化哈希由共享 Rust 领域代码实现，生成算法不得包含 SQL、Surreal record-id 或全局数据库调用。

首次从现有站点切换时，只把当前已提交完整状态引导为第一个权威 snapshot，并记录 `history_start_snapshot`。更早 SurrealDB 历史保持 legacy 只读，不伪造为 DuckLake snapshot。首版不支持任意 `dbnum/sesno` 历史组合。

模型产物写入仍由独立 `ModelWriterBackend` 负责；读取 backend 不进入模型版本身份。模型版本发布协议及物理表设计不在本决策范围内。

被否决的方案包括：通用 SQL/QuerySpec 暴露给生成层、逐 refno 点查、会话内自动 fallback、DuckDB 与 SurrealQL 分别实现 CATA 递归语义，以及把 DuckLake 同时作为模型写入 backend。
