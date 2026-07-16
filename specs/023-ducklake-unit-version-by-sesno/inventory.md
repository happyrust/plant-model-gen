# Inventory: release_id 读写面（Phase A3/A4）

**Date**: 2026-07-16（刷新）  
**Scope**: `src/version_management/`（必扫文件 + 邻近编排）  
**Rule**: DuckLake 版本身份真相为 `(dbnum, refno, sesno)`；`release_id` 至多作批次元数据 / 过渡别名。

## A3 — Hit counts（`rg -c release_id`，约数）

| File | Hits | Role |
|---|---|---|
| `ducklake_store.rs` | ~368 | Schema DDL、CRUD、index、diff；含 `*_v2` 与 `sync_release_units_into_v2` |
| `model_release.rs` | ~84 | register / publish-history / reconcile 编排；已接 `export_sesno` + sync |
| `cli.rs` | ~76 | `model-version` 参数与输出；`unit-diff` 已可走 sesno |
| `types.rs` | ~35 | Request/Response；含 `export_sesno`、`legacy_batch_id_*` |
| `history_replay_plan.rs` | ~21 | 历史回放计划仍绑定 release |
| `release_package.rs` | ~16 | 遗留物化 `releases/<id>/parquet/<dbnum>`；E1 已加 `units/.../sesno-N` |
| `release_state_machine.rs` | ~11 | 状态迁移 / 事件按 release（E2） |
| `physical_baseline_snapshot.rs` | ~2 | 基线快照元数据 |

### 必扫 API 面（按职责）

#### 仍以 `release_id` 为关联键（legacy catalog）

| API / 表 | 读写 | 023 处置 |
|---|---|---|
| `model_releases` + `register_release` / `get_release` / `list_releases` | RW | 过渡保留为**导出批次**壳；非单元版本身份 |
| `model_release_status_events` / `update_release_status` / `release_events` | RW | E2：事件挂批次或挂 unit_versions_v2 |
| `model_release_edges` / `parent_release_id` | RW | 废弃为血缘主模型；diff 改 sesno 序 |
| `model_release_files` / `model_release_metadata` | RW | 批次附件；可随批次壳保留 |
| `component_snapshots` / `index_release_components` | RW | 第二波：`component_snapshots_v2` |
| `delivery_unit_memberships` / `unit_versions` / `index_release_units` | RW | **DEPRECATED** 写路径；读经 C3 dual-read |
| `model_release_mesh_assets` / `index_release_mesh_assets` | RW | 第二 PR；暂不改主键 |
| `diff_releases` / `diff_units` / `impact` | R | CLI 告警；单元 diff 优先 `unit-diff` sesno / `unit-v2-diff` |
| `release_state_machine` reconcile | RW | E2 |

#### 已按 `(dbnum, refno, sesno)`（v2）

| API | 备注 |
|---|---|
| `upsert_unit_version_v2` / `get` / `list` / `diff_unit_versions_v2` | 真相读写；get/list 含 C3 legacy 回退 |
| `write_unit_version_with_members_v2` | `unit_sesno = max(member_sesno)` |
| `sync_release_units_into_v2` | 遗留表 → v2 投影；填 `package_relpath` |
| `unit_version_package_relpath` / `materialize_unit_version_package_dir` | E1：`units/<dbnum>/<refno>/sesno-<N>` |
| CLI `unit-v2-*` / `unit-get` / `unit-list` / `unit-diff` sesno 模式 | D3 |

#### 调用方已收敛（部分 D）

| 路径 | 行为 |
|---|---|
| `register` + `--sesno` / `export_sesno` | 可省略 `--release-id` → `db{N}-s{M}`；index 后 sync v2 |
| `publish-history` | `to_sesno` → `export_sesno` + sync |
| HTTP register / incremental handoff | `sesno` / `to_sesno` → `export_sesno` |

## A4 — Sesno 可映射性与弃旧策略

### 可映射（可投影到 v2，不必丢业务版本）

1. **`extra_metadata.history_publish.{from_sesno,to_sesno}`**（publish-history 写入）→ 单元版取 **`to_sesno`**。  
2. **顶层 / `incremental.{from_sesno,to_sesno}`**（`ducklake_store` ~L5393 `json_scalar_at`）→ 同上。  
3. **`release_id` 形如 `db{dbnum}-s{sesno}`**（`parse_legacy_batch_id`）→ 直接解析；C3 dual-read / `unit-diff` 映射已用。  
4. **register 带 `export_sesno`**（新写入）→ sync 进 v2。

### 不可无损映射 → 弃旧重灌

- 纯手工 `register`、无 `export_sesno`、metadata 无 sesno、且 `release_id` 不可解析为 `dbN-sM` 的旧 catalog 行。  
- 仅有 `parent_release_id` 图、无 sesno 的对比基线。  
- **策略**：不写复杂猜测迁移；对该项目 **弃旧 DuckLake unit 表 / 重跑 index + sync**（或直接 `write_unit_version_with_members_v2`）。`model_releases` 批次壳可保留作审计，但不作为版本真相。

### 抽样结论（代码路径，非生产库统计）

| 来源类 | 可映射比例（代码路径） | 动作 |
|---|---|---|
| publish-history / incremental handoff | **高**（字段强制存在） | sync / dual-read |
| `db{N}-s{M}` 别名 register | **高**（可解析） | parse → v2 |
| 旧任意字符串 release_id + 无 metadata | **低 / 零** | 弃旧重灌 |
| mesh / component legacy | 未纳入第一刀 | 第二 PR |

> 生产 catalog 若需实测比例：对 `model_release_metadata` / sidecar JSON 跑 `from_sesno|to_sesno|db*-s*` 计数即可；本仓库无固定样例库，故 A4 以路径分类为准。

## Tables still keyed by release_id（遗留）

- `model_releases`, `model_release_status_events`, `model_release_edges`
- `model_release_files`, `model_release_mesh_assets`, `model_release_mesh_asset_index_runs`
- `model_release_metadata`
- `component_snapshots`, `component_index_runs`
- `delivery_unit_memberships`, `unit_versions`, `unit_index_runs`

## Tables keyed by (dbnum, refno, sesno)（v2）

- `unit_versions_v2`, `unit_memberships_v2`
- `component_snapshots_v2`（schema 已建；写路径第二波）
- `unit_index_runs_v2`, `component_index_runs_v2`
- `export_batches`（批次元数据，非版本身份）

## Open / 后续

- E2：状态机与事件与 `release_id` 解耦（或显式降级为 batch 事件）。  
- E3：022 共存说明；planning 中 release_id 图标记过时。  
- E4：删除 C3 dual-read 与 `legacy_release_id` 列。  
- mesh / component 主键 cutover：单独 PR。
