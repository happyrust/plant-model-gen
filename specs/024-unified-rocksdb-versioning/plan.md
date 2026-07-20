# Implementation Plan: 版本化统一（RocksDB versioned 单一真相源）

**Branch**: `024-unified-rocksdb-versioning` | **Date**: 2026-07-20 | **Spec**: `specs/024-unified-rocksdb-versioning/spec.md`

## Summary

以 RocksDB versioned + 锚点为唯一版本机制：修好锚点不变量的错误传播地基（P0），删除模型 record id 的 sesno 槽位（P1），补 model_gen 模型锚点与增量入口锁（P2），退役旧 notify watcher 增量路径（P3），退役 023 DuckLake 交付链与 DuckLake ModelWriter 后端、交付改按锚点导出（P4），扩展 rs-core version_query 提供模型历史查询 CLI（P5），最后迁移脚本与文档收尾（P6）。

## Technical Context

**依赖**: surrealdb fork（dev-3.1，RocksDB UDT versioned）；aios_core（rs-core dev-3.1，本仓 Cargo patch 指向 `../rs-core`）

**测试约束**（仓库规则）: 不写 cargo test；CLI `--json` + SurrealQL 脚本 + web_server HTTP POST 验证

**关键不变量**: "有锚点 = 该 source 语义下的一致快照"。P0 先于一切：语句级错误必须传播，否则锚点语义是漏的。

**迁移边界**（D8 硬切换）: versioned 建库属性要求新目录重灌 → 升级到本版 = 按 022 quickstart 重建站点（新 id 形状从空库开始）；存量 versioned 测试站走一次性清库脚本。不做双读。

## 阶段划分

### P0 — 地基：错误传播修复（D7，先行独立合入）

1. `sesno_increment.rs::exec_statements` 改为逐 chunk `.check()`；`write_sesno_version_anchor` 的 `take` 错误传播（去掉 `unwrap_or_default` 吞噬）；`delete_increment_element` 同理
2. `pdms_inst.rs::pre_cleanup_for_regen`：chunk 失败计数并向调用方传播（清理不完整时中止生成，防 INSERT IGNORE 静默保留旧数据）；`model_query_response` 结果不再 `let _ =`
3. 验证：临时注入坏语句（如向不存在表 UPSERT）确认增量落库报错且锚点零写入；正常增量 `--json` 回归

### P1 — 模型 record id 去 sesno 槽位（D1 前置，沿用已评审的七层清单）

| 层 | 内容 |
|---|---|
| 1 | `model_record_id.rs`：`ModelRefnoIdParts` 去 sesno；新形状（点表 `[ref0,ref1]`、geo_relate `[ref0,ref1,geo_index(,hash)]`、tubi `[ref0,ref1,index]`、neg/ngmr 6 元）；删 `refno_with_sesno`/`model_refno_id_with_sesno`/`refno_from_parts`/`model_refno_sesno_range`；evidence 结构瘦身 |
| 2 | 写入端：`pdms_inst.rs`（在线 + .surql 影子路径）、`cata_model.rs` tubi_relate_id ×3、`manifold_bool.rs`（CatePos/bool 行）、`pdms_inst_surreal.rs`、`utils.rs` bool 状态行 |
| 3 | 清理端：`build_delete_model_records_by_refno_sql`、pre_cleanup ranges、`delete_tubi_relate_by_branch_refnos` 及单测断言 |
| 4 | 读取端：`model_refno_sesno_range` 调用点全部换 `model_refno_range`（inst_query / export_dbnum_instances_{parquet,v3,web} / export_prepack_lod / room_model / sqlite_spatial_api / pdms_model_query_api / cli_modes）；**`id[3] → id[2]` 位置平移 8 处**；删 `sqlite_spatial_api.rs` legacy `[pe_key, index]` 双读回退；examples 同步 |
| 5 | CLI：`model-record-id-verify` 去 `--sesno`；相关注释更新 |
| 6 | rs-core：`rs_surreal/inst.rs` legacy 形状 tubi 查询（`tubi_relate:[pe_key, 0]..`）确认调用面后对齐或删除 |
| 7 | 死文件：`pdms_inst_v2.rs`、`pdms_inst_v3.rs`（未挂 mod.rs）；`cli_modes.rs` regen 路径冗余的 `pre_cleanup_for_regen_surreal` 调用 |

风险焦点：`id[3]→id[2]` 漏改是静默错误，验证阶段专项核对 tubi index 连续性。

### P2 — model_gen 锚点 + 入口文件锁（D3、D10a）

1. schema：`ensure_sesno_version_anchor_schema` 迁移到 `id: [dbnum, sesno, source]`，source ASSERT 加 `'model_gen'`；唯一索引改三列；incremental/full 写入点改新 id
2. 锚点写入由**调用方**在生成成功后执行（sesno 语义调用方最清楚）：
   - 增量：`run_incremental_sesno_once` 生成成功分支，按 (dbnum, actual_end_sesno) 写
   - regen：`cli_modes::run_regen_model` 成功后，按 dbnum_info 当前 sesno 写
   - 全量：全量生成完成路径同 regen
   - 条件：`use_surrealdb && model_writer_mode.writes_to_surreal() && !gen_model_dry_run`
3. 文件锁：`output/<project>/incremental.lock`（含 pid、启动时间），watch-incremental 与手动 incremental-sesno、regen 共用获取逻辑；获取失败明确报错退出
4. 验证：SC-001/SC-005；生成失败路径（断 mesh）无 model_gen 锚点，上一锚点可读上一代（依赖 P5 查询就绪前可用裸 SurrealQL VERSION 验证）

### P3 — 旧增量路径退役（D4、D10b）

1. `increment_manager.rs`：删 notify watcher / `execute_incr_update` / 相关轮询与解压逻辑；`aios_core::version::{backup_data, backup_owner_relate}` 引用一并去除
2. `db_model.rs`：`exec_watcher` / `spawn_exec_watcher` 退役；MQTT 文件分发（SyncE3dFileMsg 收发、压缩传输）拆出独立保留（不依赖 watcher）
3. `lib.rs` sync_live 分支与 `remote_runtime.rs` 的 init_watcher 调用改为 watch 轮询语义或明确报错指引
4. `element_changes` 死路径：删 `get_changes_at_sesno` / `get_changes_between_sesnos` / `has_changes_at_sesno`；`gen_all_geos_data` 的 `target_sesno` 参数链（含 CLI `--target-sesno`）删除
5. `increment_record.rs`：MySQL INCREMENT_DATA 表读写（sql feature 部分）删除；`IncrGeoUpdateLog` 保留
6. 验证：cargo check + watch-incremental 全链路回归

### P4 — 023 交付链退役（D1/D2/D9）

1. **引用图盘点先行**：version_management 内 `set_status`/`update_log`/`source_observation`（哈希门禁部分）确认保留；`hashing`/`bounded_runner`/`scene_tree_artifact`/`missing_mesh_repair`/`types` 按"唯一消费方是否为发布链"裁决；`offline_deployer` bin 盘点其对 release_package 的依赖面后裁决
2. 删除模块群：`model_release` / `ducklake_store` / `release_state_machine` / `release_package` / `history_baseline` / `history_replay_plan` / `history_replay_validation` / `physical_baseline_snapshot` / `baseline_state`
3. CLI：`version_management/cli.rs` 发布类子命令删除（publish-history、prepare-history-replay、prepare-physical-baseline-snapshot、inspect-history-baseline、validate-history-replay、diff/unit-diff/unit-v2-*、catalog 迁移类）；保留 `history {snapshot,timeline,diff}` 并为 P5 新命令留位
4. HTTP：删 `web_api/model_version_api.rs` 与路由挂载
5. ModelWriter 后端：删 `model_writer_ducklake.rs`、`ModelWriterMode::{DuckLake, Parquet}`（收敛 Surreal/DrainOnly）、`TransformWriteBackend::DuckLake`（Parquet transform 后端保留）、`bin/ducklake_parity.rs`；`model_writer_verify` 去 ducklake 证据
6. source observation：`run_incremental_sesno_once` 内联哈希前后校验保留；manifest 目录归档与 handoff 清单（`build_incremental_publication_handoff`）删除
7. 新增按锚点导出骨架：`model-version export --dbnum --sesno [--format]`——MVP 先支持"锚点校验 + 当前态导出"（复用现有导出器），历史态导出在 P5 range VERSION 贯通后接入
8. 验证：cargo check、web_server 启动路由检查、导出冒烟、全仓 `release_id|ducklake|unit_version` 符号审计

### P5 — 模型历史查询（D6，门禁在前）

1. **门禁**：`db-data/verify_versioned_model_range.surql`——versioned 实例上验证 `SELECT ... FROM inst_relate:[a,b,NONE]..=[a,b,..] VERSION $t` 及 regen 前后两个时间戳各读到完整一代；**不过则 D6 回炉，P5 暂停（不影响 P0–P4）**
2. rs-core `version_query`：`model_snapshot_at(refno, sesno, dbnum)`（model_gen 锚点解析 + 模型表 range VERSION）、`model_diff(refnos, from, to, dbnum)`；复用 `HistoryError::Expired` 翻译
3. 本仓 CLI：`model-version history model-snapshot / model-diff`（--json）；`model-version export` 接通历史态
4. 验证：SC-003；regen 前后 model-diff 变化集正确；Expired 路径文案

### P6 — 迁移与收尾（D8）

1. 一次性清库脚本：存量 versioned 测试站 REMOVE 模型表 + 锚点表重建 + regen 指引
2. 文档：022 quickstart/ops-notes 加"已被 024 取代"横幅；CHANGELOG；retention=0 警示复核
3. 全链路验证清单执行（见 tasks.md T-最终）

## 风险与对策

| 风险 | 对策 |
|------|------|
| range VERSION 在 fork 上不贯通 | P5 门禁前置；P0–P4 交付不依赖它；不过则模型历史查询方案回炉（如按锚点导出快照归档） |
| `id[3]→id[2]` 漏改静默出错 | P1 验证专项：tubi index 连续性 POST 核对 + 导出行数基线对比 |
| P4 误删存活依赖 | 引用图盘点先行；set_status/update_log/source_observation 明确保留清单 |
| offline_deployer 依赖面不明 | P4 盘点后单独裁决，必要时保留其非 release 部分 |
| rs-core 跨仓节奏 | rs-core 先行提交推送，本仓 `cargo update -p aios_core` 跟进；P5 独立于 P0–P4 合入 |
| 生成失败后模型库残缺窗口 | model_gen 锚点 + versioned 保证上一代可读（SC-003 后半句专项验证） |

## Out of Scope

- HTTP 历史查询 API（延续 022 Q7 决策）
- 模型历史的自动归档/快照导出调度
- 非 versioned 存量站点的原地迁移（统一走重建）
