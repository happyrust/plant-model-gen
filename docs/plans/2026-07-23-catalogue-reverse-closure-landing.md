# 目录反向波及闭包 —— 落地方案（P1 索引 + P2 shadow 先行）

> 日期：2026-07-23
> 状态：/grill-with-docs 已定案（6 项决策全 A），待实施
> 决策记录：`docs/adr/0011-catalogue-reverse-reference-index-and-closure.md`
> 术语：`CONTEXT.md`「目录反向波及闭包（Catalogue Reverse Impact Closure）」
> 上位方案：`docs/plans/2026-07-23-incremental-model-impact-closure-refactor-plan.md`（本方案是其 Q3/§6 的**目录切片**）
> 依据逆向：`docs/reverse/core_dll_noun_att_model_update.md` §11.2–§14、`docs/reverse/incremental_update_vs_core_dll.md` §4.4

## 1. 目标与定位

闭合当前最大漏判：**目录定义（SCOM 及其下几何/尺寸）被改时，所有引用它的设计实例未被重生成**。落在今天的 `IncrGeoUpdateLog` 接缝，先建反向索引 + shadow 验证，**不改生成目标**（可随时回退）；命名/语义向大计划 `pe_reference_edge` + `ReverseReference` 对齐，零返工。

**节奏（已定 A）**：P1 索引 + P2 shadow 先上；P3 接管等项目 run barrier（specs/027）就绪后按项目级落，本轮**不建**临时 per-db 扇出。

## 2. 已核对的现状事实

- 出向解析齐全：设计实例 ATT 存**原始** `CATR/SPRE/PRTREF`；`resolve.rs::get_or_create_scom_info` / `resolve_desi_comp` 解析到 SCOM；specs/002 有 design→cata 出向闭包。
- `cata_hash`（`pe` 表 per 实例列）= 几何复用键，`CataHashRefnoKV{cata_hash, group_refnos}` 按几何分组——与「引用」正交；resolved SCOM **不**持久化在实例上。
- **缺**：无按 `target_refno` 反查「谁引用了我」的入边索引。
- 写入接缝：`sesno_increment.rs::persist_pdms_increment_grouped`（L930）生成 PE/ATT/`pe_owner` UPSERT → `versioned_db/version_commit.rs::commit_version`（L202）原子提交（AGENTS.md：禁止旁路直写、复用同一 seam）。
- schema 接缝：`pe` 为 `TYPE NORMAL SCHEMALESS`，索引走代码内 ensure（`pe_owner_tree.rs:501` 的 `DEFINE INDEX IF NOT EXISTS … ON TABLE pe …`）。
- CLI 接缝：`model-version` 命令树（`cli.rs`），已有 `backfill-pe-cata-hash`（L239，handler `handle_backfill_pe_cata_hash_command`）为 backfill 子命令范式。
- 跨库难点：CATA dbnum ≠ DESIGN dbnum；`model_gen_debt:[dbnum,sesno]` 假设「变化库=生成库」，SCOM 改动会漏到 design 库——这正是 P3 需要项目 run 的原因。

## 3. 六项决策（/grill-with-docs 定案）

| # | 决策 | 结论 |
|---|---|---|
| Q1 | 边目标语义 | as-written 原始引用目标（纯语法边，= `pe_reference_edge`；多跳靠查询期 BFS） |
| Q2 | 属性覆盖 | 除 OWNER 外全部 Ref/RefList（OWNER 归 `pe_owner`；传播期再滤目录族/PRTREF 特例） |
| Q3 | 版本化 | 随主库 MVCC（先删后插保旧版、删 target 用 tombstone；`read_at` 可反查已删 SCOM） |
| Q4 | ready 门 | 项目级（所有活动 dbnum ready 才允许 P2 反查；backfill 仍 per-dbnum） |
| Q5 | 读 API | 一跳原语分页；传递 BFS / 环 / 深度 / effect 收敛归 P2 expander |
| Q6 | shadow oracle | 暴力全扫描解析比对（实例 resolve SCOM 集 ∩ 索引 BFS 集）；全量重生差分留作 P3 晋升门 |

## 4. P1 形态（仅索引，shadow-safe）

### 4.1 表 `cata_ref_index`（MVCC 版本化，主 PE/ATT 同源）

```text
行: { source_refno, source_dbnum, attribute(规范大写), ordinal, target_refno }
DEFINE TABLE cata_ref_index TYPE NORMAL SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_crx_target ON cata_ref_index FIELDS target_refno;
DEFINE INDEX IF NOT EXISTS idx_crx_source ON cata_ref_index FIELDS source_dbnum, source_refno;
// cata_ref_index_state: { dbnum, ready, row_count, checksum, backfilled_at }
```
= 大计划 `pe_reference_edge` 目录子集；列名对齐，将来收敛为其视图/改名、不重抽取。

### 4.2 新模块 `src/versioned_db/cata_ref_index.rs`
- `ensure_cata_ref_index_schema()`：仿 `pe_owner_tree` 代码内 ensure。
- `extract_ref_edges(source_refno, dbnum, att: &NamedAttrMap) -> Vec<RefEdge>`：抽全部 Ref/RefList（除 OWNER）；RefList 带稳定 `ordinal`；attribute 规范大写。
- `build_replace_by_source_sql(dbnum, source_refno, edges)`：`DELETE cata_ref_index WHERE source_dbnum=.. AND source_refno=..` + `INSERT ..`（先删后插，MVCC 保旧版）。
- 反查读 API（供 P2，一跳分页）：`load_inbound_references(target_refnos, families, cursor, limit)` + `load_outbound_references(sources)`，绑定同一 `read_at`。

### 4.3 写入接缝
在 `persist_pdms_increment_grouped` 的 `commit_version` `apply` 闭包里，对本轮每个 changed source 追加 replace-by-source SQL（与 PE/ATT UPSERT 同批同事务）。删 source→tombstone；删 target→不级联。提交失败 → 索引与 data anchor 一起不推进（复用 Commit Pending）。

### 4.4 backfill / audit CLI（仿 `backfill-pe-cata-hash`）
- `model-version reference-index backfill --dbnum <n>`：扫 `pe`→读 att→`extract_ref_edges`→UPSERT；写 `cata_ref_index_state`（ready + row_count + checksum）；受项目 mutation lock + 固定 data anchor、可 resume。
- `model-version reference-index audit --dbnum <n> [--at <sesno>] --json`：反向索引 vs 全扫描 outbound 抽取做 count/hash 对账；输出 JSON，是 P2 消费前的 ready 门。

## 5. 正确性不变量
1. replace-by-source 与 data anchor **同一 `commit_version` 事务**。
2. attribute 规范大写、RefList `ordinal` 稳定、重复 target 不误去重不同 attr/ordinal。
3. 删 target 不级联（被删 SCOM 仍可反查引用者）。
4. **项目级** ready 门：所有活动 dbnum `cata_ref_index_state.ready` 前，P2 shadow 不得反查。
5. 语义红线：设计实例改自身 `CATR/SPRE/PRTREF` = direct-only，不扇出兄弟；仅「目录定义被改」走反向闭包。

## 6. 分阶段（映射大计划 M2/M3/M4）

- **P1 索引**（= M2 目录子集）：schema ensure + `extract_ref_edges` + 写入接缝 replace-by-source + backfill + audit CLI + `cata_ref_index_state`。
- **P2 shadow**（= M3 shadow）：项目级 ready 后，`expand_catalogue_reverse_targets` 旁路计算——目录定义变更 → 上卷 definition root（复用 `get_or_create_scom_info` owner 逻辑）→ `load_inbound_references` 一跳 + expander BFS（环/深度保护）→ 得引用实例集；与「解析全扫描」oracle 差分（Q6），**不消费、不改生成目标**。
- **P3 接管**（= M4 目录切片，**等项目 run barrier**）：反查目标真正并入生成目标 + 跨库发布按项目级 `model_generation_run` 落；超集降级（热门 SCOM → `FullDb/FullProject`）。
- **P4**：成本阈值调优 + 生产观察窗。

## 7. 验证（遵仓库 CLI+JSON，不跑 cargo test）
- `reference-index audit --json`：随机 source 与全扫描 outbound 抽取一致；`VERSION AT` 前后边集合可复现。
- P2 shadow 差分：解析全扫描实例集 == 索引 BFS 实例集（Q6）。
- 证据：`db-data/*.surql` fixture + `scripts/smoke/model_impact_reference_index.ps1`。
- 必测：① 实例 `CATR` A→B 只重算该实例、兄弟不动；② 改 SCOM-A 尺寸跨 dbnum 反查全部引用实例、DESIGN sesno 不变；③ 多跳 + 环闭包收敛、provenance 可解释。

## 8. 前向兼容与非目标
- 前向兼容：`cata_ref_index` = `pe_reference_edge` 目录子集，将来平滑收敛为其视图/改名。
- 非目标：不含克隆/分布式副本闭包（`DB_Clone::getRelatedElements`，另议）、不做 effect 三态升级、不改 mesh/transform lane 分离、P1/P2 不改生成目标、不建临时 per-db debt 扇出。

## 9. 实施进度（2026-07-23：P1 索引层代码完成，step-1 + step-2）

### 已完成
- **step-1（独立模块 + CLI，可 CLI 自测）**
  - 新模块 `src/versioned_db/cata_ref_index.rs`（已在 `versioned_db/mod.rs` 注册）：`ensure_cata_ref_index_schema` / 纯函数 `extract_ref_edges`（除 `OWNER/CHILDREN/MEMBERS/MEMB/REFNO/ID` 外全 Ref/RefList，RefList 带 `ordinal`，跳过 unset/自环）/ `build_replace_by_source_sql` · `delete_sources_sql` · `insert_edges_sql`（先删后插）/ 一跳读 API `load_inbound_references`（可按属性族过滤）· `load_outbound_references` / `count_index_rows` / `read_state` · `write_state` / `EdgeDigest`（顺序无关 XOR 摘要）。
  - CLI `model-version reference-index {backfill,audit}`（`src/version_management/cli.rs`，仿 `backfill-pe-cata-hash`）。
  - 证据夹具 `db-data/model_impact_reference_index.surql` + `scripts/smoke/model_impact_reference_index.ps1`。
- **step-2（增量写入接缝）**
  - `src/data_interface/sesno_increment.rs::persist_pdms_increment_grouped`：在 `commit_version` 的 `apply` 闭包内按 changed source `replace-by-source`（顶部幂等 ensure schema；deleted 清出边、add/modified `extract_ref_edges` 重写出边；先删段/后插段分属不同 exec 请求）。
  - **红线守住**：`compute_commit_fingerprint` **不含** crx SQL（仿 debt，索引为附属产物、不构成数据版本身份，既有锚点幂等/冲突判定不受影响）；crx 与数据同处 apply 保护域，失败 → pending → recover 重放同一 apply 幂等收敛；删 target 不级联。

### 验证证据
- 编译：`cargo check --no-default-features --features sync-cli` 干净通过（`cata_ref_index` / `sesno_increment` 无 error/warning），无 lint。
- SQL 形态 smoke（临时内存 surreal，HTTP `/sql`）：**19 语句 OK + 10 判定全 PASS**——schema DDL、`INSERT[{id:数组 record id}]`、`DELETE WHERE source`、反查 `target_refno IN`（改 SCOM `200_1` → 反查 `100_1/100_2`）、属性族过滤、出边有序、replace-by-source（`100_1` 改指 `200_9` 后旧目标只剩 `100_2`）、`count() GROUP ALL`、state UPSERT/read。
- 可移植性修正：`count_index_rows` 由 `SELECT VALUE count()`（fork/标准引擎行为不一）改为规范 `SELECT count() … GROUP ALL` + 结构体反序列化，两侧稳。

### 真机 e2e 验证命令（CLI+JSON；`-c` 后接不带扩展名的配置路径，默认 `db_options/DbOption`）
```powershell
# 1) 建立/重建某 dbnum 的反向索引（首建或修抽取器后重跑），写 ready + checksum
aios-database -c db_options/DbOption model-version reference-index backfill --dbnum <N> --json

# 2) 对账：索引 vs 当前 PE/ATT 全扫描抽取（行数 + 内容 checksum + 孤儿行 + 抽样差分），
#    不一致非零退出。step-2 的验收：跑过一次增量的站点直接 audit 应 PASS（seam 已维护）。
aios-database -c db_options/DbOption model-version reference-index audit --dbnum <N> --json

# 3) 增量→索引自动维护验证：对该 dbnum 走一次正常增量（watch-incremental / incremental-sesno），
#    随后再 audit 应仍 PASS（seam 写入 == 全扫描抽取）。
```

### 未做（P1 加固 / P2，按需推进）
- audit `--at <sesno>`（VERSION AT 历史对账，需 P2 读会话）；backfill 项目 mutation-lock + 固定 anchor + resume；**项目级 ready 门的消费**（P2 反查前所有活动 dbnum ready）。
- step-2 目前无 feature 门、每次增量都写索引（shadow-safe，无消费方）；如需可加 kill-switch 开关。
- **P2 shadow**：`expand_catalogue_reverse_targets`（上卷 SCOM definition root → `load_inbound_references` 一跳 → expander BFS 环/深度保护）+ 解析全扫描 oracle 差分，只旁路不消费不改生成目标。

## 10. 实施进度（2026-07-24：P2 expander 核心已落，shadow 接入/oracle 待续）

### 已完成
- 新模块 `src/versioned_db/cata_ref_closure.rs`（已在 `versioned_db/mod.rs` 注册）：
  - `expand_reverse_closure(seeds, limits, lookup)`：**纯 BFS 反查闭包**，注入 `lookup`
    便于无 DB 单测；`ReverseClosureLimits{max_depth, max_instances, per_hop_limit}` 三重
    保护；`ReverseClosureResult{instances, depth_reached, truncated_depth, truncated_size,
    visited_count}`；seed 剔除 + `visited` 环保护 + `sort_by_key` 确定性。
  - `expand_catalogue_reverse_targets(seeds, families, limits)`：**async 真机入口**，逐跳
    `load_inbound_references` → `source_refno.parse::<RefU64>()` 回解续跳；与纯 BFS 共用
    `ClosureAccumulator`（去种子/环/规模逻辑单点收敛）。
  - 单测 5 例（`cargo test --lib versioned_db::cata_ref_closure::` 全过）：多跳收敛、
    环有界+去种子、深度截断、规模截断（超集降级信号）、空 seeds。
- **红线守住**：本模块**不 import/不触碰** `IncrGeoUpdateLog`/生成目标，纯计算+返回，
  与 effect 分类器（ADR-0009）正交。effect `needs_dependency_redirect()` 作为将来挑
  seeds 的门（DependencyCascade/Structural），本轮未接入热路径。

### 未做（下一步）
- **增量 shadow 接入**：在增量链路挑「目录定义被改」seeds（noun/CATA 库判定 + effect
  `needs_dependency_redirect`）→ 项目级 ready 门校验 → 调 `expand_catalogue_reverse_targets`
  → 仅日志/落 shadow 表，**不改生成目标**。
- **oracle 差分（Q6）**：解析全扫描实例集 vs 索引 BFS 实例集 的对账 harness。
- **P3 接管**：等 specs/027 项目 run barrier；届时反查目标并入生成目标 + 跨库项目级发布 +
  热门 SCOM 超集降级。
