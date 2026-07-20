# Tasks: 版本化统一（RocksDB versioned 单一真相源）

**Input**: `specs/024-unified-rocksdb-versioning/{spec,plan}.md`

**Tests**: 仓库规则不写 cargo test；验证走 CLI `--json` + SurrealQL 脚本 + web_server HTTP POST。

**Format**: `[ID] [P?] [阶段] 描述`（[P] = 可并行）

---

## Phase 0: 地基——错误传播修复（阻塞全部后续）

- [x] T001 [P0] `sesno_increment.rs`：`exec_statements` 逐 chunk `.check()`；`write_sesno_version_anchor` 的 `take` 错误传播（删 `unwrap_or_default` 吞噬）；`delete_increment_element` 补 check
- [x] T002 [P0] `pdms_inst.rs::pre_cleanup_for_regen`：chunk 失败收集并传播（清理不完整则中止生成）；`let _ = model_query_response(...)` 改错误传播；`pre_cleanup_for_regen_surreal` 同理
- [x] T003 [P0] 验证：注入坏语句确认增量报错且锚点零写入（SC-002）；正常增量 `incremental-sesno --json` 回归

**Checkpoint**: 锚点不变量成立，可独立合入。

---

## Phase 1: 模型 record id 去 sesno 槽位

- [x] T004 [P1] `model_record_id.rs`：新形状 + 删死函数（`refno_with_sesno`/`model_refno_id_with_sesno`/`refno_from_parts`/`model_refno_sesno_range`）+ evidence 瘦身
- [x] T005 [P1] 写入端跟随：`pdms_inst.rs`（含 .surql 影子路径）、`cata_model.rs`（tubi_relate_id ×3）、`manifold_bool.rs`、`pdms_inst_surreal.rs`、`utils.rs`
- [x] T006 [P1] 清理端跟随：`build_delete_model_records_by_refno_sql`、pre_cleanup ranges、`delete_tubi_relate_by_branch_refnos` + 单测断言
- [x] T007 [P1] 读取端跟随：range 全换 `model_refno_range`；**`id[3]→id[2]` ×8**（inst_query:231 / export_dbnum_instances_parquet:2053 / v3:822 / web:302 / export_prepack_lod:3886 / pdms_model_query_api:215 / sqlite_spatial_api:456 / cli_modes:3367）；删 sqlite_spatial legacy 双读（L466–485）；examples 同步
- [x] T008 [P] [P1] CLI `model-record-id-verify` 去 `--sesno`；`export_glb.rs:214` 等注释更新
- [x] T009 [P] [P1] 删 `pdms_inst_v2.rs`/`pdms_inst_v3.rs` 死文件；删 `cli_modes.rs` regen 冗余 `pre_cleanup_for_regen_surreal` 调用
- [x] T010 [P1] rs-core：`rs_surreal/inst.rs` legacy tubi 查询（`tubi_relate:[pe_key, 0]..`）调用面确认后对齐/删除（跨仓）
- [x] T011 [P1] 验证：`model-record-id-verify --json` 断言新形状；小集合 regen 两遍幂等（行数一致）；tubi POST index 连续性；导出行数对基线（SC-006 前置基线在本任务先采集）

**Checkpoint**: 全仓无 sesno 槽位构造（SC-004 前半）。

---

## Phase 2: model_gen 锚点 + 入口文件锁

- [x] T012 [P2] schema：锚点 id 扩 `[dbnum, sesno, source]`、source ASSERT 加 'model_gen'、唯一索引三列；incremental/full 写入点改新 id
- [x] T013 [P2] 锚点写入点：`run_incremental_sesno_once` 生成成功分支（dbnum, actual_end_sesno）；`run_regen_model` 与全量生成成功路径（dbnum_info 当前 sesno）；条件 `use_surrealdb && writes_to_surreal && !dry_run`
- [x] T014 [P] [P2] 文件锁：`output/<project>/incremental.lock`（pid + 时间），watch-incremental / incremental-sesno / regen 共用；失败明确报错
- [x] T015 [P2] 验证：SC-001（同 sesno 双锚点时序）；断 mesh 阶段确认无 model_gen 锚点且裸 SurrealQL VERSION 按上一锚点读到完整上一代；并发第二进程被拒（SC-005）

---

## Phase 3: 旧增量路径退役

- [x] T016 [P3] `increment_manager.rs`：删 notify watcher / `execute_incr_update` / 解压轮询；去 `backup_data`/`backup_owner_relate` 引用
- [x] T017 [P3] `db_model.rs`：`exec_watcher`/`spawn_exec_watcher` 退役；MQTT 文件分发（SyncE3dFileMsg/压缩传输）拆出独立保留
- [x] T018 [P3] `lib.rs` sync_live 与 `remote_runtime.rs`：改 watch 轮询语义或明确报错指引
- [x] T019 [P] [P3] 删 element_changes 读取三函数 + `gen_all_geos_data` target_sesno 参数链 + CLI `--target-sesno`
- [x] T020 [P] [P3] `increment_record.rs`：删 MySQL INCREMENT_DATA 读写（保留 `IncrGeoUpdateLog`）
- [x] T021 [P3] 验证：cargo check；watch-incremental 全链路回归（含锚点）

---

## Phase 4: 023 交付链退役

- [x] T022 [P4] 引用图盘点：确认保留 `set_status`/`update_log`/source observation 哈希门禁；裁决 `hashing`/`bounded_runner`/`scene_tree_artifact`/`missing_mesh_repair`/`types`/`offline_deployer`
- [x] T023 [P4] 删发布链模块群：`model_release`/`ducklake_store`/`release_state_machine`/`release_package`/`history_baseline`/`history_replay_plan`/`history_replay_validation`/`physical_baseline_snapshot`/`baseline_state`
- [x] T024 [P4] `version_management/cli.rs`：删发布类子命令（publish-history / prepare-history-replay / prepare-physical-baseline-snapshot / inspect-history-baseline / validate-history-replay / diff / unit-diff / unit-v2-* / catalog 类）；保留 `history {snapshot,timeline,diff}`
- [x] T025 [P] [P4] 删 `web_api/model_version_api.rs` + 路由挂载
- [x] T026 [P4] ModelWriter 收敛：删 `model_writer_ducklake.rs`、`ModelWriterMode::{DuckLake,Parquet}`、`TransformWriteBackend::DuckLake`、`bin/ducklake_parity.rs`；`model_writer_verify` 去 ducklake 证据；Parquet transform 后端与 web Parquet 导出器**不动**
- [x] T027 [P4] source observation 内联化：保留哈希前后校验；删 manifest 归档与 `build_incremental_publication_handoff`
- [x] T028 [P4] `model-version export --dbnum --sesno` 骨架：锚点校验 + 当前态导出（历史态待 P5）
- [x] T029 [P4] 验证：cargo check；web_server 启动 + 路由审计；全仓 `release_id|ducklake|unit_version` 符号审计（SC-004）

---

## Phase 5: 模型历史查询（门禁在前，独立于 P0–P4 合入）

- [x] T030 [P5] **门禁**：`db-data/verify_versioned_model_range.surql`——range record id + VERSION 贯通、regen 前后两代各自可读；不过则 D6 回炉
- [x] T031 [P5] rs-core `version_query`：`model_snapshot_at`/`model_diff`（model_gen 锚点 + range VERSION + Expired 翻译）（跨仓）
- [x] T032 [P5] 本仓 CLI：`model-version history model-snapshot / model-diff`（--json）；`export` 接通历史态
- [x] T033 [P5] 验证：SC-003 全项；regen 前后 model-diff 变化集；Expired 文案

---

## Phase 6: 迁移与收尾

- [x] T034 [P6] 一次性清库脚本（存量 versioned 测试站：REMOVE 模型表 + 锚点表重建 + regen 指引）
- [x] T035 [P] [P6] 文档：022 quickstart/ops-notes 横幅、CHANGELOG、retention=0 警示复核
- [x] T036 [P6] 全链路终验：SC-001~SC-006 逐项过 + 记录到本文件附录

---

## Dependencies & Execution Order

- P0 阻塞全部；P1 → P2（锚点验证依赖新 id 清理语义）→ P3 → P4；P5 仅依赖 P2（锚点）+ 自身门禁 T030，可与 P3/P4 并行推进；P6 收尾
- 跨仓任务（T010、T031）：rs-core 先行提交推送，本仓 `cargo update -p aios_core` 跟进
- MVP = P0 + P1 + P2（锚点闭环 + id 清理）；P3/P4 是删除量大头；P5 提供查询消费端

---

## 终验附录（2026-07-20）

### 执行命令

- `$env:AIOS_LOWMEM_BUILD='1'; $env:CARGO_BUILD_JOBS='1'; powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-sync-cli.ps1`
  - 首轮并行 release 编译在 `surrealdb-core` 遇到 Windows rustc 内存分配失败，脚本自动回退串行构建。
  - 串行 release 构建成功（`Finished release ... in 38m 46s`）；产物 `D:\Rust\target\release\aios-database.exe`，大小 `70,194,176` 字节。
- `pnpm --dir ui/admin build`
  - 退出码 `0`；管理端静态资源重新生成，源代码与产物均不再包含已退役的 `target_sesno` 配置。
- `$env:CARGO_BUILD_JOBS='1'; powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke/unified_versioning_e2e.ps1 -SurrealBin surreal`
  - 开始：`2026-07-20T17:57:14.4848150+08:00`
  - 完成：`2026-07-20T17:59:53.3022783+08:00`
  - 总耗时：`158,817 ms`
  - 退出码：`0`
  - JSON 证据：`db-data/unified_versioning_e2e.out.json`

### SC-001～SC-006 结果

- **SC-001 PASS**：`verify_024_model_history.surql` 验证同一 sesno 的 data/model_gen 双锚点存在，且 model_gen 写入时序更晚。
- **SC-002 PASS**：注入语句级错误后 statement=`ERR`、commit state=`pending`、anchor count=`0`。
- **SC-003 PASS**：
  - array record-id range + VERSION 门禁通过；
  - 单 refno 历史快照耗时 `703 ms`；
  - 请求 sesno `150` 确定性回退到 model_gen sesno `100`；
  - 硬删除前后快照分别为 `exists=true/false`；
  - model diff 返回 `changed` 与 `deleted` 两类变化；
  - 缺少 model_gen 锚点时显式失败。
- **SC-004 PASS**：
  - `cargo build --features review --bin aios-database --bin web_server`；
  - `cargo check --no-default-features --features sync-cli,sqlite-index`；
  - `cargo check --features full`；
  - pure-refno ID、退役文件、`release_id|unit_version|ducklake`、旧 watcher 与 `target_sesno` 符号审计均为零命中；
  - 启动 web_server 后旧 `/api/model-version/releases` 返回 `404`。
- **SC-005 PASS**：持有 `incremental.lock` 时第二个写命令按预期退出码 `1`，明确报告项目写入锁占用。
- **SC-006 PASS**：sesno `100` 与 `200` 的锚点 v3 导出均为 `2` 行；世界变换 X 分别为 `10.0/11.0`；删除 refno 只存在于第一代导出。
