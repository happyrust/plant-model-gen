# 按需解析元件库（CATA）— refno 级引用闭包 设计

> **For Claude:** REQUIRED SUB-SKILL: 落地实现时用 superpowers:executing-plans 按 Task 逐个执行。

**Goal:** 解析期不再整库解析 CATA。改为：全量解析 DESI → 从 DESI 出向引用起种，沿引用关系做 **refno 级传递闭包**，只部分解析"本模型真正用到的" CATA 元素集合，并以"运行期惰性兜底 + 离线校验"保证零漏边。

**Why:** 元件库（catalogue/spec/paragon）通常远大于实际被引用的子集；当前 `DEFAULT_DATA_SYNC_DB_TYPES = ["DESI","CATA"]` 把 CATA **整库逐页全解析**，浪费大量 I/O/内存/时间。

---

## 1. 决策汇总（grill-me Q1~Q8）

| # | 决策点 | 结论 |
|---|--------|------|
| Q1 | 解析粒度/落地方式 | **A 元件级部分解析**（按 `index_map` 页位只解析闭包内 refno）。前置：确认/扩展解析器 by-refno 能力 |
| Q2 | 闭包跟边策略 | **C 跟所有出向 RefU64 + db_type 收口 + 容器子树 + owner 链**；表达式按名引用风险留 Q7 |
| Q3 | 种子边范围 | **B DESI 全出向 RefU64 → CATA 收口**（复用 `extract_outbound_ref0s` + `db_index`） |
| Q4 | 纵向边界 | **B 命中 + owner 链到库根 + 仅容器节点整棵子树**（owner 链可设开关） |
| Q5 | spec 选择器 | **B 命中 SELE/SPEC 即纳入全部 SPCO 子树**，选择留生成期；几何主路径多为具体 SPRE→CATR 已覆盖 |
| Q6 | 跨库/终止/去重 | **A refno 级 visited 去重 + 按 dbnum 聚合 frontier**；`cata_hash` 不参与解析去重（仅生成期几何复用） |
| Q7 | 漏边兜底/校验 | **B 闭包 + 运行期惰性兜底 + 离线校验模式**（纵深防御，复用 `cache_miss_report`） |
| Q8 | 流水线落点 | **A 独立前置闭包 pass**，置于 `aios-database` 解析域（与 `db_index.rs` 同处），结果持久化 + 增量 delta |

---

## 2. 引用边模型（要遍历的闭包图，结合模型生成逻辑）

模型生成时单个 DESI 几何走的就是这条链（`resolve_desi_comp` / `get_or_create_scom_info`）：

```
DESI ──SPRE──> SPEC/SELE ──(生成期按 HBOR 选)──> SPCO ──CATR──> SCOM
                                                              │
   ├─ ->GMRE ->GSTR ─> GMSE(正几何集) ─> 子原语 (SBOX/SCYL/…; SPRO->SPVE)
   ├─ NGMR ─────────> 负几何集（开洞）
   └─ PTRE / PSTR ──> PTSE/PSET ─> PTAX(连接点) / PLIN(板轮廓点)
```

- **横向边**：元素属性里所有 `RefU64Type/RefU64Array`（Q2=C 全跟，db_type 收口到 CATA）。
- **纵向边（子树）**：到达"容器名词"时按 owner→children 展开整棵子树——`GMSE/NGMS/PTSE/PSTR/SPRO/SELE/SPCO`（`query_gm_params` 就是遍历 GMSE 的子节点取原语）。
- **owner 链**：到达节点向上补到库根（保 tree 连贯，消除孤儿节点）。
- 子节点可由 B+树 `index_map`（refno→owner）派生，**无需解析属性**即可枚举。

---

## 3. 复用的现有基础设施

- `pdms_io::PdmsIO::{new,open,build_index_map,get_latest_sesno}` — index-only 随机访问（refno→page），**不解析属性**。
- `src/data_interface/db_index.rs`：
  - `DbIndexStore`（`ref0→dbnum`、`dbnums_by_type`、`file_by_dbnum`、依赖边/闭包）；
  - `extract_outbound_ref0s(&PdmsDbData)`（出向引用抽取，现为一跳、ref0 级 → 泛化到元素级）；
  - `resolve_related_closure`（**dbnum 级** BFS —— 本设计把它下沉到 **refno 级**）。
- `cal_cata_hash()` / `CataHashRefnoKV` / `build_cata_hash_map_from_tree` — 生成期几何复用（与本闭包正交）。
- `cache_miss_report.rs` —"跳过+记录、不中断"模式，复用于 Q7 兜底/校验。

---

## 4. 核心数据结构与函数签名（新增于 `src/data_interface/cata_closure.rs`）

```rust
/// 闭包结果：每个 CATA dbnum 需解析的 refno 集合 + 统计。持久化用。
pub struct CataClosureManifest {
    pub by_dbnum: BTreeMap<u32, BTreeSet<RefU64>>,
    pub seed_count: usize,
    pub visited_count: usize,
    pub rounds: usize,
}

pub struct CataClosureConfig {
    pub include_owner_chain: bool,             // Q4，默认 true
    pub container_subtree_nouns: HashSet<u32>, // GMSE/NGMS/PTSE/PSTR/SPRO/SELE/SPCO 的 db1_hash
    pub max_rounds: usize,                      // 防御性上限
}

/// refno 级 CATA 闭包引擎（BFS）。
pub struct CataClosureResolver<'a> {
    index: &'a DbIndexStore,
    cfg: CataClosureConfig,
    visited: HashSet<RefU64>,
    frontier_by_db: HashMap<u32, Vec<RefU64>>,
}

impl<'a> CataClosureResolver<'a> {
    pub fn new(index: &'a DbIndexStore, cfg: CataClosureConfig) -> Self;
    /// Q3=B：DESI 出向 RefU64 ∩ CATA 类型库 → 种子。
    pub fn seed_from_design(&mut self, desi_data: &PdmsDbData);
    /// Q1/Q2/Q4/Q5/Q6：每轮按 dbnum 聚合 frontier，index_map 定位页 → 部分解析
    /// → outbound + 容器子树 + owner 链 → 入队，直至 frontier 空。
    pub async fn resolve(&mut self) -> anyhow::Result<CataClosureManifest>;
}

/// Q1=A 关键能力：单库按 refno 子集部分解析（若 parse_file 不支持则新增）。
pub async fn parse_db_refnos(project: &str, path: &Path, refnos: &[RefU64])
    -> anyhow::Result<PdmsDbData>;

/// 元素级出向引用（把 db_index 私有逻辑泛化）。
pub fn outbound_refs_of(att: &NamedAttrMap) -> Vec<RefU64>;

/// Q7 运行期惰性兜底：命中未解析 CATA refno 时即时解析其小闭包并存 pe + 记 miss。
pub async fn ensure_cata_refno_parsed(refno: RefnoEnum) -> anyhow::Result<()>;

/// Q7 离线校验：整库解析 vs 按需闭包，diff 生成结果（inst_relate/几何 hash）。
pub async fn verify_cata_closure(dbnums: &[u32]) -> anyhow::Result<ClosureVerifyReport>;
```

持久化：`output/<project>/scene_tree/cata_closure.json`（或并入 `db_index.sqlite` 新表），供增量复用。

---

## 5. 流水线（Q8=A）

```
Phase0 预扫(已有)  : index-only → db_index.sqlite (ref0→dbnum, db_type)
Phase1 解析 DESI   : 全量(模型范围必须全)
Phase2 闭包(新)    : seed_from_design → resolve() → CataClosureManifest
Phase3 部分解析CATA: 按 manifest.by_dbnum 用 parse_db_refnos 只解析命中 refno → 建 tree/存 pe
Phase4 生成        : gen_all_geos_data，带 Q7 ensure_cata_refno_parsed 惰性兜底
```

增量：DESI 子树变更 → 只对变更子树重算 delta 种子，并入既有 manifest。

---

## 6. 落地任务拆解

- **Task 1 — 解析器 by-refno 部分解析**：确认 `parse_pdms_db::parse::parse_file` 第二参数是否为 refno 过滤；否则基于 `build_index_map()` 新增 `parse_db_refnos`。*前置依赖。*
- **Task 2 — 出向引用泛化**：`outbound_refs_of(att)` + `seed_from_design`（把 `extract_outbound_ref0s` 下沉到元素级，保留 refno）。
- **Task 3 — 闭包引擎**：`CataClosureResolver::resolve` BFS（跨库聚合 + index_map + 部分解析 + visited 防环）。
- **Task 4 — 纵向纳入**：容器子树（owner→children 派生）+ owner 链 + SELE 全 SPCO（Q4/Q5）。
- **Task 5 — manifest 持久化 + 增量 delta**。
- **Task 6 — 接入 sync Phase2/3**：CATA 由整库改为按 manifest 部分解析（改造 `DEFAULT_DATA_SYNC_DB_TYPES`/CATA 解析分支）。
- **Task 7 — 运行期惰性兜底**：`ensure_cata_refno_parsed` 接入 `resolve.rs` 命中未解析路径 + `cache_miss_report`。
- **Task 8 — 离线校验模式**：`verify_cata_closure` + CI/灰度门禁。

每个 Task 完成后跑 `cargo check -q`（按仓库规则：不跑 test；web_server 用运行+HTTP/CLI 冒烟）。

---

## 7. 风险与未决

- **R1 解析器 by-refno 能力**（Task1 确认）——若不支持需先补，否则 A 退化为"缩小整库范围"。
- **R2 表达式按名引用**（`DTAB/CATREF` 非 RefU64 边）——闭包抓不到，靠 Q7 运行期惰性兜底 + 校验量化。
- **R3 SELE 选择器口径来源**——生成期按设计 bore 选 SPCO，解析期全纳入（Q5=B）。
- **R4 owner 链必要性**——取决于是否导出 CATA tree / 是否有按 owner 查询；默认开，可关。
