# Tasks: 按版本实时查询模型树（versioned pe_owner）

**Input**: Design documents from `/specs/023-versioned-model-tree-query/`

**Prerequisites**: plan.md、spec.md、research.md（能力验证已完成）、data-model.md、contracts/tree-version-api.md、quickstart.md

**Tests**: 仓库规则不使用 cargo test；每个故事的验证以 smoke 脚本 / HTTP / CLI `--json` 任务显式列出。

**Organization**: 按用户故事分组。注意：US1 与 US2 同为 P1，**US2（写路径）先行**——它是 pe_owner 历史可信的地基；US1（读路径）凭 pe.children 回退路径仍可独立测试，不被 US2 阻塞。

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

**Purpose**: 补齐 research 遗留的确认性验证，锁定实现前提

- [x] T001 在 `db-data/smoke_023_pe_owner_version.surql` 追加"同值重插（in/out 完全一致）"确认用例，并在 `scripts/smoke/pe_owner_version_capability_smoke.ps1` 增加对应判定；重跑确认 C0~C5 基线仍全过（2026-07-19：C8 同值重插 OK 且边完好；research 遗留项已关闭）

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 两条故事线共用的元记录与版本解析基础

- [x] T002 [P] 新增 `src/versioned_db/pe_owner_meta.rs`：`pe_owner_version_meta:<dbnum>` 的 schema DEFINE（幂等）与读写函数（`get_maintained_since` / `upsert_maintained_since` / `create_once_maintained_since`），并在 `src/versioned_db/mod.rs` 挂载（2026-07-19 完成，cargo check 通过）
- [x] T003 [P] 在 `src/web_api/e3d_tree_api.rs` 新增版本解析辅助：`resolve_tree_version(_for_refno)` 调 rs-core `resolve_anchor`（AnchorHit.anchored_at 直接拼 `VERSION d'…'`，无需再走 fn::sesno_version）；`TreeVersionError` 映射 404 `AnchorMissing` / 410 `Expired` / 400 `VersionUnsupported` / 502 `QueryFailed`；`TreeVersionInfo` 信封 + `ResolvedTreeVersion::use_pe_owner()` 数据源分界（2026-07-19 完成，cargo check 通过）

**Checkpoint**: 元记录可读写、版本解析辅助可独立冒烟（手工 HTTP 调一个临时 echo 或直接进入 US2/US1）

---

## Phase 3: User Story 2 - 增量落库同步维护版本化 pe_owner (Priority: P1)

**Goal**: 层级变更与 PE/ATT 同一版本提交批次，pe_owner 历史自本故事上线起可信

**Independent Test**: quickstart Scenario 1——fixture 跑一次含层级变更的增量：当前态双源一致、按增量前锚点 VERSION 查询返回旧层级、提交摘要含边计数

- [x] T004 [US2] `src/data_interface/sesno_increment.rs`：op 循环按 sesno 顺序生成边维护 SQL 进 `mutation_sqls`——Add/Modified 重写 owner 边区间（先删后插，500 行/INSERT 分块）；Deleted 追加 `DELETE pe:<x>->pe_owner; DELETE pe:<x><-pe_owner;`（2026-07-19 完成）
- [x] T005 [US2] `version_commit.rs`：`VersionCommitCounts.pe_owner_rows`（serde default）贯通 ExistingAnchor / preparing / committed / anchor 写读与两张表 schema；`PdmsIncrementPersistStats.pe_owner_rows` 计数进 `expected_counts`（2026-07-19 完成）
- [x] T006 [US2] **实现期设计修正**：增量不写 `pe_owner_version_meta`——增量只重写本批变更 owner，修不了旧二进制时期的陈旧边，由增量打可信标记会产生静默错误历史；可信起点只由 full_reload / rebuild_cli 建立（pe_owner_meta.rs、data-model.md 已同步更新）
- [x] T007 [P] [US2] `pe.rs::save_pe_relates` 改先删后插（PERelateJson 语义改为"完整语句批"，database.rs 两处 sink 同步改为直接拼接执行）（2026-07-19 完成）
- [x] T008 [US2] `database.rs::write_full_version_anchors`：注释固化"PERelateJson 与 PE/ATT 同 sender/sink，join 后才写 full 锚点"时序不变量；锚点成功后 UPSERT meta（source=full_reload，仅 surreal-save 构建，失败降级 warn）（2026-07-19 完成）
- [x] T009 [US2] 契约级验证通过：`scripts/smoke/pe_owner_incr_shapes_smoke.ps1` 精确镜像 T004 语句形态（graph-delete/先删后插/UPSERT 顺序）在 versioned 实例上 10/10 全过（含 t1/t2 历史回溯、删除元素历史快照、保底 children 字段）；**真机端到端（真实 PDMS 源文件 + incremental-sesno --json + pe_owner_rows 摘要核对）留待 M4/T021 在 fixture 站点执行**。附带实测：`ORDER BY record::id(id)[1]` 是解析错误，正确写法 `ORDER BY id VERSION $t`（ORDER BY 在 VERSION 之前），已回填 research/contracts

**Checkpoint**: 增量后 pe_owner 当前态与 pe.children 一致率 100%（SC-004），历史回溯正确

---

## Phase 4: User Story 1 - 指定版本浏览模型树 (Priority: P1) 🎯 MVP 读路径

**Goal**: `/api/e3d/*` 带 `?sesno=` 实时返回该版本的树；不传 sesno 零回归

**Independent Test**: quickstart Scenario 2——两锚点 children 各自正确（US2 未上线时走 `pe_children_fallback` 亦成立）；404/400 语义正确；不传 sesno 与现状一致

- [x] T010 [US1] `get_children` 版本分支：`?sesno=` → `get_children_versioned`（主路径 `SELECT VALUE in FROM pe:<owner><-pe_owner ORDER BY id VERSION d'…'`，回退 `pe.children`；区间扫禁令写入代码注释；children_count 恒 null；响应附 version 信封）（2026-07-19 完成）
- [x] T011 [US1] `fetch_pe_snapshots_versioned`：500/批 `SELECT id, name, noun, owner FROM [keys] VERSION d'…'`（record-list + VERSION 已在 8030 实测：语法通过、已删记录自动缺行）；空名回退 `"{noun} {order+1}"` 与现状对齐（2026-07-19 完成）
- [x] T012 [US1] `get_node` / `get_world_root` 版本分支：node 于 t 不存在 → `success:false, "Node not found at sesno N"`；world-root 版本模式仅单 dbnum 上下文（synthetic WORL + version 信封）（2026-07-19 完成）
- [x] T013 [US1] `get_site_nodes` / `get_visible_insts` 加 `VersionGuardQuery`、`search_nodes` 加 body sesno 字段——带 sesno 一律 400 `VersionUnsupported`（wrapper + inner 模式，原逻辑零改动）（2026-07-19 完成）
- [ ] T014 [US1] 验证执行 quickstart Scenario 2（两锚点 children 比对、version 信封、404 AnchorMissing、VersionUnsupported、不传 sesno 对照）——**部分完成**：查询语句语义已由 8030 实测（capability/incr_shapes/record-list smoke），代码 `cargo check` 两套 feature 通过；**整机 HTTP 验收需 versioned fixture 站点（真实 db_meta/锚点），与 M4/T021 合并执行**

**Checkpoint**: US1 独立可演示（版本选择器数据源用既有 `/api/model-history/anchors`）

---

## Phase 5: User Story 3 - 祖先链与子树按版本查询 (Priority: P2)

**Goal**: ancestors / subtree-refnos 支持 sesno，复用 US1 的版本解析与数据源选择

**Independent Test**: 移动过父级的元素，移动前后两锚点 ancestors 链路各自正确

- [x] T015 [US3] `get_ancestors` 版本分支：`?sesno=` → `get_ancestors_versioned`（`SELECT VALUE owner FROM pe:<x> VERSION d'…'` 逐级点查上溯，深度上限 20，自指/成环防护，根→父顺序与现状一致）（2026-07-19 完成）
- [x] T016 [US3] `get_subtree_refnos` 版本分支：children BFS 逐层复用 T010 数据源选择，沿用 `include_self`/`max_depth`/`limit`/`truncated` 语义，超限提前止损（2026-07-19 完成）
- [x] T017 [US3] 契约级验证通过：在 8030 versioned 实例对"父级移动"场景实测——`SELECT VALUE owner FROM pe:a VERSION $t2` 返回旧父 `pe:p`、`VERSION $t3` 返回新父 `pe:x`（与 get_ancestors_versioned 查询形态完全一致）；整机 HTTP 核对并入 M4/T021

**Checkpoint**: US1+US3 组成完整版本树浏览能力

---

## Phase 6: User Story 4 - 存量站点接入与历史兜底 (Priority: P2)

**Goal**: 存量 versioned 站点一次性重建接入；上线前历史锚点自动 fallback

**Independent Test**: quickstart Scenario 4——重建后双源一致率 100%；重建前锚点查询走 `pe_children_fallback` 且结果正确

- [x] T018 [US4] `model-version rebuild-pe-owner --dbnum <D> [--batch-size N] [--dry-run] [--json]`：TreeIndex 枚举候选节点 + 批量点查权威 `pe.children` + 每 owner 先删后插重写；成功后 UPSERT meta（rebuild_cli，值=dbnum_info_table latest_sesno，查不到则拒绝）；main.rs 已加 ensure_surreal_connected 门（2026-07-19 完成，cargo check 通过）
- [x] T019 [US4] 契约级验证通过：`pe_owner_version_meta` DDL 幂等 / UPSERT 覆盖 / 读回 / source ASSERT 拒绝非法值均在 8030 实测；`use_pe_owner()` 分界纯逻辑已 review（meta 缺失 → 恒 fallback）。**整机 fallback source 切换核对并入 M4/T021**
- [ ] T020 [US4] 验证执行 quickstart Scenario 4 全流程（升级 → 重建 → 抽样一致 → 老锚点 fallback），把执行记录回填 quickstart——**需 fixture 站点，与 M4 合并执行**

**Checkpoint**: 全部用户故事独立可用

---

## Phase 7: Polish & 端到端验收

- [ ] T021 新建 `scripts/smoke/tree_version_smoke.ps1` 端到端验收：fixture 构造两锚点（增/删/移/改名各至少一例）→ `children`/`ancestors`?sesno 全量比对 → 404/VersionUnsupported/不传 sesno 对照 → fallback source 标注（SC-001/003/005）
- [ ] T022 性能抽样：≥200 子的 owner 版本 children 与深度 ≥10 的 ancestors 计时，核对 SC-002（P95 ≤ 1s）；不达标则按 plan 风险表切 `pe.children` 点查为主路径并复测
- [x] T023 [P] `AGENTS.md` 增补 specs/023 编码守则段：区间扫禁令 + ORDER BY 写法、先删后插、增量同批维护与 counts、可信分界与"增量不写 meta"、错误语义对齐（2026-07-19 完成）
- [ ] T024 spec.md Success Criteria 勾选回填 + research.md 遗留验证项关闭说明

---

## Dependencies & Execution Order

- **Phase 1 → Phase 2**：T001 独立可先行；T002/T003 互相独立 [P]
- **US2（Phase 3）依赖 Phase 2 的 T002**；T004→T005→T006 顺序（同文件），T007 与 T004 并行 [P]（不同文件），T008 依赖 T007
- **US1（Phase 4）依赖 Phase 2 的 T002/T003**；不依赖 US2（凭 fallback 独立成立），但**主路径验收**（source=pe_owner）需要 US2 完成
- **US3（Phase 5）依赖 US1 的 T010**（复用数据源选择）
- **US4（Phase 6）依赖 T002**；T018 与 US1/US3 可并行（不同文件）
- **Phase 7 依赖全部故事**；T023 可随时先行 [P]

### Parallel Opportunities

- T002 ∥ T003；T004 ∥ T007；T018 ∥（T010~T016）；T023 随时
- US1 与 US2 可由两人并行（读侧先以 fallback 验收，US2 合入后补 pe_owner 主路径验收）

## Implementation Strategy

**MVP**：Phase 1~2 → US2（写路径地基）→ US1（读路径）→ 停下用 quickstart Scenario 1+2 验证 → 再推 US3/US4 → Phase 7 收口。
单人执行直接按 T001→T024 顺序走即可（顺序已按依赖排好）。
