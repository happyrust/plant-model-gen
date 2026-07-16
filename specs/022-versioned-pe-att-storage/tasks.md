# Tasks: PE/ATT 版本化存储（SurrealDB RocksDB versioned）

**Input**: Design documents from `/specs/022-versioned-pe-att-storage/`

**Prerequisites**: plan.md, spec.md

**Tests**: 遵循仓库规则不写 cargo test；每个阶段的验证任务用 CLI `--json` + SurrealQL 脚本 + HTTP（如涉及 web_server）完成。

**Organization**: 按 plan.md 的 M1~M4 里程碑组织，对应 spec.md 的 US1~US4。

---

## ⚠️ 进度真相校对（2026-07-16 晚间更新）

**关键事实**：M1（T001–T007）与 M2 主体（T008–T011）已落定到分支 `feat/022-versioned-pe-att-storage`（提交 `c55844f0`）。此前曾长期仅存在于 `stash@{0}`（`dirty-wt-before-narrow-023`）；stash **暂不 drop**，确认无误后再清。

- 勾选状态说明：下文 `[x]` = 已落定到分支 HEAD；`[ ]` = 未做。
- **T012（2026-07-16）**：`db-data/verify_versioned_pe_att.surql` 在 `rocksdb://…?versioned=true&retention=30d`（8030）上通过——锚点 3 行（sesno 1/2/3，source incremental/incremental/full）、VERSION $t1→BORE=150、VERSION $t2→BORE=200、硬删除后当前态空、幂等 UPSERT 行数仍为 3。空批不写锚点见 R2 静态核对。真机 `incremental-sesno` E2E / 断连失败路径未跑（需项目数据）。
- **R1（关闭）**：`parse_single_db_file` 只导出 tree / 更新 db_meta，**不写 Surreal PE/ATT**，故不写 full 锚点；full 锚点仅在 `sync_pdms*` 写库 join 后固化（代码旁已加注释）。
- **R2（关闭）**：`persist_pdms_increment_grouped` 在 `actual_start/end_sesno==0` 时提前返回（L670），锚点写入在成功路径末尾（L768+），空批不写锚点。

---

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 可并行（不同文件、无依赖）
- **[Story]**: 对应用户故事（US1~US4）或基础设施（INFRA）

---

## Phase 1: M1 — 实例级 versioned 开关（INFRA，阻塞全部故事）

**Purpose**: 让 SUL_DB 实例以 `versioned=true&retention=<r>` 启动，是锚点与历史查询的前提。

- [x] T001 [INFRA] `src/options.rs`：`DbOptionExt` 新增 `versioned_storage: bool` 与 `version_retention: String`（默认 "90d"）。⚠️ 与原计划的偏差：默认改为 **false**——versioned 是建库属性，存量数据目录以 versioned=true 打开会因 comparator 不匹配直接失败；"新站点默认开"改由建站流程显式写配置（M4/T022 处理）。已落定 `c55844f0`
- [x] T002 [INFRA] `src/options.rs` 新增 `pub fn rocksdb_conn_str(data_path, versioned, retention) -> String` 与 `current_versioned_params()`（供拿不到 DbOptionExt 的 web_server 手动工具从 DB_OPTION_FILE 提取）。已落定 `c55844f0`
- [x] T003 [INFRA] `src/cli_modes.rs`：`auto_start_surreal` 改收 `&DbOptionExt` 并拼 versioned 参数；`surreal_start_test_command` 改收完整 db_uri；启动诊断/手动启动提示文案同步。已落定 `c55844f0`
- [x] T004 [P] [INFRA] `src/web_server/managed_project_sites.rs`：新增 `site_versioned_params`（读站点 DbOption.toml）+ `site_rocksdb_conn_str`；站点进程、systemd 模板、nohup 模板三处透传（nohup 处整体 sh_quote，`?`/`&` 留在引号内）。已落定 `c55844f0`
- [x] T005 [P] [INFRA] `db_startup_manager.rs`（versioned 开启时才由 file: 换成带参 rocksdb://）、`handlers.rs`（经 current_versioned_params）、`src/bin/web_server.rs`（从同一 toml 提取扩展字段）透传完成。已落定 `c55844f0`
- [x] T006 [INFRA] `cargo check` 全绿；全仓 `rocksdb://` 审计——src/ 下仅剩 helper 本体、cli_modes 单测常量、无关的 file:// URL 判断（2026-07-13 已过；落定后以 `c55844f0` 为准）
- [x] T007 [INFRA] 冒烟通过（fork release 3.3.0-nightly，8031 端口 versioned 实例）：VERSION $t1 读到旧值(BORE 150/sesno 1)、硬 DELETE 后 VERSION $t1 仍可查删除前记录；8032 非 versioned 对照实例 VERSION 查询报 "The underlying datastore does not support versioned queries"（预期）

**Checkpoint**: 新建实例具备版本化能力，存量实例不受影响。**M1 已落定 `c55844f0`。**

---

## Phase 2: M2 — sesno_version_anchor 锚点表（US2，P1）

**Goal**: 增量/全量落库完成后固化 sesno→时间戳锚点；失败不写。

**Independent Test**: 跑一次 incremental-sesno，锚点表出现 (dbnum, sesno) 记录且时间戳晚于本批全部写入;人为制造落库失败,无新锚点。

- [x] T008 [US2] 锚点表 schema 定义：`DEFINE TABLE sesno_version_anchor SCHEMAFULL` + 字段 dbnum(int)/sesno(int)/anchored_at(datetime DEFAULT time::now())/source(string) + UNIQUE INDEX (dbnum, sesno)。已实现于 `ensure_sesno_version_anchor_schema()`（`OnceCell` 幂等 + `note: option<string>` + `source ASSERT IN ['full','incremental']`），挂进 `sync_pdms`/`sync_pdms_with_callback`。已落定 `c55844f0`
- [x] T009 [US2] `persist_pdms_increment_grouped` 成功路径末尾 UPSERT 锚点 `{dbnum, sesno: actual_end_sesno, source: "incremental"}` + `PdmsIncrementPersistStats.anchors`。已落定 `c55844f0`
- [x] T010 [US2] `sync_pdms` / `sync_pdms_with_callback` 全量完成后 `write_full_version_anchors`（sender drop + insert_handles 排空后）。已落定 `c55844f0`。`parse_single_db_file` 见 R1（不适用）
- [x] T011 [US2] `src/main.rs`：`run_incremental_sesno_once` 汇总 JSON 增加 `"version_anchor": persist_stats.anchors`。已落定 `c55844f0`
- [x] T012 [US2] 验证：`db-data/verify_versioned_pe_att.surql` @ 8030 versioned 通过（锚点 3 行 + VERSION 时间旅行 + 幂等）；R2 空批静态核对通过。**未覆盖**：真机 `incremental-sesno` E2E、断连失败路径（有数据后再补）

**Checkpoint**: US2 可独立验收（SC-002，契约级）。**M2 已落定；T012 契约冒烟 2026-07-16 通过。**

### 复核项（2026-07-16 关闭）

- **R1（关闭·不适用）**：`parse_single_db_file` 不写 Surreal PE/ATT，故不写 full 锚点；注释已加。
- **R2（关闭·通过）**：空批 guard 在锚点写入之前提前返回，空批不写锚点。

---

## Phase 3: M3 — rs-core version_query 封装 + history CLI（US1 P1 / US3 P2）

**Goal**: 按 sesno 查询历史快照、时间线、区间 diff 的 CLI 能力。

**Independent Test**: 对 db-data 测试实例执行三个子命令,输出与手工 SurrealQL VERSION 查询一致。

### rs-core 侧（D:\work\plant-code\rs-core）

- [x] T013 [US1] `src/rs_surreal/version_query.rs` 新建模块 + `mod.rs` 导出：`resolve_anchor`（精确/`exact:false` 回退）+ `HistoryError::Expired`（InvalidArgument/GC 文案翻译）。2026-07-16 落地
- [x] T014 [US1] `snapshot_at(refno, sesno, dbnum, pe_key_override)`——锚点换算后 PE VERSION；有 noun 时尝试 ATT 表同刻查询
- [x] T015 [US3] `diff_range`——两端快照字段级对比（changed/added/deleted/unchanged）
- [x] T016 [US1] `timeline`——锚点区间内 content_hash 变化点（O(锚点数)）

### plant-model-gen 侧

- [x] T017 [US1] `model-version history {snapshot,timeline,diff}` 注册，均支持 `--json`；snapshot 支持 `--pe-key` 夹具
- [x] T018 [US1] handler → `aios_core::{snapshot_at,timeline,diff_range}`；Expired 经 `format_history_error` 输出固定中文提示；main 在 history 前 `ensure_surreal_connected`
- [x] T019 [P] [US1] `db-data/verify_versioned_pe_att.surql` 已存在并在 T012 跑通
- [x] T020 [US1] 验证（SC-001/SC-003）：2026-07-16 对 8030 versioned + `DbOption-t020-history` 跑通——`history snapshot` sesno1→BORE=150 / sesno2→BORE=200；`timeline` 1/2 exists、3 deleted；`diff` 1→2 `attrs.BORE` 150→200。Expired 路径未单独构造（依赖文案映射，有真实 GC 水位后再补）

**Checkpoint**: US1/US3 MVP + CLI E2E（夹具）已通过。

---

## Phase 4: M4 — 存量站点切换（US4，P2）

**Goal**: 存量站点可安全切换到 versioned 实例。

**Independent Test**: 测试站点全流程切换后当前态一致、增量回归通过。

- [x] T021 [US4] `specs/022-versioned-pe-att-storage/quickstart.md`：切换手册已刷新——新建 versioned 数据目录 → 配置开关 → sync_pdms 重灌 → 锚点确认 → 抽样比对 → 切流 → incremental + history CLI；含 T022 管理端行为表。2026-07-16
- [x] T022 [US4] `CreateManagedSiteRequest` / `UpdateManagedSiteRequest` 增加 `versioned_storage` / `version_retention`；`update_site` 对 Parsed/Failed 改参拒绝（文案含重灌指引）；`build_site_config` 保留既有开关；create 可显式开启。2026-07-16
- [ ] T023 [US4] 验证（SC-004）：按 quickstart 对测试站点走全流程,记录抽样比对与增量回归结果到 quickstart 附录（**阻塞**：需真实 managed 测试站点）

**Checkpoint**: T021/T022 落地；T023 待真站演练后关闭。

---

## Phase 5: Polish & Cross-Cutting

- [x] T024 [P] 七项决策要点与运维约束精简版已写入根 `AGENTS.md`（建库属性 / 串行增量 / 锚点入口 / retention / MODEL_KV / 023 共存）；全文见 `ops-notes.md`。2026-07-16
- [x] T025 [P] `specs/022-versioned-pe-att-storage/ops-notes.md`：磁盘水位观察、retention 调整（改 toml 重启、无需重建库）、未开 MODEL_KV 警示、与 T022 管理端行为对齐。2026-07-16
- [ ] T026 跑一遍 quickstart.md 全流程终验（依赖 T023 真站；契约级冒烟已由 T012/T020 覆盖）

---

## Dependencies & Execution Order

- **Phase 1 (M1)** 阻塞全部后续:T001 → T002 → T003/T004/T005（三者并行）→ T006 → T007
- **Phase 2 (M2)** 依赖 Phase 1:T008 → T009/T010（并行）→ T011 → T012
- **Phase 3 (M3)** 依赖 Phase 2（锚点表必须存在）:rs-core T013 → T014/T015/T016 → 本仓 T017 → T018;T019 可与 rs-core 侧并行
- **Phase 4 (M4)** 依赖 Phase 1~3 全部完成
- rs-core 改动（T013~T016）与 plant-model-gen 的 T017 跨仓库,注意 rs-core 先行提交/推送后本仓才能 `cargo update -p aios_core` 拉到

## Implementation Strategy

MVP = Phase 1 + Phase 2 + Phase 3 的 snapshot 路径（T013/T014/T017/T018/T020）。timeline 与 diff（T015/T016）可在 MVP 验收后补齐;Phase 4 面向存量站点,可最后排期。
