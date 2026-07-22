---
status: superseded by ADR-0007
date: 2026-07-20
supersedes: ADR-0001
---

# DuckLake 作为数据版本权威，SurrealDB 作为版本化读副本

我们决定撤销 ADR-0001 中“RocksDB versioned 是唯一版本机制、DuckLake 整体退役”的方向。数据版本以 `(dbnum, sesno)` 作为业务身份，并唯一映射到 DuckLake 原生 `snapshot_id`；DuckLake 是唯一有权发布版本和推进已提交水位的权威版本库。SurrealDB 保留完整历史查询能力，但定位为版本化读副本：按相同增量顺序复制已发布版本，可以落后但不得领先，也不能自行发布版本。

DuckLake 提交成功而 SurrealDB 复制失败时，权威数据版本仍然有效；系统记录副本水位并幂等追赶，SurrealDB 在追平前必须拒绝读取尚未复制的版本，不能静默回退。模型生成代码不直接包含 DuckDB SQL、SurrealQL 或全局数据库调用，只依赖领域查询 trait；两种存储的查询语句封装在各自适配器内。

本 ADR 先固定数据版本存储与双后端关系。模型版本将以“输入版本清单 + 生成契约”作为身份，但其发布协议和物理表设计在后续决策中单独记录。
