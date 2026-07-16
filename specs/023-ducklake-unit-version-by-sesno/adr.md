# ADR: DuckLake 最小交付单元版本键改为 (dbnum, refno, sesno)

**Status**: Accepted  
**Date**: 2026-07-16  
**Spec**: `specs/023-ducklake-unit-version-by-sesno/`

## Context

现有 `model_version` DuckLake catalog 以 `release_id` 作为几乎所有表的关联键（`model_releases`、`component_snapshots`、`delivery_unit_memberships`、`unit_versions` 等）。这与 PDMS 业务版本语义脱节：设计会话以 **sesno** 推进，元素以 **refno** 标识。

用户明确要求：

1. DuckLake **只存 export 的最小交付单元版本数据**，不承担 PE/ATT 行级历史。
2. **不应**再以人造 `release_id` 作为版本真相源。
3. 最小交付单元按 **最新 sesno** 表达，身份为 **`refno + sesno`**（实现上再加 `dbnum`）。

PE/ATT 源数据历史由 `specs/022-versioned-pe-att-storage`（SUL_DB RocksDB `versioned` + `sesno_version_anchor`）负责，与本 ADR 正交。

## Decision

| 项 | 决定 |
|---|---|
| 版本身份主键 | `(dbnum, refno, sesno)` |
| 单元 sesno | 导出索引时成员组件的 `max(member_sesno)` |
| 第一刀范围 | 先改 DuckLake **catalog 表主键与读写**；物理包目录 / CLI 文案随后 |
| `release_id` | 停止作为真相源；过渡期可读兼容别名，禁止新写依赖 |
| 血缘 | 同 `refno` 上按 `sesno` 单调序对比；废弃 `parent_release_id` 图作为主模型 |
| 幂等 | 同主键 + 同 `content_hash` → no-op；同主键不同 hash → 拒绝（或显式 force） |

## Consequences

### Positive

- 版本号与 PDMS sesno / 022 锚点语义对齐，diff 入参自然变为 `--refno --from-sesno --to-sesno`。
- 去掉 release 图后，交付单元版本可独立推进，不必为每次 export 批发明全局 ID。

### Negative / Risks

- 已有仅含 `release_id`、无 sesno 元数据的 DuckLake 行 **无法无损迁移** → 弃旧重灌。
- `history publish` 等路径已埋 `from_sesno/to_sesno` 的可映射；其余需抽样确认。
- schema 迁移与调用面（`ducklake_store` / `model_release` / `cli` / `types`）改动集中，需分阶段 dual-read。

### Out of scope

- SUL_DB PE/ATT `VERSION` 查询封装（022 M3）。
- HTTP API。
- 用 DuckLake 替换源属性历史。

## Follow-up

见同目录 `tasks.md`、`ddl-draft.sql`、`inventory.md`、`coexistence-022.md`。  
Planning 目录 `.planning/2026-06-17-ducklake-valv-version-diff/GOAL.md` 已标注 `release_id` 图过时。
