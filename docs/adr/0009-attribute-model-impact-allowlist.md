---
status: accepted
date: 2026-07-22
---

# 属性→模型影响判定采用「硬编码·宁多勿漏」基线，目标语义对齐 DCHC/EVALAT

增量生成需要判定“一次属性改动是否形成模型欠账，以及欠账应扩散到哪些目标”。
我们决定：**当前继续以 `attribute_affects_model` 为硬编码、宁多勿漏（inclusive）的
生成器输入基线，并由 `classify_attribute_model_impact` /
`classify_modified_element` 实施 trigger / known-neutral / unknown-fallback 三态；
它近似的是 Core3D 的 `DCHC/EVALAT` 模型影响层，而不是 core.dll 的
`wnoevt` 事件门。**

关键取舍：

- **分清两道不同语义**：
  - `wnoevt`（`DB_Attribute` off184 / 字段 `299311034`）只决定 core 是否执行普通
    属性广播和 `DB_UserChanges::attributeModified`；
  - Core3D 全局订阅后由 `IDCHNG` 读取 `DCHC`（字段 `596407`），再经 `EVALAT`
    的强制 code、noun/owner/ref 规则决定是否及如何写 `QCHGLS`。
  因此 `wnoevt=false` 不是“必然重算”，`wnoevt` 清单也不是模型影响真相源。

- **当前拿不到完整 DCHC/EVALAT 动态事实**：运行库 `att_meta` 只有属性名、hash 和
  中文元数据，没有 `DCHC/PLCF/wnoevt`；DCHC 只是传播起点，仍需结合 EVALAT 的
  `REDRAW/INTUBE` 强制 code 与引用/owner 重定向。为热路径引入活 E3D 依赖当前
  不划算。

- **宁多勿漏（inclusive bias）**：漏判（模型相关属性被当成 neutral → 不写欠账 →
  模型陈旧）是正确性 bug；误判只是多算成本。当前实现因此让未知属性、未知 UDA、
  空属性差异保守触发，只有明确的 `NAME/DESC/PURP/FUNCTION` 为 known-neutral；
  `--no-model-impact-filter` 仍提供“所有 Modified 触发”的逃生口。

- **离线基线的证据来源**：① `rs-core` / `plant-model-gen` 生成链实际读取的属性；
  ② `att_meta` 的 702 属性名/hash；③ Core3D 的 DCHC 通用路径、标量特例、引用级联
  和目录解析。该交叉校验足以支撑保守基线，但不宣称等价于 Core3D。

- **未来演进**：从活 E3D 或扩展字典导入
  `wnoevt/DCHC/PLCF/isPseudo/casc`，并记录
  `(source ref, noun, attr, DCHC, QCHGLS ref/code)` 动态轨迹。分类接口升级为
  `(noun, attr, effect, raw_dchc)`，至少区分 data-only、transform-only、
  direct-geometry、dependency-cascade、structural-membership、unknown。
  `wnoevt` 只补事件兼容性，DCHC/EVALAT 事实用于模型影响校验。

被否决的替代方案：

1. **把 `wnoevt=false` 当模型白名单**——语义错误；它是事件超集，会把
   `DCHC=0` 的数据变化误判为模型变化。
2. **只查 DCHC、忽略 EVALAT/依赖闭包**——会漏强制 code、owner/ref 重定向、
   draw-list 依赖、目录反向引用和克隆副本。
3. **任意属性改动即重算**——可作为逃生口，但不应成为默认热路径。
4. **极小白名单或未知默认 neutral**——漏判会形成永久模型陈旧。

> 2026-07-23 修订：原 ADR 把 `wnoevt` 写成“模型影响的内核权威”。新逆向证据已确认
> 该表述错误，现以 `wnoevt=事件边界、DCHC/EVALAT=模型影响、
> noun/ref/SignificantOwner=目标与粒度` 为准。

> 2026-07-24 补充：已反编译 `Core3D.dll` 的 `IDCHNG/EVALAT/EVALCD/EVALST`，还原
> DCHC 1..4 的**操作语义**（证据文档 §15）：**DCHC 是「作用域路由选择器」**——
> `0`=NoChange、`1`=重定向到关联/owner(REF)、`2`=自身、`3/4`=自身+依赖闭包传播；
> `REDRAW→4`、`INTUBE→1`；QCHGLS 按 ref 去重保留最大 code，但下游 `ModelState=0`
> 不消费该 code。**枚举名**在二进制中不可静态恢复（DDL 裸整数）。据此，本 ADR
> 「未来演进」的 `effect` 升级可落为：0→data-only、1→redirect-to-related、
> 2→self-direct-geometry、3/4→dependency-cascade；Rust 静态拿不到每属性 DCHC，
> 仍保留 inclusive 兜底，但应补齐**「改在 A、欠账记到 owner/被引用实例 B」**的路由
> （呼应 §13.4 与 ADR-0011 目录反向闭包）。

参见实现 `src/version_management/model_impact.rs`、
`src/data_interface/sesno_increment.rs`，逆向总览
`docs/reverse/core_dll_incremental_update_flow.md`，详细证据
`docs/reverse/core_dll_noun_att_model_update.md`，以及对照
`docs/reverse/incremental_update_vs_core_dll.md`。
