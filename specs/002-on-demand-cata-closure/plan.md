# Implementation Plan

## Approach

把 `db_index.rs` 已有的 **dbnum 级**引用闭包（`extract_outbound_ref0s` + `resolve_related_closure`）下沉到 **refno 级**。独立前置 pass：全量解析 DESI → 从 DESI 出向引用起种 → 跨 CATA 库 BFS 闭包 → 只 by-refno 部分解析命中的 CATA 元素。运行期惰性兜底 + 离线校验保证零漏边。落点在解析域（`aios-database`），符合 sidecar 边界规则。

## 决策依据（grill-me Q1~Q8）

| # | 决策 | 结论 |
|---|------|------|
| Q1 | 粒度 | 元件级部分解析（需解析器 by-refno 能力） |
| Q2 | 跟边 | 全出向 RefU64 + db_type 收口 + 容器子树 + owner 链 |
| Q3 | 种子 | DESI 全出向 RefU64 → CATA 收口 |
| Q4 | 纵向 | 命中 + owner 链到库根 + 仅容器子树（owner 可开关） |
| Q5 | 选择器 | 命中 SELE/SPEC 纳入全部 SPCO，选择留生成期 |
| Q6 | 跨库/去重 | refno visited + 按 dbnum 聚合 frontier；cata_hash 不参与 |
| Q7 | 兜底 | 闭包 + 运行期惰性兜底 + 离线校验 |
| Q8 | 落点 | 独立前置 pass，置 aios-database 解析域，结果持久化 + 增量 |

## 边模型

- **横向边**：元素属性里所有 `RefU64Type/RefU64Array`，目标经 `db_index` 的 ref0→dbnum 映射收口到 CATA 类型库。
- **纵向边**：到达"容器名词"（`GMSE/NGMS/PTSE/PSTR/SPRO/SELE/SPCO` 的 `db1_hash`）时按 owner→children 展开整棵子树；每个到达节点纳入 owner 祖先链到库根。子节点可由 `index_map`（refno→owner）派生，无需解析属性。
- **去重 / 终止**：`visited: HashSet<RefU64>` + frontier 空终止。

## 复用的现有基础设施

- `pdms_io::PdmsIO::{open, build_index_map, get_latest_sesno}` — index-only 随机访问（refno→page），不解析属性。
- `src/data_interface/db_index.rs`：`DbIndexStore`（ref0→dbnum / db_type / 文件）、`extract_outbound_ref0s`（→ 泛化到元素级）、`resolve_related_closure`（dbnum 级 → 下沉 refno 级）。
- `src/fast_model/gen_model/cache_miss_report.rs` — "跳过+记录" 模式，复用于兜底/校验。
- `cal_cata_hash()` / `build_cata_hash_map_from_tree` — 生成期几何复用（与本闭包正交）。

## IDA / core.dll 交叉验证

> 验证对象：`D:\AVEVA\Everything3D2.10\core.dll`（E3D 2.10），经 `user-ida-pro-mcp` 的 `py_eval`（IDAPython）实测。

### 已确认（live IDA）

1. **Schema 全部为一等 DB 对象**（命名全局，地址实测）：
   - 属性：`ATT_CATR`@0x10f624ac、`ATT_SPRE`@0x10f61d0c、`ATT_GMRE`@0x10f6287c、`ATT_GSTR`@0x10f62b70、`ATT_NGMR`@0x10f607e0、`ATT_PTRE`@0x10f61bf8、`ATT_GEOM`@0x10f643c0，外加 `ATT_PSPREF / ATT_TGEOM / ATT_UDGEOM / ATT_XGMREF`。
   - 名词：`NOUN_GMSE`@0x10f5f86c、`NOUN_SCOM`@0x10f5f05c、`NOUN_SPCO`@0x10f5efbc、`NOUN_PTSE`@0x10f5f1d0、`NOUN_XGEOM`。
   - **印证**本设计的边集（GMRE/GSTR/NGMR/PTRE/SPRE/CATR + GMSE/SCOM/SPCO/PTSE）是真实的元件库引用图。
2. **设计→元件库边顺序**：先 `ATT_SPRE`，无则 fallback `ATT_CATR`（`DB_ComparisonSession::isCatmodModified`@0x105a9680 的 `getAtt(ATT_SPRE) else getAtt(ATT_CATR)`）——与 `resolve.rs::get_cat_refno` 一致。
3. **引用访问原语 = 前向 + 反向 RefTable 索引**：
   - 前向：`DB_Element::getElement(elem, ATT_X)` / `getAtt(elem, ATT_X, qual, &out)`（= 我们的 `outbound_refs_of`）。
   - 反向：`DB_RefTableIterator(DB, ATT_X, elem)`、`DB_MDB::findElements(buf, 0, ATT_X, elem)`（每属性 RefTable 支持反查），**与本设计复用的 `db_index` / `refno_assoc_index` 同构**。
   - 结合既有 RE 笔记（`DGOTO/DGETI/DGETF/DBOWNR@0x10541c8c/CLIMBA/GSTRAM@0x100b0c22/TRAVCI@0x10687028`）：引擎全程**惰性按引用导航**，从不整库解析 —— 强支撑 by-refno 闭包。
4. **新发现的额外几何边（原边模型未列）**：`XGMREF` / `NOUN_XGEOM`（辅助/扩展几何）、`UDGEOM`（用户自定义几何）、`TGEOM`、`PSPREF`、`ATT_GEOM`。**直接印证 Q2 选"跟所有出向 RefU64"优于白名单**——白名单只列 GMRE/GSTR/NGMR/PTRE 会漏掉这些。→ 已并入跟边收口规则（见 spec.md Req 3 备注）。

### 待验证（需加载 `libgeom.dll`）

- **几何遍历不在 core.dll**：`core.dll` 仅含 DB schema + 导航层；`libgeom.dll` 为 delay-load（字符串 'libgeom.dll'@0x10af34a0，IAT 项 @0x10af154c）。`SCOM→GMRE→GSTR→GMSE→原语` 的具体构建与"几何表达式是否按名引用 `DTAB/CATREF`"需在 `libgeom.dll` 复核（Task 9 续）。
- 在此之前以源码 `resolve.rs` + 上述 core.dll 已确认事实为权威边模型。

## Files

- 新增 `src/data_interface/cata_closure.rs`
  - `CataClosureManifest` / `CataClosureConfig` / `CataClosureResolver`
  - `outbound_refs_of(att)`、`seed_from_design(desi_data)`、`resolve()`
  - `parse_db_refnos(project, path, refnos)`（若 `parse_pdms_db::parse_file` 不支持 refno 过滤则新增）
  - `ensure_cata_refno_parsed(refno)`（运行期惰性兜底）
  - `verify_cata_closure(dbnums)`（离线校验）
- 改 `src/data_interface/db_index.rs`：暴露/泛化 `extract_outbound_ref0s` 为元素级。
- 改 `src/versioned_db/database.rs`：CATA 由整库解析改为按 manifest 部分解析（Phase2/3 接入）。
- 改 `src/fast_model/gen_model/resolve.rs`：命中未解析 refno → 调 `ensure_cata_refno_parsed` + 记 `cache_miss_report`。
- 持久化：`output/<project>/scene_tree/cata_closure.json`（或并入 `db_index.sqlite`）。

## 流水线

```
Phase0 预扫(已有)  : index-only → db_index.sqlite (ref0→dbnum, db_type)
Phase1 解析 DESI   : 全量
Phase2 闭包(新)    : seed_from_design → resolve() → CataClosureManifest
Phase3 部分解析CATA: 按 manifest.by_dbnum 用 parse_db_refnos 只解析命中 refno → 建 tree/存 pe
Phase4 生成        : gen_all_geos_data，带 ensure_cata_refno_parsed 惰性兜底
```

## Validation

- 静态检查改动路径；`cargo check -q`；改动 Rust 文件跑 `cargo fmt`。
- **不跑 test / 不编译 test 目标**（仓库规则）；web_server 相关用运行 + HTTP/CLI 冒烟。
- 离线校验模式 `verify_cata_closure`：AvevaPlantSample 单库对比整库 vs 按需的生成结果，diff 必须为空（或仅 R2 残余）。
- IDA 验证（Task 9）：live IDA + core.dll 复核边集。

## 风险

- **R1（已关闭，T001）** 解析器 by-refno 能力：`parse_file` 整文件解析（2nd arg 是 db info 非过滤位），但 `PdmsIO::build_index_map()` + `get_element_at_session(refno,sesno)` / `parse_element(offset)` 等随机访问原语齐备 → `parse_db_refnos` 薄封装即可，低风险。
- **R2** 表达式按名引用（`DTAB/CATREF`）非 RefU64 边 → 惰性兜底覆盖 + IDA 复核。
- **R3** SELE 选择器生成期口径来源。
- **R4** owner 链必要性取决于是否导出 CATA tree（默认开，可关）。
- **R5** IDA：core.dll 已复核（T009a 完成）；几何遍历在 `libgeom.dll`，深挖延后到 T009b（需加载该 DLL）。

## 性能对照 core.dll（效率分析）

| 维度 | core.dll | 本实现（现状） |
|---|---|---|
| 访问范围 | 只导航被引用元素 | 只解析被引用闭包 ✅ 同量级 |
| refno→页 | db3 B 树定点 + db1 取页 | `build_index_map()` 全库索引 + `parse_element(offset)` |
| 页缓存 | db1 LRU 页缓存常驻整会话 | **每 dbnum 缓存 `PdmsIO`+index_map（`CataClosureResolver.sessions`），跨 BFS 轮复用页缓存 ✅** |
| 跟边 | 精确按几何需要 | 过近似（全出向+owner+children，零漏边优先） |
| 复用 | 每会话重新导航 | manifest 持久化 + 增量 delta ✅ 优于 core.dll |

- **相对旧的整库解析**：`O(全部元件)` → `O(引用闭包)`，大型共享库约 10–100× 解析量削减。
- **相对 core.dll**：同一效率量级（都只碰被引用的）。差距 #1（跨轮重复 open+建索引、丢页缓存）**已通过会话缓存修复**；残余差距：② 小批量仍 build 整库 index_map（可改 `find_refno_loc` 定点）；③ 过近似多碰一点（有意取舍）。
- **caveat**：以上为算法/IO 复杂度判断，**未冷构建、未 benchmark**；实测数字由 T008 校验模式产出。
