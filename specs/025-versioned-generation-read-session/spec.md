# Feature Specification: 版本化模型生成读取双后端

**Feature Branch**: `feat/022-versioned-pe-att-storage`  
**Created**: 2026-07-20  
**Status**: Accepted for implementation  
**Upstream**: `docs/adr/0002-ducklake-authoritative-data-versioning.md`, `docs/adr/0003-versioned-generation-read-session.md`

## User Scenarios

### US1 - 同一模型生成可选择两个读取后端

模型生成操作者可显式选择 `surreal`、`ducklake` 或 `compare`。相同输入版本清单与生成契约在两个后端得到相同领域查询结果和模型语义结果；任何 backend 都不能泄漏 SQL 到生成算法。

### US2 - 运行期间数据版本保持固定

生成开始时系统解析一个权威 DuckLake snapshot 及其 `dbnum -> sesno` 清单。即使运行期间有新版本提交，本次运行的 PE、ATT、层级、CATA 和 transform 查询仍读取原 snapshot。

### US3 - 副本落后时拒绝读取

选择 SurrealDB backend 时，系统必须确认连续副本水位和 snapshot binding。副本未覆盖目标 snapshot 时生成失败，不读取 latest，也不切到 DuckLake。

### US4 - 元件库依赖按共享规则解析

给定设计侧 CATA 引用种子，两个 backend 只批量返回节点、owner、children 和属性引用边；共享 Rust `CatalogResolver` 以确定性 BFS 计算同一闭包。

## Functional Requirements

- **FR-001**: `VersionedReadSession` MUST 绑定不可变输入版本清单、权威 snapshot 和单一 backend。
- **FR-002**: 生成代码 MUST 只依赖 `ElementRead`、`AttributeRead`、`HierarchyRead`、`CatalogGraphRead`、`TransformRead` 领域接口。
- **FR-003**: 接口 MUST batch-first；热路径不得为每个 refno 发起一次 backend 查询。
- **FR-004**: DTO MUST 使用 `RefnoEnum` 且不得包含 Surreal record-id、SQL 或 Arrow 类型。
- **FR-005**: 必需数据缺失 MUST 返回结构化错误；诊断结果不得被发布为模型版本。
- **FR-006**: children、BFS、CATA 闭包和规范化哈希 MUST 确定性一致。
- **FR-007**: DuckLake MUST 是唯一权威版本库；SurrealDB MUST 只复制已发布 snapshot。
- **FR-008**: SurrealDB snapshot 复制 MUST 原子写入数据和 binding；部分复制不得推进水位。
- **FR-009**: 首次迁移 MUST 只建立明确的 `history_start_snapshot`，不得伪造旧历史。
- **FR-010**: `ModelWriterBackend` MUST 与读取 backend 独立。
- **FR-011**: 正式双后端路径 MUST 使用内存 `GenerationArtifacts`，不得从模型表补读当前运行中间产物。
- **FR-012**: 配置 MUST 显式选择 `surreal|ducklake|compare`，不得提供自动 fallback。

## Success Criteria

- **SC-001**: 同 fixture 的两个 adapter 在全部能力接口上具有完全相同的规范化结果、missing 集与顺序。
- **SC-002**: 同 snapshot 的两次端到端生成具有相同 `GenerationArtifacts` 和最终模型语义哈希。
- **SC-003**: 副本落后、binding 缺失、snapshot 缺失、payload 损坏均明确失败且不产生模型发布记录。
- **SC-004**: 生成算法目录中无新增全局数据库调用或 backend SQL。
- **SC-005**: release 基准无 N+1，端到端耗时相对 Surreal 基线劣化不超过 10%。

## Non-goals

- 模型版本发布协议与模型表物理版本设计。
- 任意跨 dbnum 历史版本拼接。
- PostgreSQL DuckLake catalog、对象存储和切换前完整历史迁移。
