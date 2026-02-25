# Full Noun CATE 离线生成（PrefetchThenGenerate）Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在 Full Noun 模式下落地 “Prefetch 批量缓存 → Generate 只读缓存” 的 CATE 链路，使生成阶段不再回查 SurrealDB（cache miss 跳过并记录）。

**Architecture:** 将 CATE 生成拆为两类缓存：1) `geom_input_cache` 缓存每个实例 refno 的 inst_info 相关字段（attmap/world_transform/owner/visible）；2) `cata_resolve_cache` 缓存按 `cata_hash` 复用的 prepared geos/ptset。Prefetch 阶段负责批量填充两类缓存；Generate 阶段仅从缓存读取并组装 `ShapeInstancesData`，miss 写入 `output/<project>/cache_miss_report.json`。

**Tech Stack:** Rust、tokio、foyer HybridCache、rkyv payload、TreeIndex（.tree 文件）

---

## 约束与约定（必须遵守）

- **ref0 != dbnum**：任何按库分桶/目录/缓存 key 均必须使用 `db_meta_info.json` 映射得到的 dbnum；禁止用 ref0 兜底。
- 缓存根目录必须使用 `DbOptionExt.get_foyer_cache_dir()`（即 `DbOptionExt.foyer_cache_dir` 指定目录）。
- 离线语义：Generate 阶段（且 `CacheRunMode != Direct`）禁止主动访问 SurrealDB 拉取生成输入；允许保留编排级必要查询（如 `query_mdb_db_nums`），但生成热路径应只读缓存。
- cache miss 策略：跳过并记录，不中断全流程；报告覆盖写且原子写（tmp + rename）。

---

### Task 1: 扩展 geom_input_cache 支持 CATE 的 Prefetch/Load API

**Files:**
- Modify: `src/fast_model/foyer_cache/geom_input_cache.rs`
- Test: `src/fast_model/foyer_cache/geom_input_cache.rs`（现有 roundtrip 测试扩展或新增）

**Step 1: 写一个失败测试（CateInput roundtrip）**

- 目标：`GeomInputBatch` 写入包含 `cate_inputs` 后，`get()` 能读回且字段一致。

**Step 2: 实现 `prefetch_cate_inputs`**

- 行为：对给定 `(dbnum, refnos)`，批量拉取 attmap、批量 cache-first 拉取 world_transform、批量取 owner_type，写入 `GeomInputBatch.cate_inputs`。
- 失败/缺失：跳过单个 refno，并统计 skipped。

**Step 3: 实现 `load_cate_inputs_for_refnos_from_global`**

- 行为：严格按 dbnum 分桶；只读取相关 dbnum 的 batch 合并结果，再按 refnos 过滤（不扫描全库）。

**Step 4: 扩展 `prefetch_all_geom_inputs`（保持兼容）**

- 新增支持 CATE refnos（可新增 v2 函数返回三元组，并保留旧签名 wrapper）。

**Step 5: 跑测试确保通过**

Run: `cargo test -q -p aios_database geom_input_cache -- --nocapture`（若无该 package 名称，则改用 `cargo test -q geom_input_cache -- --nocapture`）

---

### Task 2: 为 cata_resolve_cache 增加全局只读访问（避免每页重复打开）

**Files:**
- Modify: `src/fast_model/foyer_cache/cata_resolve_cache.rs`

**Step 1: 增加 `OnceCell` 全局管理器与 init/get API**

- `init_global_cata_resolve_cache(cache_dir: PathBuf) -> Result<()>`
- `global_cata_resolve_cache() -> Option<&'static CataResolveCacheManager>`

**Step 2: 编译检查**

Run: `cargo check -q`

---

### Task 3: CATE Processor 在 Generate 阶段走 cache-only 组装

**Files:**
- Modify: `src/fast_model/gen_model/cate_processor.rs`
- Modify: `src/fast_model/gen_model/cache_miss_report.rs`（仅当需要新增 kind/note 约定时）

**Step 1: 增加分支：`ctx.is_offline_generate()`**

- 从 `geom_input_cache` 加载 `CateInput`（cache-only）。
- 从 `cata_resolve_cache` 按 `cata_hash` 读取 `CataResolvedComp`（cache-only）。
- 组装 `EleGeosInfo`（必须设置 `world_transform`、`ptset_map`、`cata_hash`、`visible`、`owner_*` 等）。
- 组装 `EleInstGeosData`（复用 resolved_comp 的 prepared geos；按 `AIOS_RESPECT_TUFL` 过滤）。
- 达到 `SEND_INST_SIZE` 即通过 `sender` 发送。

**Step 2: cache miss 记录**

- `cate_input_miss`：refno 缺少实例级输入。
- `cata_resolve_cache_miss`：cata_hash 无 prepared 结果（整组 refno 记 miss）。
- `cata_hash_map_build_failed`：tree/db_meta 缺失导致无法分组（记 simple miss）。

**Step 3: 非离线（Direct）路径保持原行为**

- 仍可保留现有 `cata_model::gen_cata_instances` 生成逻辑。
- `cata_resolve_cache_pipeline` 预热仅允许在 Prefetch 阶段触发（Generate 阶段禁止）。

**Step 4: cargo check**

Run: `cargo check -q`

---

### Task 4: Full Noun Prefetch 阶段补齐 CATE 缓存预热

**Files:**
- Modify: `src/fast_model/gen_model/full_noun_mode.rs`

**Step 1: PrefetchThenGenerate 阶段预取 CATE inputs**

- 复用 Task 1 的 `prefetch_all_geom_inputs*`（包含 cate_refnos）。

**Step 2: PrefetchThenGenerate 阶段预热 cata_resolve_cache**

- 对 cate_refnos 构建 `target_cata_map = build_cata_hash_map_from_tree(&cate_vec)`（依赖 tree 文件与 db_meta 映射）。
- 调用 `cata_resolve_cache_pipeline::prefetch_cata_resolve_cache_for_target_map(...)` 写入 `foyer_cache_dir/cata_resolve_cache/`。

**Step 3: cargo check**

Run: `cargo check -q`

---

### Task 5: 端到端验证（最小可运行）

**Files:**
- None（必要时新增 `examples/` 或 `tests/`）

**Step 1: Debug 模式跑一次 Full Noun PrefetchThenGenerate**

- 预期：Prefetch 输出包含 cate_inputs cached 数量；Generate 阶段不再打印任何 “get_named_attmap/get_pe/resolve_desi_comp” 相关 DB 查询日志（若有，视为未达成）。
- 预期：若缓存不全，`output/<project>/cache_miss_report.json` 中出现 cate 相关 bucket。

**Step 2: CacheOnly 再跑一次**

- 预期：Generate 阶段命中缓存更多；miss 报告下降。

---

## 已知暂不覆盖（M1）

- CATE 相关 `neg_relate/ngmr` 的离线生成：若现有 DB 路径包含额外关系写入，M1 先不实现，后续按需求补齐（需设计额外输入缓存或单独 prefetch）。
- BRAN-only 管线的 CATE 生成仍可能走 DB 路径（本计划优先 Full Noun 的 USE_CATE）。

