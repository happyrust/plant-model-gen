# 022 ↔ 023 共存说明

> **2026-07-20 废止**：023 DuckLake 交付单元版本链已由 `specs/024-unified-rocksdb-versioning` 决议整体退役（`docs/adr/0001`），本共存说明仅作历史记录保留。

**Date**: 2026-07-16

| 维度 | specs/022 PE/ATT versioned | specs/023 DuckLake unit version |
|---|---|---|
| 存什么 | PE/ATT **源数据行**历史 | **导出**后的最小交付单元版本与成员 |
| 引擎 | SUL_DB RocksDB `versioned` + `sesno_version_anchor` | DuckLake catalog（`unit_versions_v2` 等） |
| 版本身份 | `dbnum + sesno` → 存储时间戳 → `SELECT … VERSION $t` | `(dbnum, unit_refno, sesno)`；单元 sesno = `max(member_sesno)` |
| `release_id` | 不涉及 | **非**真相源；至多 export-batch 别名（如 `db{N}-s{M}`） |
| 保留期 | retention（默认 90d） | 随交付存档长期保留 |
| CLI | 计划：`model-version history *`（M3） | 已有：`unit-v2-*` / `unit-diff` sesno 模式 |

两路径**不要混用**：022 不写 DuckLake 单元表；023 不回答 PE/ATT 行级时间旅行。
