# Tasks: PE/ATT 版本化存储（SurrealDB RocksDB versioned）

**Input**: Design documents from `/specs/022-versioned-pe-att-storage/`

**Prerequisites**: plan.md, spec.md

**Tests**: 遵循仓库规则不写 cargo test；每个阶段的验证任务用 CLI `--json` + SurrealQL 脚本 + HTTP（如涉及 web_server）完成。

**Organization**: 按 plan.md 的 M1~M4 里程碑组织，对应 spec.md 的 US1~US4。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 可并行（不同文件、无依赖）
- **[Story]**: 对应用户故事（US1~US4）或基础设施（INFRA）

---

## Phase 1: M1 — 实例级 versioned 开关（INFRA，阻塞全部故事）

**Purpose**: 让 SUL_DB 实例以 `versioned=true&retention=<r>` 启动，是锚点与历史查询的前提。

- [x] T001 [INFRA] `src/options.rs`：`DbOptionExt` 新增 `versioned_storage: bool` 与 `version_retention: String`（默认 "90d"）。⚠️ 与原计划的偏差：默认改为 **false**——versioned 是建库属性，存量数据目录以 versioned=true 打开会因 comparator 不匹配直接失败；"新站点默认开"改由建站流程显式写配置（M4/T022 处理）
- [x] T002 [INFRA] `src/options.rs` 新增 `pub fn rocksdb_conn_str(data_path, versioned, retention) -> String` 与 `current_versioned_params()`（供拿不到 DbOptionExt 的 web_server 手动工具从 DB_OPTION_FILE 提取）
- [x] T003 [INFRA] `src/cli_modes.rs`：`auto_start_surreal` 改收 `&DbOptionExt` 并拼 versioned 参数；`surreal_start_test_command` 改收完整 db_uri；启动诊断/手动启动提示文案同步
- [x] T004 [P] [INFRA] `src/web_server/managed_project_sites.rs`：新增 `site_versioned_params`（读站点 DbOption.toml）+ `site_rocksdb_conn_str`；站点进程、systemd 模板、nohup 模板三处透传（nohup 处整体 sh_quote，`?`/`&` 留在引号内）
- [x] T005 [P] [INFRA] `db_startup_manager.rs`（versioned 开启时才由 file: 换成带参 rocksdb://）、`handlers.rs` L4191/L7956（经 current_versioned_params）、`src/bin/web_server.rs`（从同一 toml 提取扩展字段）透传完成
- [x] T006 [INFRA] `cargo check` 全绿；全仓 `rocksdb://` 审计——src/ 下仅剩 helper 本体、cli_modes 单测常量、无关的 file:// URL 判断
- [x] T007 [INFRA] 冒烟通过（fork release 3.3.0-nightly，8031 端口 versioned 实例）：VERSION $t1 读到旧值(BORE 150/sesno 1)、硬 DELETE 后 VERSION $t1 仍可查删除前记录；8032 非 versioned 对照实例 VERSION 查询报 "The underlying datastore does not support versioned queries"（预期）

**Checkpoint**: 新建实例具备版本化能力，存量实例不受影响。

---

## Phase 2: M2 — sesno_version_anchor 锚点表（US2，P1）

**Goal**: 增量/全量落库完成后固化 sesno→时间戳锚点；失败不写。

**Independent Test**: 跑一次 incremental-sesno，锚点表出现 (dbnum, sesno) 记录且时间戳晚于本批全部写入;人为制造落库失败,无新锚点。

- [ ] T008 [US2] 锚点表 schema 定义：`DEFINE TABLE sesno_version_anchor SCHEMAFULL` + 字段 dbnum(int)/sesno(int)/anchored_at(datetime DEFAULT time::now())/source(string) + UNIQUE INDEX (dbnum, sesno)。放入项目库 schema 初始化路径（与 PE 表同处初始化,确认 `src/versioned_db/database.rs` 或 rs-core 的建表入口）
- [ ] T009 [US2] `src/data_interface/sesno_increment.rs`：`persist_pdms_increment_file/grouped` 成功路径末尾（`flush_increment_upserts` 与 delete flush 全部完成后）UPSERT 锚点 `{dbnum, sesno: report.actual_end_sesno, source: "incremental"}`,复用 `exec_sql_checked` 保证错误传播;任何前序错误直接返回,不触达锚点写入（满足 FR-004）
- [ ] T010 [US2] `src/versioned_db/database.rs`：`sync_pdms` / `parse_single_db_file` 全量完成后写 `source: "full"` 锚点（每个 dbnum 的 latest_sesno）
- [ ] T011 [US2] `src/main.rs`：incremental-sesno 汇总 JSON 增加 `version_anchor` 字段（写入的 dbnum/sesno/anchored_at）,便于 CLI 验证
- [ ] T012 [US2] 验证：db-data 测试实例上跑增量 → SurrealQL 查锚点表核对;再模拟失败路径（如断连）确认无锚点残留

**Checkpoint**: US2 可独立验收（SC-002）。

---

## Phase 3: M3 — rs-core version_query 封装 + history CLI（US1 P1 / US3 P2）

**Goal**: 按 sesno 查询历史快照、时间线、区间 diff 的 CLI 能力。

**Independent Test**: 对 db-data 测试实例执行三个子命令,输出与手工 SurrealQL VERSION 查询一致。

### rs-core 侧（D:\work\plant-code\rs-core）

- [ ] T013 [US1] `src/rs_surreal/version_query.rs` 新建模块 + `mod.rs` 导出：
  - `resolve_anchor(dbnum, sesno) -> Result<Option<AnchorHit>>`（精确命中或"最近不大于"回退,回退时标记 `exact: false`）
  - `HistoryError::Expired` 错误类型：捕获 kvs InvalidArgument（读时间戳低于 GC 水位线）并翻译
- [ ] T014 [US1] `version_query.rs`：`snapshot_at(refno, sesno) -> Result<ElementSnapshot>`——锚点换算后对 pe_key、noun 表、ATT_UDA 发 `SELECT * FROM <target> VERSION $t`,组装 PE+ATT 快照
- [ ] T015 [US3] `version_query.rs`：`diff_range(refnos, from_sesno, to_sesno) -> Result<Vec<ElementDiff>>`——两端快照字段级对比,输出 changed/added/removed/deleted 分类
- [ ] T016 [US1] `version_query.rs`：`timeline(refno, from_sesno, to_sesno) -> Result<Vec<TimelinePoint>>`——列出锚点区间内该元素有变化的 sesno（逐锚点快照 hash 对比,首版可接受 O(锚点数) 查询）

### plant-model-gen 侧

- [ ] T017 [US1] `src/version_management/cli.rs`：`model-version history` 子命令组注册——`snapshot --refno --sesno [--dbnum]`、`timeline --refno --from-sesno --to-sesno`、`diff --refnos <csv> --from-sesno --to-sesno`,均支持 `--json`
- [ ] T018 [US1] `handle_model_version_command` 接三个子命令 → 调 rs-core version_query;`HistoryError::Expired` 输出"该 sesno 历史已超出 retention 窗口,请改用 DuckLake 存档或源文件重扫"
- [ ] T019 [P] [US1] `db-data/verify_versioned_pe_att.surql`：基于 test_version_data.surql 扩展——含锚点表写入、按锚点时间 VERSION 查询、删除元素历史查询三段验证
- [ ] T020 [US1] 验证（SC-001/SC-003/SC-005）：8030 测试实例执行 T019 脚本 + 三个 CLI 子命令,对照手工查询;构造过期时间戳确认 Expired 错误路径

**Checkpoint**: US1 MVP 完成,US3 diff 同批交付。

---

## Phase 4: M4 — 存量站点切换（US4，P2）

**Goal**: 存量站点可安全切换到 versioned 实例。

**Independent Test**: 测试站点全流程切换后当前态一致、增量回归通过。

- [ ] T021 [US4] `specs/022-versioned-pe-att-storage/quickstart.md`：切换手册——新建 versioned 数据目录 → 站点配置改 versioned_storage=true → sync_pdms 重灌 → 确认首条锚点 → 抽样 refno 当前态比对（旧库 vs 新库）→ 切流 → 跑一次 incremental-sesno 回归
- [ ] T022 [US4] `src/web_server/managed_project_sites.rs`：站点编辑接口允许修改 versioned 开关;已初始化站点修改时返回明确提示"需要重灌数据目录"（不静默改参数）
- [ ] T023 [US4] 验证（SC-004）：按 quickstart 对测试站点走全流程,记录抽样比对与增量回归结果到 quickstart 附录

**Checkpoint**: 全部故事可独立验收。

---

## Phase 5: Polish & Cross-Cutting

- [ ] T024 [P] 把七项 grill 决策要点沉淀到 AGENTS.md（versioned 实例约束:同 dbnum 增量串行、retention 语义、锚点是唯一业务入口）
- [ ] T025 [P] `docs/` 或 spec 目录补一页运维说明:磁盘水位观察建议、retention 调整方式（改配置重启即可,无需重建库）、未开 MODEL_KV 站点的磁盘代价警示
- [ ] T026 跑一遍 quickstart.md 全流程终验

---

## Dependencies & Execution Order

- **Phase 1 (M1)** 阻塞全部后续:T001 → T002 → T003/T004/T005（三者并行）→ T006 → T007
- **Phase 2 (M2)** 依赖 Phase 1:T008 → T009/T010（并行）→ T011 → T012
- **Phase 3 (M3)** 依赖 Phase 2（锚点表必须存在）:rs-core T013 → T014/T015/T016 → 本仓 T017 → T018;T019 可与 rs-core 侧并行
- **Phase 4 (M4)** 依赖 Phase 1~3 全部完成
- rs-core 改动（T013~T016）与 plant-model-gen 的 T017 跨仓库,注意 rs-core 先行提交/推送后本仓才能 `cargo update -p aios_core` 拉到

## Implementation Strategy

MVP = Phase 1 + Phase 2 + Phase 3 的 snapshot 路径（T013/T014/T017/T018/T020）。timeline 与 diff（T015/T016）可在 MVP 验收后补齐;Phase 4 面向存量站点,可最后排期。
