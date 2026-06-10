# Feature Specification: 按需解析元件库（CATA）— refno 级引用闭包

## User Need

解析期不再整库解析 CATA（元件库 / 规格库 / paragon）。当前 `DEFAULT_DATA_SYNC_DB_TYPES = ["DESI","CATA"]` 把 CATA 逐页全解析，但实际只有被 DESI 模型引用到的一小部分元件会被使用，浪费大量 I/O / 内存 / 时间。需要：从 DESI 出向引用起种，沿引用关系做 **refno 级传递闭包**，只解析"本模型真正用到的" CATA 元素集合。

## Scope

- 解析域（`aios-database` 侧，与 `src/data_interface/db_index.rs` 同处）的 CATA 解析裁剪。
- refno 级引用闭包：种子收集、跨库 BFS 扩展、容器子树 / owner 链纳入、终止 / 去重。
- CATA 元素的 by-refno 部分解析能力。
- 运行期惰性兜底 + 离线校验，保证零漏边。
- 闭包结果持久化与增量复用。

## Background：模型生成的引用链（要遍历的图）

单个 DESI 几何在生成期走的链（见 `src/fast_model/gen_model/resolve.rs` 的 `resolve_desi_comp` / `get_or_create_scom_info`）：

```
DESI ─SPRE→ SPCO ─CATR→ SCOM ─┬─ ->GMRE ->GSTR → GMSE(正几何集) → 原语(SBOX/SCYL; SPRO→SPVE)
                              ├─ NGMR → 负几何集(开洞)
                              └─ PTRE / PSTR → PTSE/PSET → PTAX(连接点) / PLIN(板轮廓点)
```

spec 选择器（SELE/SPCO 按口径 `HBOR ∈ [ANSW,MAXA]`）在生成期动态选（见 `resource/surreal/common.surql`）。

## 交叉验证（IDA Pro / core.dll，已实测）

经 `user-ida-pro-mcp` 对 `D:\AVEVA\Everything3D2.10\core.dll` 实测确认（详见 plan.md）：

- **Schema 真实存在**：`ATT_CATR/SPRE/GMRE/GSTR/NGMR/PTRE/GEOM` 等属性与 `NOUN_GMSE/SCOM/SPCO/PTSE` 等名词均为命名全局对象（印证边集真实）。
- **边顺序**：设计→元件库先 `ATT_SPRE` 后 fallback `ATT_CATR`（与 `get_cat_refno` 一致）。
- **访问机制**：前向 `getElement(elem,ATT_X)` + 反向 `DB_RefTableIterator(DB,ATT_X,elem)` 的每属性 RefTable 索引，叠加 `DGOTO` 惰性导航 —— 原始引擎从不整库解析，**与 by-refno 闭包同构**。
- **额外边**：发现 `XGMREF/UDGEOM/TGEOM/PSPREF/GEOM`（辅助/用户几何），印证"跟所有出向 RefU64"优于白名单。
- **待续**：`SCOM→GMRE→GSTR→GMSE→原语` 的具体构建在 delay-load 的 `libgeom.dll`，Task 9 续。

## Requirements

1. 提供 CATA 元素的 **by-refno 部分解析**：给定 dbnum + refno 集合，仅解析这些元素（基于 B+树 `index_map` 页定位），不整库解析。
2. **种子收集**：解析 DESI 后，取所有出向 `RefU64`（`RefU64Type/RefU64Array`）中目标落在 CATA 类型库的引用作为闭包种子。
3. **闭包扩展**：对每个到达的 CATA 元素，跟随其所有出向 `RefU64`（db_type 收口至 CATA），并对"容器名词"（`GMSE/NGMS/PTSE/PSTR/SPRO/SELE/SPCO`）纳入整棵 tree 子树，对每个到达节点纳入 owner 祖先链到库根。
   - 备注（IDA 实测）：几何边除 `GMRE/GSTR/NGMR/PTRE` 外还有 `XGMREF/UDGEOM/TGEOM/PSPREF/GEOM` 等；"跟所有出向 RefU64" 可自动覆盖，无需逐一枚举（这正是不选白名单的原因）。
4. **spec 选择器**：闭包到达 `SELE/SPEC` 时纳入其全部 `SPCO` 子树（及各自 `→CATR→SCOM` 闭包），选择留给生成期。
5. **跨库 / 去重 / 终止**：frontier 按 dbnum 聚合（每库 `index_map` 只打开一次），`visited` 按 refno 去重并防环；frontier 空即终止。`cata_hash` 不参与解析去重。
6. **持久化 + 增量**：闭包结果（各 dbnum 的 refno 集）持久化；DESI 子树变更时只重算 delta 种子并并入。
7. **运行期惰性兜底**：生成期若需要未解析的 CATA refno，即时部分解析其小闭包并存入 pe，记录 miss（复用 `cache_miss_report`）。
8. **离线校验**：可对同一 DESI 集对比"整库解析 vs 按需闭包"的生成结果差异，量化覆盖率与漏边。

## Non-Goals

- 不在解析期复刻 spec 口径选择逻辑（留生成期）。
- 不改变生成期几何复用（`cata_hash`）机制。
- 不解析与几何无关的库（DICT/SYST 仍按既有策略）。
- 不跑 Rust test / 不编译 test 目标（仓库规则）。

## Acceptance Criteria

- 对 AvevaPlantSample 单 DESI 库：按需闭包后解析的 CATA 元素数 ≪ 整库元素数，且生成结果（inst_relate / 几何 hash）与整库解析**逐元素一致**（校验模式 diff 为空或仅落在已知 R2 残余）。
- 闭包对任意"目标落在 CATA 库的出向引用"零漏种、零漏边（除表达式按名引用，由惰性兜底覆盖）。
- 运行期遇未解析 refno 时能惰性补齐并产出几何，不静默缺模型；miss 写入报告。
- 闭包结果持久化，二次运行 / 增量更新可复用。
