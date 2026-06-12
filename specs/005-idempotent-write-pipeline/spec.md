# Feature Specification: 模型写入管线幂等化（统一 INSERT IGNORE 与部署零旧数据检查）

## User Need

站点首次部署是空库写入，不存在旧数据，但旧写入管线在多处「写入前先检查/删除旧数据」：`inst_relate_aabb` 逐 chunk `DELETE + INSERT` 成对替换、`inst_relate` 写入前无条件图遍历扫描删除、`tubi_info` 写入前批量预查已存在 id。这些检查在部署场景全部是白跑的开销，且 `DELETE + INSERT` 成对语句本身是事故源（同批次重复 id 自冲突、配对被拆进两个并发事务导致 `already exists` 中断，见 51c7f516 修复的两处根因）。

需要把写入管线统一为幂等 `INSERT IGNORE`：写入层只管快写，旧数据清理职责完全归重生成入口 `pre_cleanup_for_regen`，首次部署零旧数据检查。

## Evidence（代码库探索，2026-06-12）

- 站点部署 generate job（`spawn_generation_process` → `run_sidecar_cli_job_with_site_events`）以 `args: Vec::new()` 启动 sidecar（仅 `-c DbOption-generate.toml`），**不带 `--regen-model`**，从不调用 `pre_cleanup_for_regen` —— 部署本来就是「空库纯写入」。
- 旧数据清理只有一个合法入口：CLI `--regen-model`（`cli_modes.rs:1848`）→ `pre_cleanup_for_regen`（按 seed 展开删 inst_relate / inst_info / geo_relate / inst_geo / neg_relate / ngmr_relate / inst_relate_aabb / tubi_relate）。
- `replace_exist` 已全面废弃（4 处恒 `false`，orchestrator/cata_model/mesh_generate/manifold_bool 均注明「由 pre_cleanup_for_regen 替代」）。
- 历史失败：`inst_relate_aabb:24381_146695 already exists` / `Cannot COMMIT` —— 均由 `DELETE + INSERT` 成对语句的并发拆分引发。
- 既有 spec 004（model-generation-write-pipeline-performance）的 Q4 决策「两阶段：生成阶段收集，结束后统一删除 + 批量插入」已被本方案取代（更简单：入口清理 + 全程幂等写入，无需收尾阶段）。

## 决策记录（grill-me 已确认，2026-06-12）

| # | 分支 | 决策 |
|---|---|---|
| Q1 | spec 定位 | **新开 spec 005**（写入语义变更），004 保持性能主题并注记 Q4 被取代 |
| Q2 | INSERT IGNORE 语义契约 | **接受**：写入层前置条件 = 目标行不存在（空库或已被入口清理）；旧数据清理唯一权威是 `pre_cleanup_for_regen`，漏删属清理层 bug，写入层不做兜底 |
| Q3 | `save_tubi_info_batch` 预查 | **删除** existing 预查，全量直接 `INSERT IGNORE`（返回值退化为提交条数，仅 debug 日志消费，无业务影响） |
| Q4 | 验收场景 | **三场景全验**：空库首次部署 / 不清理重跑 generate / `--regen-model` 重生成 |
| Q5 | 性能验收 | **不纳入硬验收**，只记观测项（基线归 spec 004） |

## 不变量（写入契约）

1. 写入层（`save_instance_data_with_report` / `save_inst_relate_aabb_rows` / `save_tubi_info_batch` / `save_instance_data_to_sql_file`）**永不**发出 DELETE 或旧数据查询；所有表写入统一 `INSERT IGNORE` / `INSERT RELATION IGNORE`。
2. 旧数据清理的**唯一权威**是 `pre_cleanup_for_regen`（仅 `--regen-model` 触发）；清理漏删导致旧值存活属清理层 bug，由清理层（含 `refno_assoc_index`）修复。
3. 同一写入批次内按目标 id 去重（`dedupe_inst_relate_aabb_rows`，保留同 id 最后一行），避免单条 INSERT 自冲突。

## Scope

- `inst_relate_aabb`：DB 直写两路径 + `.surql` 导出路径，`DELETE + INSERT` → `INSERT IGNORE`（已落地）。
- `inst_relate`：删除写入前无条件 `delete_inst_relate_by_in` 图遍历扫描（连同函数本身）；写入 `INSERT RELATION IGNORE`（已落地）。
- `geo_relate` / `neg_relate` / `ngmr_relate` / `tubi_info`：写入语句统一补 `IGNORE`（已落地）。
- `save_tubi_info_batch`：删除 `query_existing_tubi_info_ids` 预查，全量直接 `INSERT IGNORE`（待办）。
- spec 004 注记 Q4 决策被本 spec 取代（待办）。

## Non-Goals

- 不改变 `pre_cleanup_for_regen` 的清理范围与实现（含 refno_assoc_index 快路径 / Legacy 扫描降级）。
- 不在 `--regen-model` 收尾加抽样校验兜底（Q2 决策：漏删由清理层负责）。
- 不做性能基线对比（spec 004 领域）。
- 不引入两阶段「收尾统一删除 + bulk insert」写入（spec 004 Q4 旧方案，已取代）。
- 不跑 Rust test / 不编译 test 目标（仓库规则）；验证走 release 构建 + 实测。

## Requirements

1. 全写入管线（`pdms_inst.rs`）不得残留任何「写入前 DELETE / 旧数据预查」：`delete_inst_relate_by_in` 调用与函数删除；`save_tubi_info_batch` 预查删除。
2. 所有模型表写入语句统一 `INSERT IGNORE INTO` / `INSERT RELATION IGNORE INTO`（含 `.surql` 导出路径，与 DB 直写同口径）。
3. 同批次同 id 去重保留（`dedupe_inst_relate_aabb_rows` 行为不变，附带 cfg(test) 用例）。
4. 空库首次部署的生成日志中不得出现 `inst_relate` / `inst_relate_aabb` / `tubi_info` 相关 DELETE 或预查 SQL。
5. 不清理直接重跑 generate 必须幂等成功：零 `already exists`、零 `Cannot COMMIT`、行数不增长。
6. `--regen-model` 链路行为不变：pre_cleanup 删除统计正常输出，重生成后同 refno 的 `inst_relate_aabb` 指向本轮新 aabb_id。

## Acceptance Criteria

- `cargo build --release --bin aios-database` 成功（cargo check 已通过，2026-06-12）。
- 场景 1（空库首次部署）：站点部署 parse + generate 退出 0；generate 日志 grep 无 `delete_inst_relate` / `DELETE [` / `save_tubi_info_batch: total=.*existing=` 痕迹；`inst_relate` / `inst_relate_aabb` / `tubi_info` 行数与生成报告一致。
- 场景 2（不清理重跑）：同站点不删库再跑一次 generate，退出 0，零 `already exists` / `Cannot COMMIT`，三表行数与场景 1 持平。
- 场景 3（`--regen-model`）：pre_cleanup 日志含删除统计（`inst_relate_ids= ... inst_relate_aabb_ids= ...`）；重生成成功；抽查 ≥3 个 refno 的 `inst_relate_aabb.aabb_id` 为本轮新值。
- 观测项（不作硬验收）：记录场景 1 的 `base_write_ms` / `inst_aabb_ms`，供 spec 004 基线参考。
