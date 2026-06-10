# Tasks

> 落地用 superpowers:executing-plans 按序执行。每个 Task 后跑 `cargo check -q`（不跑 test）。

## 前置

- [x] **T001 解析器 by-refno 能力确认（已完成，读 `d:\work\plant\pdms-io` 源码）**
  - 结论：`parse_pdms_db::parse::parse_file(path, &Option<PdmsDatabaseInfo>, file_name, project)` 第二参数是**数据库信息，不是 refno 过滤位** → bulk parser 整文件解析，不支持 by-refno。
  - **但随机访问原语已齐备**（`pdms-io/src/io.rs::PdmsIO`）：`build_index_map() -> HashMap<RefU64, Vec<RefnoDataLoc>>`（refno→页/偏移）、`get_element_at_session(refno, sesno) -> EleData`（内部 `find_refno_loc` + `parse_element(offset)`）、`get_element_with_history`、`read_element_record_cached(offset)`；另有 `engine_v2` 全套 db1–db5（B 树按 refno 搜索）。
  - → by-refno 部分解析**完全可行、低风险**：`parse_db_refnos` 仅需薄封装（open → build_index_map → 逐 refno 取最新 session 的 `parse_element` → 组装 partial `PdmsDbData`），无需深改解析器。
- [x] **T001b 实现 `parse_db_refnos`（已完成）**
  - 薄封装上述原语（`src/data_interface/cata_closure.rs`）。
  - 实现偏差说明：返回 `HashMap<RefU64, ParsedCataEle>`（闭包跟边所需最小信息：owner/noun/outbound/children），**未组装完整 `PdmsDbData`**——闭包发现 pass 不落库，正式属性解析由 sync 按 manifest 部分解析完成（见 T006b），避免双份属性表驻留内存。

## 闭包引擎

- [x] **T002 出向引用泛化（已完成）**
  - `outbound_refs_of(att) -> Vec<RefU64>`：从 `extract_outbound_ref0s` 下沉到元素级、保留完整 `RefU64`（`src/data_interface/cata_closure.rs`）。
  - 种子：`CataClosureResolver::seed(refs)`；DESI 侧只需对各 DESI 元素跑 `outbound_refs_of` 再 `seed`（`seed_from_design` 薄包装并入 T006 流水线）。
- [x] **T003 BFS 闭包引擎（已完成）**
  - `CataClosureResolver::resolve()`：按 dbnum 聚合 frontier → `parse_db_refnos` 部分解析 → 跟边 → `visited` 去重/防环；`max_rounds` 兜底。
  - 产出 `CataClosureManifest { by_dbnum, seed_count, visited_count, rounds, missing }`。
  - 解耦：`CataDbLocator` trait 抽象 ref0→dbnum / db_type / 文件；`DbIndexStore` 在 `sqlite-index` 下实现它（引擎不强依赖该 feature）。
- [x] **T004 纵向纳入（已完成，实现优于原计划）**
  - 容器子树：发现 `EleData.children`（成员）直接可得 → `ParsedCataEle.children` + `follow_children` 入队（**无需 owner→children 反向索引**）；SELE→全部 SPCO 由 children 自然覆盖（Q5）。
  - owner 链：`ParsedCataEle.owner` 直接入队（`include_owner_chain`，默认开）。
  - db_type 收口：`cata_db_types`（默认 {"CATA"}，可加 "PADD"）。

## 持久化与接入

- [x] **T005 manifest 持久化 + 增量 delta（已完成）**
  - `CataClosureManifest::save_json(path)`（原子写 tmp+rename）/ `load_json(path)`：DTO 把 `RefU64` 落为 `u64`，不依赖其 serde 实现。建议路径 `output/<project>/scene_tree/cata_closure.json`。
  - `merge_from(other)`：增量 delta 合并（DESI 子树变更后并入既有 manifest）。
- [x] **T006a 闭包入口 + DESI 播种（已完成）**
  - `seed_refs_from_design_data(&PdmsDbData) -> Vec<RefU64>`：DESI 全出向引用去重。
  - `resolve_cata_closure_from_design_file(index, project, desi_path, cfg) -> CataClosureManifest`（`sqlite-index`）：解析单个 DESI → 播种 → 跑闭包 → 出 manifest（端到端可运行）。多 DESI 由调用方循环合并。
- [x] **T006b 接入 sync 流水线 Phase2/3（已完成）**
  - 开关：env `AIOS_CATA_CLOSURE_MODE=manifest` 启用（默认 Off=整库解析，零行为变化）；
    manifest 缺失 / 未覆盖某 dbnum → 整库回退（仅告警）。
  - `cata_closure.rs` 新增 sync 接入层：`CataClosureSyncMode` / `default_manifest_path`
    （`output/<project>/scene_tree/cata_closure.json`，与 db_index 同目录）/ `load_sync_filter`
    / `apply_sync_filter` / 前置闭包 pass 编排入口 `run_cata_closure_pass_for_roots`（扫 roots
    下 DESI → 逐库闭包 → merge → 原子落盘；spec Q8 独立前置 pass，sync 只消费产物）。
  - `versioned_db/database.rs` 三个解析入口统一接入过滤：`sync_total_async_threaded` /
    `sync_total_async_threaded_with_callback` / `parse_single_db_file`（`apply_sync_filter`
    在 `all_refnos` 收集后裁剪，仅对 CATA 类型生效）。
  - 配套修正：db_meta 的 `ref0s` 改为基于整库 `refno_table_map` 收集（原 tree_nodes 口径会随
    部分解析缩水 ref0→dbnum 映射；顺带过滤 0/0x8000_0001 哨兵）。
  - `DEFAULT_DATA_SYNC_DB_TYPES` 保持 `["DESI","CATA"]` 不变：部分解析靠 refno 裁剪实现，
    CATA 仍需进解析循环以产出 .tree/db_meta。
  - 冷构建验证：`cargo check --lib --no-default-features --features review` 绿（2026-06-10）。

## 正确性安全网

- [ ] **T007 运行期惰性兜底**
  - `ensure_cata_refno_parsed(refno)`：命中未解析 → 即时 `parse_db_refnos` 小闭包 → 存 pe → 记 `cache_miss_report`。
  - 接入 `resolve.rs`（`get_or_create_scom_info` / `resolve_desi_comp` 命中未解析路径）。
- [ ] **T008 离线校验模式**
  - `verify_cata_closure(dbnums)`：整库解析 vs 按需闭包，diff 生成结果（inst_relate / 几何 hash），写 `output/<project>/cata_closure_verify.json`。
  - 接 CI/灰度门禁。

## IDA 交叉验证

- [x] **T009a core.dll 复核（已完成，live IDA：`D:\AVEVA\Everything3D2.10\core.dll`）**
  - 确认 Schema 全局（`ATT_CATR/SPRE/GMRE/GSTR/NGMR/PTRE/GEOM`、`NOUN_GMSE/SCOM/SPCO/PTSE` 等）、边顺序（SPRE→CATR）、前向+反向 RefTable 访问机制（`getElement` / `DB_RefTableIterator`）。
  - 新发现额外几何边 `XGMREF/UDGEOM/TGEOM/PSPREF/GEOM` → 已回填 spec/plan（强化"跟所有出向 RefU64"决策）。
  - 结论：geometry 遍历不在 core.dll（在 delay-load `libgeom.dll`）。
- [ ] **T009b libgeom.dll 几何遍历复核（需在 IDA 加载 `libgeom.dll`）**
  - 反编译 `SCOM→GMRE→GSTR→GMSE→原语` 的构建链，确认是否总跟 `NGMR/PTRE/PSTR` 及容器子树展开方式。
  - 确认几何表达式是否经 `DTAB/CATREF` 等**按名引用**其它元素（R2 残余 → 决定惰性兜底 vs 名字预扫）。
  - 回填 T002/T004 跟边规则与 `container_subtree_nouns`。

## 收尾

- [ ] **T010 文档与风险**
  - 更新 spec/plan；记录 R1~R5 残余与跳过的 test。
  - 校验报告（AvevaPlantSample 单库）：解析元素数下降比 + diff 结果。
