# 房间计算流程分析（Linear 文档）

> 本文档可直接复制到 Linear 的 Project Document 或 Issue 中。

---

## 概述

房间计算用于确定**构件（EQUI、BRAN、PIPE 等）与房间（FRMW/SBFR）的空间归属关系**，并将结果写入 SurrealDB 的 `room_relate` 和 `room_panel_relate` 表，供材料表、空间查询、fn::room_code 等使用。

---

## 数据模型

| 表名 | 方向 | 含义 |
|------|------|------|
| **room_panel_relate** | FRMW/SBFR → PANE | 房间包含哪些面板 |
| **room_relate** | PANE → 构件 | 面板内包含哪些构件 |

---

## 主流程

1. 刷新 SQLite 空间索引 ← inst_relate_aabb
2. 构建房间-面板映射 ← FRMW/SBFR + pe_owner + room_keyword
3. 写入 room_panel_relate（房间 → 面板）
4. 预生成面板几何缓存（可选）
5. 对每个房间的每个面板：粗算（SQLite RTree）→ 细算（27 关键点投票）→ 写入 room_relate

---

## 关键模块

- **plant-model-gen**：`room_model.rs`（主计算）、`room_worker.rs`（后台任务）
- **rs-core**：`room/`（查询、算法）、`spatial/sqlite.rs`（RTree）
- **配置**：`room_keyword`、`gen_spatial_tree`、`AIOS_ROOM_USE_CACHE`

---

## 完整文档

详见仓库 `plant-model-gen/docs/房间计算流程分析.md`
