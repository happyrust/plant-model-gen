# 增量更新如何判定"哪些需要模型增量生成" —— 当前实现 vs core.dll 逻辑

> 分析对象：`plant-model-gen`（aios-database）当前增量管线
> 对照基准：`docs/reverse/core_dll_noun_att_model_update.md`（AVEVA Everything3D `core.dll` / `Core3D.dll` 逆向，§1–§14）
> 依据代码：`src/data_interface/sesno_increment.rs`、`src/data_interface/increment_record.rs`、
> `src/version_management/model_impact.rs`、`src/fast_model/gen_model/{orchestrator,gen_pipeline,noun_collection}.rs`、`src/pe_transform_refresh.rs`
> 日期：2026-07-22

---

## 0. 结论速览（TL;DR）

- **顶层设计忠实复刻了 core.dll**：当前"要不要重算"由**两道门**决定——① 属性门（被改属性是否影响几何）× ② NOUN 几何门（元素类型是否产生几何）；这正是 core.dll 的 **`wnoevt`(属性级) × `geomset`(NOUN 级)**。增量范围同样按 sesno 区间取，对齐 `elementsChangedBetween`。
- **真相源的取舍差异（已知、见 ADR-0009）**：属性门用一张硬编码"几何影响属性白名单"`attribute_affects_model`，而非 core.dll 的内核字典位 `wnoevt`（`wnoevt` 静态不可导出）。取"宁多勿漏 + 三方交叉校验"兜底。
- **主要缺口在"粒度与波及"**（对比 Core3D `PartialUpdateDesiMgr`）：当前只做 loop 容器→owner 的一处上溯，**缺**通用"显著 owner + Members 展开"和**波及闭包**（CATR/SPRE→引用实例、owner 移动→子实例、克隆/分布式副本）。摆放（POS/ORI）已在**变换缓存层**按子树失效兜底，但元素自身 mesh 仍会被重算。

---

## 1. 当前实现：五阶段管线

### ① 增量采集（sesno 区间 → 操作流）

`sesno_increment.rs::collect_pdms_increment_for_file_with_operations(project, file, cached_sesno, to_sesno, detail)`：

- 打开 E3D/PDMS db 文件（`pdms_io::PdmsIO`），读最新 sesno；请求区间 = `cached_sesno + 1 .. to_sesno`（`to_sesno` 省略则用文件最新 sesno）。`cached_sesno` 即系统已固化到的 `sesno_version_anchor`（specs/022）。
- 用 `get_nearest_large_sesno` / `get_nearest_less_sesno` 把请求区间对齐到文件里真实存在的 sesno 边界（`actual_start..=actual_end`）。
- `io.collect_increment_eles(actual_start..=actual_end)` → `BTreeMap<sesno, Vec<EleOperationData>>`。每个 `EleOperationData.detail` ∈ `Add / Modified / Deleted / None`；`Modified` 携带**属性级 delta**：`ModifiedElement.{added,deleted,modified}_attrs`（+ `*_explicit_attrs` 变体）与 `children_changed`。

> ≈ core.dll `DB_DB::elementsChangedBetween(sesno,…)`（§5.1）——按 session 号取变更元素集，且带"逐元素已改属性列表"（`DB_UserChanges::AttributesModified`）。

### ② 分类进桶（"要不要重算"的核心判定）

`apply_pdms_operation(io, update_log, operation)` 对每个操作分类，**两道门**：

**属性门（wnoevt 类比）** —— `modified_element_affects_model(modified)`：
- `children_changed` 有变化 → 影响（子表变化）。
- 否则：被改属性名（added/deleted/modified + explicit 六个来源）里**任一命中** `crate::version_management::model_impact::attribute_affects_model` → 影响。
- 一个 `Modified` 若**不影响任何几何属性** → `operation_is_known_model_noop` 判真 → `remove_refno_from_log` **移出日志、跳过重算**（这就是"纯元数据/外观改动跳过"的落地）。

**NOUN 几何门 + 分桶** —— `insert_change_by_noun(update_log, refno, noun, is_delete)`：
- `is_delete` → `delete_refnos`。
- 按 noun 映射进 **4 个几何桶**之一：
  - `PRIM` / `PRIM_NOUN_SET`（`GNERAL_PRIM_NOUN_NAMES`）→ `prim_refnos`
  - `LOOP` / `LOOP_OWNER_NOUN_SET`（`GNERAL_LOOP_OWNER_NOUN_NAMES`）→ `loop_owner_refnos`
  - `BRAN` / `HANG` / `HANGER` → `bran_hanger_refnos`
  - `CATA` / `CATA_NOUN_SET`（`TOTAL_CATA_GEO_NOUN_NAMES ∪ USE_CATE_NOUN_NAMES ∪ BRAN_COMPONENT_NOUN_NAMES`）→ `basic_cata_refnos`
  - 其它（非 geo-noun）→ 返回 `None`，`collect_*` 打印"警告：未知 PDMS noun/type"、**不进桶、不重算**。
- **loop 容器上溯**（唯一的一处"显著 owner"逻辑）：若被改元素是 LOOP/顶点容器 noun（`LOOP_CONTAINER_NOUN_SET` = `TOTAL_LOOP_NOUN_NAMES ∪ TOTAL_VERT_NOUN_NAMES`），`resolve_non_container_owner` 沿 owner 向上爬 **≤6 层**，找到第一个非容器 owner，改按 **owner** 入桶——因为几何在 loop-owner 层重算，而非顶点层。

产物 `IncrGeoUpdateLog`（`increment_record.rs`）：

```9:20:src/data_interface/increment_record.rs
pub struct IncrGeoUpdateLog {
    //基本体模型修改了的参考号
    pub prim_refnos: HashSet<RefnoEnum>,
    //拉伸体模型修改了的参考号
    pub loop_owner_refnos: HashSet<RefnoEnum>,
    //元件库模型的属性修改了的参考号
    pub bran_hanger_refnos: HashSet<RefnoEnum>,
    //元件库模型的属性修改了的参考号
    pub basic_cata_refnos: HashSet<RefnoEnum>,
    //删除了的模型
    pub delete_refnos: HashSet<RefnoEnum>,
}
```

> ≈ core.dll 的**两级门控**：`DB_ElementChangesPlugger::PostSetAttribute` 先查 `wnoevt`（属性门），元素类型是否有几何由 `geomset`/`graphicsBehaviour`（NOUN 门）决定（§3、§4）。

### ③ 落库提交（数据层，与模型生成解耦）

`persist_collected_pdms_increment_files → persist_pdms_increment_grouped → commit_version`（specs/022/023）：按 op 顺序生成 `UPSERT pe/<noun>/ATT_UDA` + `pe_owner` 边（先删后插、跨请求）+ `dbnum_info_table`，经 `compute_commit_fingerprint` / `commit_version`（lease + Commit Pending + 锚点固化）原子提交，末尾写 `sesno_version_anchor`。

> ≈ core.dll 的 `DB_UserChanges` 变更日志 + session 锚点；这里对应"把变更落进版本库并固化 committed watermark"。

### ④ 日志 → 生成目标

`orchestrator.rs::gen_all_geos_data(manual_refnos, db_option, incr_updates)`：

- **scope 判定** `decide_generation_scope`：有 incr log 且无 manual/debug → `GenerationScope::Incremental{ log }`；**永不回退 Full**（空日志 / 纯删除都保持 Incremental，有单测锁定）。
- **生成前变换失效**：对 `log.get_all_visible_refnos()`（prim ∪ loop_owner ∪ bran_hanger ∪ basic_cata）调 `pe_transform_refresh::invalidate_pe_transform_for_root_refnos`——`collect_subtree_refnos` **BFS 收 roots∪子孙**、清整棵子树的 `pe_transform` + 内存 transform 缓存；`delete_refnos` 另行 `clear_pe_transform_for_refnos`。避免 owner/POS 变更后 lazy-miss 命中陈旧世界变换；**禁止整库 clear**。
- **目标解析** `gen_pipeline.rs::resolve_incremental_generation_targets(hierarchy, config, log)`：

```152:170:src/fast_model/gen_model/gen_pipeline.rs
pub(crate) fn resolve_incremental_generation_targets(
    hierarchy: &HierarchySnapshot,
    config: &GenPipelineConfig,
    log: &IncrGeoUpdateLog,
) -> Result<GenerationTargets> {
    let generated = targets_from_candidates(
        hierarchy,
        config,
        log.get_all_visible_refnos().into_iter().collect(),
        should_include_bran_hang(config),
    )?;
    Ok(GenerationTargets::new(
        generated.bran_hang_refnos().iter().copied(),
        generated.loop_refnos().iter().copied(),
        generated.cate_refnos().iter().copied(),
        generated.prim_refnos().iter().copied(),
        log.delete_refnos.iter().copied(),
    ))
}
```

  **关键**：增量目标 = **改动 refno 本身**（经 `targets_from_candidates` 按 hierarchy noun 再分类进 bran_hang/loop/cate/prim），**不做 `hierarchy.descendants` 子树展开**（这一点与 `resolve_root_generation_targets` 明显不同——后者会展开子孙）。

### ⑤ 执行生成

`gen_pipeline.rs::execute_generation_targets`：对 `GenerationTargets{ bran_hang, loop, cate, prim, delete }` 跑 GenPipeline（cate/loop/prim/bran-hang 几何 → mesh → 布尔 → 写模型表），删除项移除。`target_hash` 保证不同 scope（full/root/incremental）落到同一目标集时身份一致、可复现。

> **watch 路径**：`increment_manager::execute_incr_update`（watch-incremental 单队列）复用 ①③ 同一 seam（`collect` + `persist_collected_pdms_increment_files` → `commit_version`，见 AGENTS.md specs/022）；模型生成（④⑤）由上层消费 `update_log` 触发。

---

## 2. 逐层对照表

| 关注点 | core.dll / Core3D（逆向） | plant-model-gen（现状） | 结论 |
|---|---|---|---|
| 增量范围 | `DB_DB::elementsChangedBetween(sesno)` `0x58ffc50` | `collect_increment_eles(actual_start..=actual_end)` + `sesno_version_anchor` | ✅ 一致 |
| 变更单元 | `DB_UserChanges` + `AttributesModified(el, vec<attr>)` | `EleOperationData` + `ModifiedElement.{added,deleted,modified}_attrs` | ✅ 一致 |
| **属性门** | `DB_Attribute::wnoevt` `0x58d5290`（off184 / DDL 299311034，内核权威） | `attribute_affects_model` 硬编码白名单 + `operation_is_known_model_noop` 跳过元数据 | ⚠️ 语义对齐，真相源不同（ADR-0009） |
| **NOUN 几何门** | `DB_Noun::geomset` `0x58d8a20`（DDL 859903）/ `graphicsBehaviour` | noun 名单分桶（`PRIM/LOOP_OWNER/CATA_NOUN_SET`+BRAN/HANG）/ `is_geo_noun` | ⚠️ 对齐，但靠名单非字典 |
| **重算粒度 / 显著 owner** | `PartialUpdateDesiMgr::GranularityExpansion` `0x1047d8c0`：IsPrimitive→**SignificantOwner→Members** | 仅 loop 容器→owner 上溯 ≤6（`resolve_non_container_owner`）；**无通用 Members/子树展开** | ❌ 部分缺 |
| 摆放传播 | 摆放变→重算所属几何块（世界变换） | 子树 `pe_transform` BFS 失效（懒重算）；mesh 只重生成被改 refno | ◐ 变换层覆盖子树，mesh 不冗余；但元素自身 POS/ORI 仍触发 mesh 重算 |
| **波及闭包（cascade）** | 克隆/分布式副本（`DB_Clone::getRelatedElements` §10.3）、owner 移动→子树、`CATR`/`SPRE`→引用同 SCOM 的实例 | **未实现**（仅 loop 上溯） | ❌ 缺 |
| 删除 | `ElementsDeleted` + `AncestorDeletes` | `delete_refnos` + 清变换 + 管线移除 | ✅ 基本一致 |
| 非几何跳过 | `wnoevt=true` 或非 `geomset` → 不发事件/不记账 | 元数据-only Modified 跳过；非 geo-noun 落空(警告) | ✅ 一致 |
| 属性级定点修正 | Core3D `VDESPT` 少数 (noun,attr) 特例（§11.2/§14.3，如 PLOO·HEIG、SJOI·JFRE、COCO·CTYP） | 无（通用管线按桶整体重算该元素几何） | ◐ 由整体重算覆盖，无逐特例 |

---

## 3. 对齐点（设计正确）

1. **两道门结构** = `wnoevt × geomset`：属性显著性过滤（白名单）× geo-noun 判定（分桶），与 core.dll 的判定语义同构。
2. **sesno 增量范围**：`cached_sesno+1..=actual_end` ↔ `elementsChangedBetween`，锚点驱动、committed watermark 一致。
3. **元数据跳过**：`operation_is_known_model_noop` ↔ `wnoevt=true` 不发事件——纯 NAME/DESC/外观改动不触发重算。
4. **删除路径**：单独 `delete_refnos` 桶 + 变换清理 + 管线移除，对齐 `ElementsDeleted`。
5. **确定性**：`GenerationTargets` 规整/去重/`target_hash`，使 HashSet 遍历序不影响执行与 provenance（core.dll 无此需求，是 Rust 侧工程增强）。

---

## 4. 缺口 / 差异（价值所在）

### 4.1 属性门真相源：白名单 vs 内核 `wnoevt`（已知取舍）
- core.dll 权威门是内核字典位 `wnoevt`；plant-model-gen 用硬编码白名单（`wnoevt` 不随模型库同步、静态不可导出，见逆向 §14.2）。
- 取"宁多勿漏"+三方交叉校验（rs-core 读取点 / `att_meta` 702 字典 / Core3D 消费方引用）兜底，并有 `scripts/verify_model_impact_attrs.py` 覆盖体检。详见 **ADR-0009** 与开发计划 `docs/plans/2026-07-22-attribute-model-impact-reconciliation-dev-plan.md`。
- 风险：白名单可能与真实 `wnoevt` 漂移（漏判=模型陈旧；误判=多算一次）。留"未来 `att_meta.wnoevt` 查表"钩子。

### 4.2 缺通用"显著 owner + Members 展开"
- Core3D `GranularityExpansion`：`IsPrimitive` → 上卷到 `SignificantOwner`（有意义几何容器）→ 展开 `Members`/处理 `AncestorDeletes`，**按块粒度**排队重算。
- 现状只有 loop 容器→owner 一处上溯；一般元素按自身入桶、且增量目标不展开子孙。→ 复杂几何容器（如设备子装配）若只有内部某元素变更，可能重算粒度与 core.dll 不一致。

### 4.3 缺波及闭包（cascade）
- **owner 移动 → 子实例**：现按子树失效 `pe_transform`（世界变换懒重算）兜底，但**不重生成子实例 mesh**。多数情况够用（子几何本地空间不随父移动而变），但若 mesh 烘焙了世界坐标或存在依赖父姿态的派生几何，则可能漏。
- **`CATR`/`SPRE` → 引用同 SCOM 的所有实例**：改一个实例的目录/规格只重算该实例；**共享目录几何的其它实例未波及**（逆向 §13.1/§11.3 指出这类改动影响面最大）。
- **克隆 / 分布式副本**：`DB_Clone::getRelatedElements`（§10.3）会把改动扩散到所有克隆/绑定副本；现状未纳入。

### 4.4 placement-vs-mesh 残留
- 元素自身的 `POS`/`ORI` 在白名单里 → 触发 mesh 重算，理论上只需变换刷新（`pe_transform`）。子树变换失效已做，但被改元素本身仍走完整重生成。→ 可与 `docs/plans/2026-03-29-transform-refresh-cross-repo-fix.md` 合并优化。

### 4.5 非几何 noun 落空为静默丢弃
- 变更元素 noun 若不在任何 geo-noun 集合 → 打警告后**丢弃**；core.dll 只要过 `geomset`+`wnoevt` 就会重算。→ 若 noun 名单覆盖不全（对比 DDL `geomset`），存在漏判风险（对应"`is_geo_noun` 向字典对齐"后续项，逆向 §7 建议 2）。

---

## 5. 落地建议 / 后续项（记录在案）

以上 4.2–4.4 正是加固计划 §5 已记录的三个后续项：

1. **波及闭包（cascade）**：`CATR`/`SPRE`→引用实例、owner 移动→子树、克隆/分布式副本纳入增量重算集合。
2. **通用显著 owner 粒度**：把 Core3D `GranularityExpansion` 的 `SignificantOwner + Members` 展开移植到 `resolve_incremental_generation_targets`。
3. **placement-vs-mesh 分离**：`POS`/`ORI` 只触发变换刷新、不重算 mesh。

另（4.1/4.5）：`wnoevt` 权威查表（需活 E3D / 扩展字典导入）、`is_geo_noun` 与 DDL `geomset` 对齐——均为长期项。

---

## 6. 关键代码锚点速查

| 环节 | 符号 / 文件 |
|---|---|
| 增量采集 | `sesno_increment.rs::collect_pdms_increment_for_file_with_operations` |
| 属性门 | `sesno_increment.rs::modified_element_affects_model` / `operation_is_known_model_noop` |
| 白名单 | `version_management/model_impact.rs::attribute_affects_model` |
| NOUN 分桶 | `sesno_increment.rs::insert_change_by_noun` / `resolve_non_container_owner` |
| 增量日志 | `data_interface/increment_record.rs::IncrGeoUpdateLog` |
| 落库提交 | `sesno_increment.rs::persist_pdms_increment_grouped` → `versioned_db::version_commit::commit_version` |
| scope 判定 | `fast_model/gen_model/orchestrator.rs::decide_generation_scope` / `gen_all_geos_data` |
| 变换失效 | `pe_transform_refresh.rs::invalidate_pe_transform_for_root_refnos` / `collect_subtree_refnos` |
| 目标解析 | `fast_model/gen_model/gen_pipeline.rs::resolve_incremental_generation_targets` / `targets_from_candidates` |
| 目标聚合 | `fast_model/gen_model/noun_collection.rs::GenerationTargets` |
| 执行生成 | `fast_model/gen_model/gen_pipeline.rs::execute_generation_targets` |

> 对照逆向文档：`docs/reverse/core_dll_noun_att_model_update.md` §3（wnoevt 门控）、§4（NOUN/ATT 字典标志）、§5（DB_UserChanges/增量）、§10.3（克隆波及）、§11.3（PartialUpdateDesiMgr 粒度）、§13/§14（属性清单与三方校验）。
