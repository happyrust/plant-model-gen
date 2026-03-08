# 在 Worktree 中落地 rkyv + SQLite AABB 方案（3 PR 决策完成版）

## 简要摘要
本计划按你已确认的策略执行：**新建独立 worktree + 同步推进 rkyv 缓冲与 SQLite AABB + 分 3 个 PR 交付**。  
目标是把模型生成从“批次实时写 Surreal 主体表”改为“Phase A 写 rkyv + 实时写 neg/ngmr + 写 SQLite AABB，Phase B 批量回写 Surreal 主体表，Phase C 再做布尔”。

---

## Implementation Plan（中文）

### 0) Worktree 启动与基线校验
- 使用技能 `using-git-worktrees` 的既定流程。
- 已确认目录存在且被忽略：`.worktrees`（来自 `/Volumes/DPC/work/plant-code/plant-model-gen/.gitignore`）。
- 创建独立工作区与分支（分支前缀遵循 `codex/`）：
  - `git -C /Volumes/DPC/work/plant-code/plant-model-gen worktree add /Volumes/DPC/work/plant-code/plant-model-gen/.worktrees/rkyv-buffered-aabb -b codex/rkyv-buffered-aabb`
- 进入 worktree 后先跑只读基线：
  - `cargo check -q`
  - `cargo test -q --no-run`
- 若基线失败：记录失败清单，作为“既有问题”，不阻断后续改造，但在 PR 描述中标注。

### 1) PR1：基础设施与配置扩展（不改主流程行为）
**目标**：把“可复用积木”补齐，默认不开启新模式。

- 新增模块：`/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/gen_model/rkyv_buffer.rs`
  - `GenerationBatchSnapshot`（含 `shape_instances`, `mesh_results`, `batch_index`, `run_id`）
  - `CrossBatchNegGeoIndex`（跨批次 `neg_geo_by_carrier` 与 `cata_cross_neg_geo_map`）
  - `write_batch_atomic` / `read_batch` / `list_batches_sorted`
  - 文件落盘目录：`output/<project>/rkyv_batches/<run_id>/`
- 新增可序列化结构：
  - 在 `/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/gen_model/mesh_generate.rs` 增加 `MeshResultSerializable`（避免直接改动现有 `MeshResult` 业务语义）
- 抽取统一 ID 规则（关键）：
  - 在 `/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/gen_model/pdms_inst.rs` 抽取 `build_geo_relate_id(...)`，所有 geo_relate ID 生成统一走这一函数。
- 配置扩展（默认 false）：
  - `/Volumes/DPC/work/plant-code/plant-model-gen/src/options.rs` 增加 `rkyv_buffered_write: bool`
  - 同步补齐：结构体字段、`From<DbOption>` 默认值、TOML 手工解析路径。
- SQLite 索引扩展（schema + API，先可用后接线）：
  - `/Volumes/DPC/work/plant-code/plant-model-gen/src/sqlite_index.rs`
  - 新增表：`geo_aabb`（KV）与 `inst_aabb`（RTree）
  - 新增方法：`insert_geo_aabbs`、`get_geo_aabb`、`insert_inst_aabbs`、`query_inst_aabb_intersect`
- mesh 文件扫描器（仅工具函数）：
  - 新增 `scan_mesh_dir_for_geo_hashes`，支持 `"{hash}_{LOD}.glb"` 与 legacy `"{hash}.glb"`。
  - 正则规则固定为：`^(\d+)(?:_L\d+)?$`，**不会匹配**布尔结果 `refno` 形式（含额外下划线段）。

### 2) PR2：Phase A 接线（rkyv 模式生成期）
**目标**：开启 `rkyv_buffered_write=true` 时，生成期不写 Surreal 主体表。

- 改造入口：`/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/gen_model/orchestrator.rs`
  - 在 `insert_handle` 增加 `rkyv_buffered_write` 分支。
  - 保留 mesh 内联生成与 `BooleanTaskAccumulator`。
  - mesh 去重预热在新模式下改为目录扫描，不调用 `query_existing_meshed_inst_geo_ids()`。
- neg/ngmr 实时写入改造：
  - 在 `/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/gen_model/pdms_inst.rs` 新增 `write_neg_ngmr_relates_realtime_with_index(...)`
  - 使用 `CrossBatchNegGeoIndex` + `pending` 队列，**去掉 missing_carriers 的 DB 回查路径（仅在新模式）**
  - geo_relate 引用 ID 必须来自 PR1 的统一 `build_geo_relate_id(...)`
- SQLite AABB 写入接线：
  - 每批 mesh 完成后写 `geo_aabb`
  - 每批实例完成后写 `inst_aabb`（refno -> merged AABB）
- rkyv 批次持久化：
  - 每批 `ShapeInstancesData + mesh_results` 原子写入 `batch_000001.rkyv` 等文件。
- 安全门：
  - `rkyv_buffered_write=true` 与 `defer_db_write=true` 互斥；若同时开启，启动即报配置错误并退出。

### 3) PR3：Phase B 回写 + 兼容清理
**目标**：生成后批量回写 Surreal 主体表，并完成依赖方兼容。

- 在 `orchestrator` 中增加 Phase B：
  - 读取 `rkyv_batches/<run_id>`，按 `batch_index` 顺序回放。
  - 每批调用 `bulk_write_batch_to_db(...)`。
- `bulk_write_batch_to_db` 设计：
  - 基于当前 `save_instance_data_optimize` 抽出新函数，写入：
    - `inst_geo`, `geo_relate`, `inst_info`, `inst_relate`, `trans`, `vec3`
  - 跳过：
    - `neg_relate`, `ngmr_relate`（已在 Phase A）
    - `aabb`, `inst_relate_aabb`（迁移到 SQLite）
- 布尔时序修正：
  - 仅在 Phase B 成功后执行 `run_bool_worker_from_tasks(...)`
  - `rkyv_buffered_write=true` 时不调用 `reconcile_missing_neg_relate`
- 消费方兼容改造（防回归）：
  - `/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/room_model.rs`
  - `/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/export_model/export_common.rs`
  - `/Volumes/DPC/work/plant-code/plant-model-gen/src/fast_model/cal_model/equip_model.rs`
  - 读取策略统一：**先读 SQLite `inst_aabb`，读不到再 fallback Surreal `inst_relate_aabb`**（过渡期兼容）

---

## 对外接口/类型/配置变更（Public API/Interface）
- 配置新增：
  - `DbOptionExt.rkyv_buffered_write: bool`（默认 `false`）
- 新增模块与类型：
  - `rkyv_buffer.rs`
  - `GenerationBatchSnapshot`
  - `CrossBatchNegGeoIndex`
  - `MeshResultSerializable`
- SQLite 索引 API 扩展：
  - `insert_geo_aabbs/get_geo_aabb/insert_inst_aabbs/query_inst_aabb_intersect`
- 行为开关语义：
  - `rkyv_buffered_write=true`：启用新管线
  - `defer_db_write=true`：保持旧的 SQL 文件导出模式
  - 二者互斥

---

## Test Cases and Scenarios
- 单元测试
  - `ShapeInstancesData` 的 rkyv roundtrip 包含 `neg_relate_map`（验证 `#[serde(skip)]` 不影响 rkyv）
  - `build_geo_relate_id` 稳定性测试（同输入同 ID）
  - `scan_mesh_dir_for_geo_hashes` 对 `123_L2.glb`、`123.glb`、`17496_106028_L2.glb` 的判定
  - `SqliteAabbIndex` 的 `geo_aabb` upsert/query 与 `inst_aabb` intersect
- 集成回归
  - 同一数据集下，`rkyv_buffered_write=false` vs `true` 对比：
    - `inst_geo/geo_relate/inst_info/inst_relate/neg_relate/ngmr_relate` 数量一致
    - 抽样记录内容一致（含关键字段）
  - 布尔阶段回归：
    - `inst_relate_bool/inst_relate_cata_bool` 成功率不低于基线
  - 房间/导出/设备链路回归：
    - `room_model`、`export_common`、`equip_model` 查询结果可用
- 失败场景
  - Phase A 完成、Phase B 失败时，重新执行同配置不应产生脏重复（依赖 `INSERT IGNORE` + 稳定 ID）

---

## Task List（中文）
- T1：创建 worktree 与基线校验，记录环境基线报告。
- T2：完成 PR1（rkyv 基础设施、配置、统一 ID、SQLite schema/API、扫描器）。
- T3：完成 PR2（Phase A 接线：rkyv 写批次 + neg/ngmr 实时写 + SQLite AABB 写入）。
- T4：完成 PR3（Phase B 回放回写 + 布尔时序 + 兼容消费方改造）。
- T5：执行完整回归矩阵并产出对比报告（计数 + 抽样内容 + 性能指标）。
- T6：提交 3 个 PR，附迁移说明与回滚方案。

---

## Thought（中文）
- 这次不再做“只改一半”的折中：你已选择 rkyv + SQLite AABB 同步推进，我按此定案。
- 最大风险点已前置锁定为三件事：**geo_relate ID 一致性、mesh 文件名解析、旧消费方兼容读取**。
- 只要按 3 PR 顺序推进并在每个 PR 末做回归闸门，这条路径可落地且可回滚。

---

## 关键假设与默认值（Assumptions & Defaults）
- 基线分支：`main`。
- worktree 位置：`/Volumes/DPC/work/plant-code/plant-model-gen/.worktrees/rkyv-buffered-aabb`。
- 工作分支名：`codex/rkyv-buffered-aabb`。
- `rkyv_buffered_write` 默认关闭，不影响现网默认行为。
- 过渡期保留 Surreal `inst_relate_aabb` 读取 fallback，待全链路验证稳定后再考虑彻底移除。
