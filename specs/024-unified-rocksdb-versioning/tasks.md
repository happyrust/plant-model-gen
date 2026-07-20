# Tasks: 版本化统一（RocksDB versioned 单一真相源）

**Input**: `specs/024-unified-rocksdb-versioning/{spec,plan}.md`

**Tests**: 仓库规则不写 cargo test；验证走 CLI `--json` + SurrealQL 脚本 + web_server HTTP POST。

**Format**: `[ID] [P?] [阶段] 描述`（[P] = 可并行）

---

## Phase 0: 地基——错误传播修复（阻塞全部后续）

- [ ] T001 [P0] `sesno_increment.rs`：`exec_statements` 逐 chunk `.check()`；`write_sesno_version_anchor` 的 `take` 错误传播（删 `unwrap_or_default` 吞噬）；`delete_increment_element` 补 check
- [ ] T002 [P0] `pdms_inst.rs::pre_cleanup_for_regen`：chunk 失败收集并传播（清理不完整则中止生成）；`let _ = model_query_response(...)` 改错误传播；`pre_cleanup_for_regen_surreal` 同理
- [ ] T003 [P0] 验证：注入坏语句确认增量报错且锚点零写入（SC-002）；正常增量 `incremental-sesno --json` 回归

**Checkpoint**: 锚点不变量成立，可独立合入。

---

## Phase 1: 模型 record id 去 sesno 槽位

- [ ] T004 [P1] `model_record_id.rs`：新形状 + 删死函数（`refno_with_sesno`/`model_refno_id_with_sesno`/`refno_from_parts`/`model_refno_sesno_range`）+ evidence 瘦身
- [ ] T005 [P1] 写入端跟随：`pdms_inst.rs`（含 .surql 影子路径）、`cata_model.rs`（tubi_relate_id ×3）、`manifold_bool.rs`、`pdms_inst_surreal.rs`、`utils.rs`
- [ ] T006 [P1] 清理端跟随：`build_delete_model_records_by_refno_sql`、pre_cleanup ranges、`delete_tubi_relate_by_branch_refnos` + 单测断言
- [ ] T007 [P1] 读取端跟随：range 全换 `model_refno_range`；**`id[3]→id[2]` ×8**（inst_query:231 / export_dbnum_instances_parquet:2053 / v3:822 / web:302 / export_prepack_lod:3886 / pdms_model_query_api:215 / sqlite_spatial_api:456 / cli_modes:3367）；删 sqlite_spatial legacy 双读（L466–485）；examples 同步
- [ ] T008 [P] [P1] CLI `model-record-id-verify` 去 `--sesno`；`export_glb.rs:214` 等注释更新
- [ ] T009 [P] [P1] 删 `pdms_inst_v2.rs`/`pdms_inst_v3.rs` 死文件；删 `cli_modes.rs` regen 冗余 `pre_cleanup_for_regen_surreal` 调用
- [ ] T010 [P1] rs-core：`rs_surreal/inst.rs` legacy tubi 查询（`tubi_relate:[pe_key, 0]..`）调用面确认后对齐/删除（跨仓）
- [ ] T011 [P1] 验证：`model-record-id-verify --json` 断言新形状；小集合 regen 两遍幂等（行数一致）；tubi POST index 连续性；导出行数对基线（SC-006 前置基线在本任务先采集）

**Checkpoint**: 全仓无 sesno 槽位构造（SC-004 前半）。

---

## Phase 2: model_gen 锚点 + 入口文件锁

- [ ] T012 [P2] schema：锚点 id 扩 `[dbnum, sesno, source]`、source ASSERT 加 'model_gen'、唯一索引三列；incremental/full 写入点改新 id
- [ ] T013 [P2] 锚点写入点：`run_incremental_sesno_once` 生成成功分支（dbnum, actual_end_sesno）；`run_regen_model` 与全量生成成功路径（dbnum_info 当前 sesno）；条件 `use_surrealdb && writes_to_surreal && !dry_run`
- [ ] T014 [P] [P2] 文件锁：`output/<project>/incremental.lock`（pid + 时间），watch-incremental / incremental-sesno / regen 共用；失败明确报错
- [ ] T015 [P2] 验证：SC-001（同 sesno 双锚点时序）；断 mesh 阶段确认无 model_gen 锚点且裸 SurrealQL VERSION 按上一锚点读到完整上一代；并发第二进程被拒（SC-005）

---

## Phase 3: 旧增量路径退役

- [ ] T016 [P3] `increment_manager.rs`：删 notify watcher / `execute_incr_update` / 解压轮询；去 `backup_data`/`backup_owner_relate` 引用
- [ ] T017 [P3] `db_model.rs`：`exec_watcher`/`spawn_exec_watcher` 退役；MQTT 文件分发（SyncE3dFileMsg/压缩传输）拆出独立保留
- [ ] T018 [P3] `lib.rs` sync_live 与 `remote_runtime.rs`：改 watch 轮询语义或明确报错指引
- [ ] T019 [P] [P3] 删 element_changes 读取三函数 + `gen_all_geos_data` target_sesno 参数链 + CLI `--target-sesno`
- [ ] T020 [P] [P3] `increment_record.rs`：删 MySQL INCREMENT_DATA 读写（保留 `IncrGeoUpdateLog`）
- [ ] T021 [P3] 验证：cargo check；watch-incremental 全链路回归（含锚点）

---

## Phase 4: 023 交付链退役

- [ ] T022 [P4] 引用图盘点：确认保留 `set_status`/`update_log`/source observation 哈希门禁；裁决 `hashing`/`bounded_runner`/`scene_tree_artifact`/`missing_mesh_repair`/`types`/`offline_deployer`
- [ ] T023 [P4] 删发布链模块群：`model_release`/`ducklake_store`/`release_state_machine`/`release_package`/`history_baseline`/`history_replay_plan`/`history_replay_validation`/`physical_baseline_snapshot`/`baseline_state`
- [ ] T024 [P4] `version_management/cli.rs`：删发布类子命令（publish-history / prepare-history-replay / prepare-physical-baseline-snapshot / inspect-history-baseline / validate-history-replay / diff / unit-diff / unit-v2-* / catalog 类）；保留 `history {snapshot,timeline,diff}`
- [ ] T025 [P] [P4] 删 `web_api/model_version_api.rs` + 路由挂载
- [ ] T026 [P4] ModelWriter 收敛：删 `model_writer_ducklake.rs`、`ModelWriterMode::{DuckLake,Parquet}`、`TransformWriteBackend::DuckLake`、`bin/ducklake_parity.rs`；`model_writer_verify` 去 ducklake 证据；Parquet transform 后端与 web Parquet 导出器**不动**
- [ ] T027 [P4] source observation 内联化：保留哈希前后校验；删 manifest 归档与 `build_incremental_publication_handoff`
- [ ] T028 [P4] `model-version export --dbnum --sesno` 骨架：锚点校验 + 当前态导出（历史态待 P5）
- [ ] T029 [P4] 验证：cargo check；web_server 启动 + 路由审计；全仓 `release_id|ducklake|unit_version` 符号审计（SC-004）

---

## Phase 5: 模型历史查询（门禁在前，独立于 P0–P4 合入）

- [ ] T030 [P5] **门禁**：`db-data/verify_versioned_model_range.surql`——range record id + VERSION 贯通、regen 前后两代各自可读；不过则 D6 回炉
- [ ] T031 [P5] rs-core `version_query`：`model_snapshot_at`/`model_diff`（model_gen 锚点 + range VERSION + Expired 翻译）（跨仓）
- [ ] T032 [P5] 本仓 CLI：`model-version history model-snapshot / model-diff`（--json）；`export` 接通历史态
- [ ] T033 [P5] 验证：SC-003 全项；regen 前后 model-diff 变化集；Expired 文案

---

## Phase 6: 迁移与收尾

- [ ] T034 [P6] 一次性清库脚本（存量 versioned 测试站：REMOVE 模型表 + 锚点表重建 + regen 指引）
- [ ] T035 [P] [P6] 文档：022 quickstart/ops-notes 横幅、CHANGELOG、retention=0 警示复核
- [ ] T036 [P6] 全链路终验：SC-001~SC-006 逐项过 + 记录到本文件附录

---

## Dependencies & Execution Order

- P0 阻塞全部；P1 → P2（锚点验证依赖新 id 清理语义）→ P3 → P4；P5 仅依赖 P2（锚点）+ 自身门禁 T030，可与 P3/P4 并行推进；P6 收尾
- 跨仓任务（T010、T031）：rs-core 先行提交推送，本仓 `cargo update -p aios_core` 跟进
- MVP = P0 + P1 + P2（锚点闭环 + id 清理）；P3/P4 是删除量大头；P5 提供查询消费端
