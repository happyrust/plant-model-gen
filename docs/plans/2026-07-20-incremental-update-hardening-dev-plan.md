# 增量更新加固开发计划（2026-07-20）

> 依据：2026-07-20 对当前增量更新实现的代码审核（specs/022 Version Commit seam / specs/023 pe_owner 边）。
> 审核结论：seam 本体（lease + Commit Pending + fingerprint + 不可变锚点）设计成立且有 SurrealQL 实测证据；
> 主要风险集中在 **CLI `watch-incremental` 起点语义** 与 **seam 缺少连续性兜底** 两处，其余为中低优先级打磨。

## 0. 范围与非目标

**范围**：增量写路径（采集 → `commit_version` → 锚点）、三类入口（CLI `incremental-sesno` / CLI `watch-incremental` / web `/api/incremental/*` 与 `/api/db-status/update`）、服务内 watcher（`async_watch` / `startup_catchup`）。

**非目标**：
- 不改锚点/历史读语义（`resolve_anchor`、`model-version history *`）；
- 不改增量模型生成管线内部（orchestrator Incremental scope 现状可用）；
- 不引入自动 `--recover-pending`（恢复保持人工，ops-notes 约束不变）。

**验证约束**（AGENTS.md）：不使用 `cargo test`；aios-database 用 CLI + `--json` 验证，web_server 起服务后 HTTP POST 验证；SurrealQL 断言走 `db-data/*.surql` + `scripts/smoke/*.ps1` 既有模式。

---

## M1（P0）：`watch-incremental` 起点与失败语义修复

### T1 起点改为 Committed Watermark（修高-1）

- **现状**：`src/main.rs` watch 循环用 db_index 的 `latest_sesno`（文件最新 sesno）做 baseline，并把上一轮文件 latest 当 `from_sesno`（L3240-3251、L3307）。违反 AGENTS.md/CONTEXT.md「增量起点只能取 Committed Watermark」硬约束；停机期间的区间被静默跳过并被后续锚点覆盖。
- **改法**：
  - 删除 `baselines` 起点职责：每轮对候选 dbnum 先 `committed_watermark(dbnum)`，`watermark == 0` 跳过（从未全量解析）；`file_latest <= watermark` 跳过；否则 `from_sesno = watermark`、`to_sesno = file_latest`。
  - db_index 扫描仍用于**发现文件与 file_latest**（cheap 探测），不再充当起点记忆。
  - 附带收益：watch 启动后第一轮天然补齐停机缺口（与 `startup_catchup` 语义一致，但走同一循环，无需额外开关）。
- **验收**：
  1. 造库：全量解析到 sesno N，停 watch，源文件推进到 N+k；启动 `watch-incremental --once --json` → summary 的 `from_sesno == N`、锚点固化到 N+k；
  2. `sesno_version_anchor` 链上新锚点 `from_sesno == 前一锚点 sesno + 1`（surql 断言）；
  3. 稳态无增长时输出「本轮无 sesno 增长」。

### T2 单库失败不再终止 watch 进程（修高-2）

- **现状**：循环内 `run_incremental_sesno_once(...).await?`（L3325），任一 dbnum 失败（LeaseBusy / PendingCommit / 采集失败）→ 整个 watcher 退出；重启后按旧实现还会跳过失败区间（T1 修掉起点问题后，重启会重试，但进程不应死）。
- **改法**：per-dbnum `match`，失败打印并 `continue`；该 dbnum 起点不推进（T1 后起点=watermark，天然如此）；LeaseBusy / PendingCommit 按「正常竞争/待人工恢复」降级为 info 级提示（与 ops-notes §4.3 对齐）。`--once` 模式下若存在失败，进程以非零码退出并在 JSON 里带 `failures` 数组（供 smoke 断言）。
- **验收**：注入 dbnum A 的 pending（复用 `db-data/verify_022_lease_gate` fixture 手法），watch 循环继续处理 dbnum B 且进程存活；A 恢复后下一轮自动续传。

**产物**：`scripts/smoke/watch_incremental_watermark_smoke.ps1`（覆盖 T1/T2 两条验收路径）。

---

## M2（P0→P1）：seam 连续性兜底 + 存量漏洞审计

### T3 `commit_version` 增加连续性门禁（修中-3）

- **现状**：`validate_request` 只校验 dbnum/区间/指纹非空；任何调用方传错 `from_sesno` 都能锚定带洞的链，水位跳过后洞不可发现。
- **改法**：`commit_while_leased` 在拿到 lease 后读该 dbnum 当前 watermark（锚点 max，回退 legacy——直接复用 `committed_watermark`），规则：
  - `source == Incremental`：要求 `from_sesno <= watermark + 1`（允许重叠重放）且 `to_sesno > watermark`；`from_sesno > watermark + 1` → 新错误 `ContinuityGap { dbnum, watermark, requested_from }`；
  - `source == Full`：豁免（全量锚点是 from==to 的基线重置语义）；
  - recover 路径同规则（pending 阻塞水位推进，恢复时 watermark 未变，天然通过）。
- **兼容性**：门禁只看「当前 watermark vs 新请求」，历史已存在的洞不影响后续正常提交。
- **验收**：CLI 传 `--from-sesno watermark+5` → 显式 `ContinuityGap` 报错；正常续传 / 幂等重放 / `--recover-pending` / full 锚点全部不受影响（`--json` 断言）。

### T4 存量锚点链审计脚本 + 运维口径

- **产物**：`db-data/audit_anchor_continuity.surql`（按 dbnum 输出 `from_sesno != prev.sesno + 1` 的断链清单）+ `scripts/smoke/anchor_continuity_audit.ps1`。
- **运维口径写入 ops-notes**：发现历史洞的修复手段是该 dbnum 全量重灌（锚点不可变，不支持"补洞"式回填），并说明原因。

---

## M3（P1）：删除路径 fingerprint 确定性（修中-4）

### T5 删除语句改为源文件驱动，不再查库内实时状态

- **现状**：`build_increment_delete_statements`（`src/data_interface/sesno_increment.rs` L600）先 `SELECT noun FROM pe:x` 再决定生成几条 DELETE。半次 apply 后重试时 pe 行可能已被删 → 语句集变化 → fingerprint 变化 → `--recover-pending` 报 `RecoveryNotFound`、普通提交被 PendingCommit 挡住，死局需手工清 `version_commit_state`。
- **方案选择**：**保留「fingerprint = SQL 文本」格式不变**，改让 SQL 生成确定化——被删元素的 noun 从源文件解析（`search_latest_refno(refno, Some(delete_sesno - 1))` + `parse_raw_element` 取删除前最后状态；pdms-io 缺口在 `../pdms-io-fork` 补 API）。理由：
  - 不改 fingerprint 版本前缀，已提交锚点的幂等重放完全兼容；
  - 顺带消灭删除路径的 N+1 `SELECT noun` 查询；
  - 备选方案「fingerprint 改逻辑操作 hash（v2 前缀）」影响面大（跨版本幂等重放会误报 FingerprintConflict），不采用。
- **noun 解析不到时的兜底**：生成固定形态语句（仅 `DELETE ATT_UDA` + `DELETE pe`），并把该 refno 记入 summary 的 `delete_noun_unresolved` 列表（可观测，不静默）。
- **验收**：构造含删除的区间，apply 半途注入失败（临时故障开关或断连），确认 `version_commit_state` 进入 pending 后，用同参数 `--recover-pending` 一次成功（fingerprint 匹配）；对照修复前该场景必死。

---

## M4（P1）：counts 校验去虚化 + 口径修正（修中-5）

### T6 apply 后实测计数，替换自证的 `expected_counts`

- **现状**：`expected_counts` 与 apply 返回值是同一份预计算（`sesno_increment.rs` L815-852；full 路径两边都是 `default()`），`CountMismatch` 在所有调用点不可能触发——校验形同虚设。
- **改法**（最小可信集）：apply 闭包末尾对**本批触及的 key**做实测回读并返回实测值：
  - `pe_rows`：`SELECT count() FROM pe WHERE id IN [...]`（按 chunk 聚合，upsert 的 refno 应全部存在）；
  - `delete_count`：同法断言被删 refno 的 pe 行已不存在；
  - `pe_owner_rows`：按 owner 聚合 `count(owner<-pe_owner)` 与 `edge_final` 对比（specs/023 树查询正确性的关键面）；
  - `att/uda/dbnum_info` 保留为记录口径（写入锚点仅作统计，不参与 mismatch），在结构体注释里写明两类口径。
- **口径修正**：删除路径 `att_rows += deleted_sqls.len()-1` 混入 ATT_UDA 删除，拆分为独立统计或并入 `delete_count` 口径，注释说明。
- **成本护栏**：实测回读按 500/chunk，与现有 `exec_statements` 粒度一致；超大批次耗时记入 perf_metrics。
- **验收**：正常增量 counts 全等通过；人为注入错位（临时改一条 UPSERT 目标）→ `CountMismatch` 触发、状态转 pending、锚点未写。

---

## M5（P2）：web 增量入口打磨（修低-6）

### T7 `/api/db-status/update` 与 `/api/incremental/sync|detect` 语义收紧

- 执行时（后台任务内、每个 dbnum 起跑前）**重读** `committed_watermark`，不用受理时的快照（排队期间水位可能已推进，现状会以 FingerprintConflict 假失败收场）；
- 增加 per-dbnum in-flight 守卫：`INCREMENT_RUNS` 中该 dbnum 存在 `queued/running` 时拒绝新触发（409，返回现有 run_id）；
- `target_sesno` 仅在单 dbnum 请求时接受，多 dbnum + target_sesno → 400（sesno 空间 per-db）；
- `force_update` 死字段：从 `IncrementalUpdateRequest` 移除（前端同步），或短期先在 handler 明确忽略并在响应注明；
- LeaseBusy / PendingCommit 归类为 `conflict` 状态而非笼统 `failed`（`IncrementRunStatus.state` 增值），前端水位页展示。

**验收**：起服务后 HTTP POST 场景化验证（重复触发 → 409；多库带 target → 400；pending 存在 → conflict 状态），脚本落 `scripts/smoke/incremental_http_smoke.ps1`。

---

## M6（P2）：watcher 入口一致性 + 文档对齐（修低-7/低-8）

### T8 `startup_catchup` 接入其余 watcher 入口

- `db_model.rs::exec_watcher / spawn_exec_watcher`、`web_server/remote_runtime.rs::start_runtime` 目前只有 `init_watcher + async_watch`，开了 `AIOS_WATCH_STARTUP_CATCHUP` 也不会补齐。统一插入 `startup_catchup()`（失败不阻断启动，与 lib.rs 主路径一致）。
- 注：T1 完成后 CLI watch 已自带补齐；此项针对服务内 watcher。

### T9 文档对齐

- ops-notes §4.6「web 增量面只读化/动作端点 501」→ 更新为方案 B 现状（sync/detect/update 已接 IncrementRun，c26d2faa）；
- AGENTS.md specs/023 段落：「层级边与 PE/ATT 同一 mutation_sqls 批次」→ 改为「同一 fingerprint/counts 覆盖，物理上边删/边插独立请求提交（unique_pe_owner 索引实测约束）」；「删除元素双向清边」→ 写实际实现（显式 `<-pe_owner` + 引擎删记录自动清边，8030 实测）；
- 新增：锚点链审计与"历史洞 → 全量重灌"运维口径（T4 产物）；
- CHANGELOG 记录 M1-M4 行为变化（尤其 T1 的"watch 启动自动补齐"）。

---

## 排期与依赖

| 里程碑 | 任务 | 规模 | 依赖 | 优先级 |
|---|---|---|---|---|
| M1 | T1 watch 起点=watermark | S-M | 无 | P0 |
| M1 | T2 watch 失败隔离 | S | T1 | P0 |
| M2 | T3 连续性门禁 | M | 建议在 T1 后合入（避免旧 watch 触发门禁误报） | P0-P1 |
| M2 | T4 锚点链审计 | S | 无 | P1 |
| M3 | T5 删除 fingerprint 确定性 | M（含 pdms-io-fork API） | 无 | P1 |
| M4 | T6 counts 实测校验 | M | T5（同文件，避免冲突） | P1 |
| M5 | T7 web 入口收紧 | S-M | T3（conflict 语义依赖错误分型） | P2 |
| M6 | T8 startup_catchup 接入 | S | 无 | P2 |
| M6 | T9 文档对齐 | S | 各里程碑落地后收尾 | P2 |

建议节奏：M1+M2 一个批次先行（止血 + 兜底，改动小、消险大）；M3+M4 第二批（同文件 `sesno_increment.rs`，串行避免冲突）；M5/M6 收尾。

## 全局验证清单（每批次合入前）

1. `powershell -File scripts/build-sync-cli.ps1` 瘦构建通过；`cargo check --lib` 干净；
2. `incremental-sesno --json` 基线回归：正常续传 / 幂等重放 / `--recover-pending` / `--no-persist` 试跑；
3. `watch-incremental --once --json` 补齐与失败隔离场景（M1 smoke）；
4. surql 断言：锚点连续性、`version_commit_state` 状态机、pe_owner 边计数（复用/扩展 `db-data/verify_022_*` 与 `scripts/smoke/pe_owner_*`）;
5. web：起服务 HTTP POST 场景（M5 smoke）；
6. 全量解析 → 增量 → `model-version history` 历史读通路不回归（specs/022 quickstart 抽查）。

## 风险与回退

- **T1 行为变化**：watch 启动第一轮会自动补齐停机缺口，大缺口意味着启动后立即出现一次较长增量运行——属预期（watermark 语义），需在 CHANGELOG/运维说明标注；如需旧行为可用 `--dbnum` 收窄或先人工 `incremental-sesno`。
- **T3 门禁误报**：若存量站点存在非常规锚点链（如手工操作产物），门禁可能挡住正常续传——T4 审计脚本先行摸底；紧急回退 = 门禁降级为 warn（加环境变量开关仅用于应急，不默认提供）。
- **T5 依赖 pdms-io-fork**：本仓 `[patch]` 指向 `../pdms-io-fork`，API 增补需同步 push 到 dev-3.1 分支，CI 才能对齐（Cargo.toml L330 注释的双 patch 约束）。
- **T6 性能**：实测回读增加每批一次聚合查询；如实测超预算（perf_metrics 观测），降级为仅校验 `delete_count + pe_owner_rows`。
