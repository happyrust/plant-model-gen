# 增量更新如何判定"哪些需要模型增量生成" —— 当前实现 vs core.dll 逻辑

> 分析对象：`plant-model-gen`（aios-database）当前增量管线
> 对照基准：`docs/reverse/core_dll_noun_att_model_update.md`（AVEVA Everything3D `core.dll` / `Core3D.dll` 逆向，§1–§14）
> 依据代码：`src/data_interface/sesno_increment.rs`、`src/data_interface/increment_record.rs`、
> `src/version_management/{model_impact,increment_run,model_gen_catchup}.rs`、
> `src/versioned_db/model_gen_debt.rs`、
> `src/fast_model/gen_model/{orchestrator,gen_pipeline,noun_collection}.rs`、
> `src/pe_transform_refresh.rs`
> 日期：2026-07-23
>
> **语义纠偏**：本文旧版把“是否重算”概括成 `wnoevt × geomset`。继续下钻
> `Core3D.dll::EVALAT/IDCHNG` 后，该结论已被推翻。当前统一采用：
> `wnoevt` 管事件，`DCHC/EVALAT` 管设计变化，noun/引用/SignificantOwner 管目标与粒度。

---

## 0. 结论速览（TL;DR）

- 当前实现与 Core3D **部分同构，但不是“忠实复刻”**。正确的三层语义是：
  ① `wnoevt` 决定是否进入 core 事件/记账；② `DCHC + EVALAT` 决定是否进入
  `QCHGLS` 及如何传播；③ noun、引用关系、`SignificantOwner + Members` 决定更新目标和粒度。
- Rust 的 `attribute_affects_model` / `classify_attribute_model_impact` 更接近
  **`DCHC/EVALAT` 的生成器侧近似**，不是 `wnoevt` 的替代品。当前采用
  trigger / known-neutral / unknown-fallback 三态：未知属性和 UDA 保守触发，
  只有明确的 known-neutral 才跳过模型欠账。
- 原始 sesno 操作流对应 `DB_UserChanges/elementsChangedBetween`；筛选后的
  `IncrGeoUpdateLog` / `GenerationTargets` 更接近有损的
  `QCHGLS + PartialUpdate` 目标队列。两者不能混为同一层。
- owner/成员结构波及已有部分实现：`apply_critical_model_expansion` 会补入旧/新
  owner 和 children 差集；仍缺通用 `SignificantOwner + Members`、目录反向引用、
  克隆/分布式副本等闭包。
- Rust 额外实现了 data anchor、持久化 `model_gen_debt`、连续欠账追平和独立
  model-gen anchor。模型失败不回滚已提交数据，这是当前一致性设计的重要组成。

---

## 1. 当前实现：七阶段管线

### ① 增量采集（sesno 区间 → 操作流）

`sesno_increment.rs::collect_pdms_increment_for_file_with_operations(project, file, cached_sesno, to_sesno, detail)`：

- 打开 E3D/PDMS db 文件（`pdms_io::PdmsIO`），读最新 sesno；请求区间 = `cached_sesno + 1 .. to_sesno`（`to_sesno` 省略则用文件最新 sesno）。`cached_sesno` 即系统已固化到的 `sesno_version_anchor`（specs/022）。
- 用 `get_nearest_large_sesno` / `get_nearest_less_sesno` 把请求区间对齐到文件里真实存在的 sesno 边界（`actual_start..=actual_end`）。
- `io.collect_increment_eles(actual_start..=actual_end)` → `BTreeMap<sesno, Vec<EleOperationData>>`。每个 `EleOperationData.detail` ∈ `Add / Modified / Deleted / None`；`Modified` 携带**属性级 delta**：`ModifiedElement.{added,deleted,modified}_attrs`（+ `*_explicit_attrs` 变体）与 `children_changed`。

> ≈ core.dll `DB_DB::elementsChangedBetween(sesno,…)` + `DB_UserChanges` 的原始变化
> 载荷：按 session 号取变更元素集，并保留逐元素属性差异。它尚未等于 Core3D 的
> `QCHGLS`。

### ② 模型影响分类、直接分桶与关键结构扩展

`classify_operation` / `classify_modified_element` 先把原始操作分类：

- `Add` / `Deleted` 固定 trigger，`None` 固定 neutral。
- `children_changed`、`OWNER`、`NOUN/TYPE` 是结构关键变化，固定 trigger。
- `--no-model-impact-filter` 关闭过滤时，所有 `Modified` 都 trigger。
- UDA 变化走 `unknown_fallback`，不会静默跳过。
- 其它属性逐个经 `classify_attribute_model_impact`：任一 `AffectsModel` → trigger；
  任一 `Unknown`（或属性列表为空）→ `unknown_fallback`；只有全部是
  `KnownNeutral` 才 neutral。当前明确 neutral 的是
  `NAME/DESC/PURP/FUNCTION`。

随后 `apply_pdms_operation` 按 noun 把**直接生成目标**分桶：
- `is_delete` → `delete_refnos`。
- 按 noun 映射进 **4 个几何桶**之一：
  - `PRIM` / `PRIM_NOUN_SET`（`GNERAL_PRIM_NOUN_NAMES`）→ `prim_refnos`
  - `LOOP` / `LOOP_OWNER_NOUN_SET`（`GNERAL_LOOP_OWNER_NOUN_NAMES`）→ `loop_owner_refnos`
  - `BRAN` / `HANG` / `HANGER` → `bran_hanger_refnos`
  - `CATA` / `CATA_NOUN_SET`（`TOTAL_CATA_GEO_NOUN_NAMES ∪ USE_CATE_NOUN_NAMES ∪ BRAN_COMPONENT_NOUN_NAMES`）→ `basic_cata_refnos`
  - 其它（非 geo-noun）→ 返回 `None`，`collect_*` 打印"警告：未知 PDMS noun/type"、**不进桶、不重算**。
- **loop 容器上溯**（唯一的一处"显著 owner"逻辑）：若被改元素是 LOOP/顶点容器 noun（`LOOP_CONTAINER_NOUN_SET` = `TOTAL_LOOP_NOUN_NAMES ∪ TOTAL_VERT_NOUN_NAMES`），`resolve_non_container_owner` 沿 owner 向上爬 **≤6 层**，找到第一个非容器 owner，改按 **owner** 入桶——因为几何在 loop-owner 层重算，而非顶点层。

最后 `apply_critical_model_expansion` 对结构变化补目标：

- `OWNER` 改动：按旧 sesno 补旧 owner，按当前 sesno 补新 owner；
- `children_changed`：对旧/新 children 的差集分别按旧/新 sesno 补目标。

这三步分别近似 Core3D 的 **`DCHC/EVALAT` 分类、直接目标选择、部分结构闭包**。
noun 分桶是生成执行路由，不是 `geomset` 与 `wnoevt` 组成的第二道严格与门。

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

> `IncrGeoUpdateLog` 已经是模型目标/欠账集合。更接近 Core3D 的
> `QCHGLS + PartialUpdate` 输入，而不是原始 `DB_UserChanges`。

### ③ 落库提交（数据层，与模型生成解耦）

`persist_collected_pdms_increment_files → persist_pdms_increment_grouped → commit_version`（specs/022/023）：按 op 顺序生成 `UPSERT pe/<noun>/ATT_UDA` + `pe_owner` 边（先删后插、跨请求）+ `dbnum_info_table`，经 `compute_commit_fingerprint` / `commit_version`（lease + Commit Pending + 锚点固化）原子提交，末尾写 `sesno_version_anchor`。

neutral 只影响是否写入模型欠账，**不影响 PE/ATT/层级数据提交**。这对应“数据历史”
和“模型更新队列”两条轴必须分离。

### ④ 数据锚点 → 持久化模型欠账

`IncrementRun` 只为成功固化 data anchor 的 dbnum 写
`model_gen_debt(dbnum, from_sesno, to_sesno, fingerprint, IncrGeoUpdateLog)`。
欠账写失败不会回滚数据锚点，但会阻止该 dbnum 本轮模型生成并显式报告失败。

### ⑤ 连续欠账分析与追平

`analyze_model_gen_debt` 从 model-gen watermark 起按 sesno 顺序合并**连续**欠账：

- 连续范围合并为一个 `merged_update_log`；
- 有洞时停止在洞前，并标记 `needs_full_regen`；
- 只有显式 `model-version catch-up --allow-full-regen` 才允许整库兜底；
- `catch_up_model_generation` 把合并后的日志交给生成管线。

### ⑥ 欠账日志 → 生成目标

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

### ⑦ 执行生成并固化模型水位

`gen_pipeline.rs::execute_generation_targets`：对 `GenerationTargets{ bran_hang, loop, cate, prim, delete }` 跑 GenPipeline（cate/loop/prim/bran-hang 几何 → mesh → 布尔 → 写模型表），删除项移除。`target_hash` 保证不同 scope（full/root/incremental）落到同一目标集时身份一致、可复现。

生成成功后，`finalize_model_generation` 在一个事务中写独立的
`sesno_version_anchor(..., source='model_gen')`，并把覆盖范围内欠账标为已消费。
生成失败时 data watermark 保持前进、model-gen watermark 不前进，后续 watch 即使源
sesno 未继续增长也能从欠账追平。

> **watch 路径**：当前复用 `IncrementRun` 的 collect → commit data →
> write debt → catch-up → finalize model-gen anchor 全链路，而不是只把一次内存
> `update_log` 直接交给生成器。

---

## 2. 逐层对照表

| 关注点 | core.dll / Core3D（逆向） | plant-model-gen（现状） | 结论 |
|---|---|---|---|
| 增量范围 | `DB_DB::elementsChangedBetween(sesno)` `0x58ffc50` | `collect_increment_eles(actual_start..=actual_end)` + `sesno_version_anchor` | ✅ 一致 |
| core 事件边界 | `DB_Attribute::wnoevt`：不广播、不写当前 `DB_UserChanges` | 无实时 Plugger 等价层；Rust 直接采集 sesno 数据变化 | ⚪ 不应拿它映射模型白名单 |
| 原始变更单元 | `DB_UserChanges` + `AttributesModified(el, vec<attr>)` | `EleOperationData` + `ModifiedElement` + `PdmsSesnoElementChange` | ✅ 基本一致 |
| **设计/模型影响分类** | `IDCHNG` 读 `DCHC`，`EVALAT` 再做特例、重定向和传播 | `classify_attribute_model_impact` + `classify_modified_element` | ⚠️ 语义接近，但 Rust 没有 DCHC 真相源 |
| **设计变化队列** | `QCHGLS(ref[2], changeCode)`，相同 ref 保留较大 code | `IncrGeoUpdateLog` → `GenerationTargets` | ⚠️ 目标集合近似；code、原因和部分传播边丢失 |
| NOUN 的角色 | `geomset/graphicsBehaviour` 是几何元数据；EVALAT/PartialUpdate 结合 noun 选目标 | noun 名单把直接目标路由到 prim/loop/bran/cata 执行桶 | ⚠️ 是执行路由近似，不是严格前置门 |
| 结构变化 | Deleted/Moved/Reordered/MemberChanged 分集合，保留旧/新 owner 影响 | delete 桶；`apply_critical_model_expansion` 补旧/新 owner 与 children 差集 | ◐ 部分覆盖，未保留完整结构事件类型 |
| **重算粒度 / 显著 owner** | `GranularityExpansion`：IsPrimitive→**SignificantOwner→Members** | loop 容器→owner 上溯 ≤6，加结构变化定点扩展 | ❌ 仍缺通用 Members/块级展开 |
| 摆放传播 | 摆放变→重算所属几何块（世界变换） | 子树 `pe_transform` BFS 失效（懒重算）；mesh 只重生成被改 refno | ◐ 变换层覆盖子树，mesh 不冗余；但元素自身 POS/ORI 仍触发 mesh 重算 |
| **引用/副本闭包** | `BAKREF/ATTABK` 目录反向引用；克隆/分布式副本扩散 | CATR/SPRE 等仅触发当前直接目标；未见反向实例索引和 clone 闭包 | ❌ 缺 |
| 删除 | `ElementsDeleted` + `AncestorDeletes` | `delete_refnos` + 清变换 + 管线移除 | ✅ 基本一致 |
| 数据-only 变化 | `wnoevt=false, DCHC=0` 时仍可有数据事件，但不进 QCHGLS | 所有操作照常提交；只有全 known-neutral 才不写模型目标 | ✅ 分层方向正确 |
| 未知属性 | 由字典 DCHC/消费方规则决定 | `unknown_fallback` / unknown UDA 保守触发 | ✅ 宁多勿漏 |
| 批次可靠性 | 当前变化同步交付，回调重入形成下一批 | data anchor + 持久 debt + 连续追平 + model-gen anchor | ✅ Rust 侧更强的崩溃恢复语义 |
| 属性级定点修正 | `EVALAT/VDESPT` 的 noun/attr 特例及引用重定向 | 多数收敛为“当前 refno 进某一生成桶” | ◐ 整体重算可覆盖直接效果，不能替代依赖传播 |

---

## 3. 已对齐或方向正确的部分

1. **原始变化与模型目标分层**：数据提交不受 known-neutral 模型筛选影响，避免把
   `DCHC=0` 误写成“不保存这次变化”。
2. **sesno 增量范围**：`cached_sesno+1..=actual_end` ↔
   `elementsChangedBetween`，锚点驱动且范围可审计。
3. **保守未知策略**：未知属性、未知 UDA、空属性差异都会触发模型欠账；只有显式
   known-neutral 才跳过，符合“漏判是正确性 bug”的取舍。
4. **结构变化已有定点扩展**：删除单独处理，OWNER 变化保留旧/新 owner，
   children 变化保留差集目标，已覆盖一部分 MemberChanged 语义。
5. **增量 scope 不暗退全量**：空日志、纯删除仍保持 Incremental；欠账有洞也只报告，
   除非运维显式允许 full regen。
6. **数据/模型双水位与持久欠账**：生成失败不回滚数据，未消费欠账可在后续追平，
   比只在内存中传递一次更新集合更可靠。
7. **确定性**：`GenerationTargets` 规整/去重/`target_hash`，使集合遍历序不影响执行
   与 provenance。

---

## 4. 缺口 / 差异（价值所在）

### 4.1 模型影响真相源：Rust 白名单/三态 vs Core3D `DCHC/EVALAT`

- `wnoevt` 是 core **事件总闸**，不是模型影响真相源；真正对应 Rust 分类器的是
  `DCHC` 以及 `EVALAT` 的强制 code、noun/owner/ref 特例。
- Rust 当前以生成器实际读取属性、`att_meta` 名单和 Core3D 静态引用交叉校验，
  这是可用的保守近似，但尚未导入每属性 `DCHC`，也没有表达 EVALAT 的传播规则。
- 风险来自“白名单与真实 DCHC/EVALAT 漂移”，而不是“白名单与 wnoevt 漂移”。
  `wnoevt` 日后可作为事件兼容性元数据导入，但不能替代模型影响字段。

### 4.2 change code 与 effect 类型丢失

- `QCHGLS` 保留 1..4 change code，并在重复 ref 时保留更大值；Rust 最终只有分类桶和
  refno 集合。
- 这会丢失“只刷新变换 / 直接重建 / 依赖扩散 / 结构变化”等原因，导致后续只能采取
  偏重的统一重算策略。DCHC 1..4 的官方枚举名尚未知，设计 Rust effect 类型时不能
  直接臆造一一对应名称。

### 4.3 缺通用“显著 owner + Members 展开”

- Core3D `GranularityExpansion`：`IsPrimitive` → 上卷到 `SignificantOwner`（有意义几何容器）→ 展开 `Members`/处理 `AncestorDeletes`，**按块粒度**排队重算。
- 现状是 loop 容器上溯 + OWNER/children 结构特例；一般元素仍按自身入桶，增量目标
  不展开子孙。复杂几何容器内部元素变化时，可能遗漏应在容器层统一重建的成员。

### 4.4 波及闭包仍不完整

- **owner/成员变化**：旧/新 owner 与 children 差集已经入目标，且可见 roots 的
  `pe_transform` 子树会失效；但尚无通用 SignificantOwner/Members 闭包。
- **`CATR`/`SPRE` → 引用同 SCOM 的所有实例**：改一个实例的目录/规格只重算该实例；**共享目录几何的其它实例未波及**（逆向 §13.1/§11.3 指出这类改动影响面最大）。
- **克隆 / 分布式副本**：`DB_Clone::getRelatedElements`（§10.3）会把改动扩散到所有克隆/绑定副本；现状未纳入。

### 4.5 placement-vs-mesh 残留

- 元素自身的 `POS`/`ORI` 在白名单里 → 触发 mesh 重算，理论上只需变换刷新（`pe_transform`）。子树变换失效已做，但被改元素本身仍走完整重生成。→ 可与 `docs/plans/2026-03-29-transform-refresh-cross-repo-fix.md` 合并优化。

### 4.6 noun 分桶会丢依赖源

- 变更元素 noun 若不在直接生成桶集合，通常只打印警告并不把自身加入
  `IncrGeoUpdateLog`；结构特例能补一部分 owner/child 目标，但目录定义、规格、
  反向引用节点等“自身不直接产 mesh、却影响别的实例”的 source 仍可能丢失。
- 正确对照不是“只要过 `geomset + wnoevt` 就重算”，而是 Core3D 的
  `EVALAT/BAKREF/ATTABK/UpdateChangeList` 会把 source 重定向或扩散成真正设计目标。

---

## 5. 落地建议 / 后续项

1. **把 bool/三态提升为 effect 模型**：至少区分 data-only、transform-only、
   direct-geometry、dependency-cascade、structural-membership、unknown；保留原始
   DCHC code（若取得），未知仍保守触发。
2. **建立依赖反向索引**：先覆盖 CATR/SPRE/SCOM→设计实例，再覆盖 clone/绑定副本；
   让非直接几何 noun 能产出真正的下游生成目标。
3. **实现通用 SignificantOwner + Members**：在目标解析前完成块级归一和成员展开，
   并明确新增、修改、删除的不同策略。
4. **placement-vs-mesh 分离**：POS/ORI 默认只失效 transform；只有确实依赖世界姿态
   的派生几何才升级为 mesh 重建。
5. **把 noun 名单定位为执行路由，不再当影响门**：影响分类先产生 effect/target，
   noun 分桶只负责把最终目标送入对应生成器。
6. **做 CLI + JSON 动态校验**：在活 E3D 记录
   `(source ref, noun, attr, DCHC, QCHGLS ref/code)`，与 Rust 输出的
   `PdmsSesnoElementChange + IncrGeoUpdateLog + GenerationTargets` 对比，覆盖普通属性、
   owner 移动、目录共享、删除和克隆场景。
7. **保持关联文档同步**：证据文档与 ADR-0009 已在 2026-07-23 完成同一纠偏；
   后续若恢复 DCHC 枚举名或新增动态证据，应同时更新流程总览、证据文档、对照文档
   和 ADR，避免再次形成两套语义。

---

## 6. 关键代码锚点速查

| 环节 | 符号 / 文件 |
|---|---|
| 增量采集 | `sesno_increment.rs::collect_pdms_increment_for_file_with_operations` |
| 影响分类 | `sesno_increment.rs::classify_operation` / `classify_modified_element` |
| 属性 effect 近似 | `version_management/model_impact.rs::classify_attribute_model_impact` / `attribute_affects_model` |
| 直接目标分桶 | `sesno_increment.rs::apply_pdms_operation` / `insert_change_by_noun` / `resolve_non_container_owner` |
| 结构扩展 | `sesno_increment.rs::apply_critical_model_expansion` |
| 增量日志 | `data_interface/increment_record.rs::IncrGeoUpdateLog` |
| 落库提交 | `sesno_increment.rs::persist_pdms_increment_grouped` → `versioned_db::version_commit::commit_version` |
| 欠账写入/分析 | `versioned_db/model_gen_debt.rs::write_model_gen_debt` / `analyze_model_gen_debt` |
| 欠账追平 | `version_management/model_gen_catchup.rs::catch_up_model_generation` |
| scope 判定 | `fast_model/gen_model/orchestrator.rs::decide_generation_scope` / `gen_all_geos_data` |
| 变换失效 | `pe_transform_refresh.rs::invalidate_pe_transform_for_root_refnos` / `collect_subtree_refnos` |
| 目标解析 | `fast_model/gen_model/gen_pipeline.rs::resolve_incremental_generation_targets` / `targets_from_candidates` |
| 目标聚合 | `fast_model/gen_model/noun_collection.rs::GenerationTargets` |
| 执行生成 | `fast_model/gen_model/gen_pipeline.rs::execute_generation_targets` |
| 模型水位固化 | `versioned_db/model_gen_debt.rs::finalize_model_generation` |

> 逆向语义总览以 `docs/reverse/core_dll_incremental_update_flow.md` 的 2026-07-23
> 三层模型为索引；`docs/reverse/core_dll_noun_att_model_update.md` 已同步该语义，
> 并保留地址、反编译片段、字典字段与复现方法等详细证据。
