---
status: accepted
date: 2026-07-22
---

# 属性→模型影响判定采用「硬编码·宁多勿漏」白名单，而非内核 wnoevt 权威

增量生成需要判定"一次属性改动要不要重算该元素的几何"。我们决定：**`attribute_affects_model` 保持为一张硬编码、宁多勿漏（inclusive）的几何影响属性白名单，其内容以 core.dll/Core3D 逆向分析 + 运行库属性字典 `att_meta` 交叉校验为依据；不从 E3D 内核的权威 `wnoevt` 标志取数。**

关键取舍：

- **`wnoevt` 才是内核权威，但取不到**：逆向确认 E3D 内核里"改一个属性要不要往下游发事件/重算"的最终开关是每属性字典标志 `wnoevt`（`DB_Attribute` off184 / dabacon 字段 `299311034`）。但它是**内核 dabacon 字典数据**：不随模型库同步（运行库 `att_meta` 只有 `hash`+`meta_cn_name`，702 条无 `wnoevt`）、不在 `dicvir.dat`（版本戳）、静态不可导出；要拿全量需活 E3D 会话导出或扩展字典导入工具。为一个**热路径**的增量门控引入活内核/字典运行时依赖，当前不划算。

- **宁多勿漏（inclusive bias）**：漏判（几何相关属性未列入 → 增量跳过 → 模型陈旧）是**正确性 bug**；误判（非几何属性误列入 → 多重算一次）只是**成本**。故对"可能影响几何"的属性一律纳入。这也决定了本表偏大而非偏小。

- **不靠内核也能有权威性**：白名单以三方交叉校验支撑（见 `docs/reverse/core_dll_noun_att_model_update.md`）——① `rs-core`/`plant-model-gen` 几何生成实际读取的属性（577 处读取点，§13.2）；② 运行库 `att_meta` 的 702 属性字典全集（名+dabacon 哈希，§13.2 命中率 100%，§14.1）；③ Core3D 消费方（DESDRA/VDESPT）实际引用的属性（§14.3）。三者交集/并集给出足够权威的清单，且**零运行时依赖**。

- **未来切换点（留钩子、非本次）**：若日后把 `wnoevt` 从活 E3D 导出或扩展字典导入落入 `att_meta.wnoevt` 列，`attribute_affects_model` 可平滑改为查表（权威门控）。届时白名单退化为回退/校验基线。

被否决的替代方案：① **现在就用内核 `wnoevt`**——需活 E3D、给热路径加内核/字典运行时依赖，收益不抵成本；② **"任意属性改动即重算"**——废掉增量门控的意义（元数据/外观改动也全量重算）；③ **极小白名单**——漏判风险高、直接导致模型陈旧的正确性问题。

参见实现 `src/version_management/model_impact.rs`、接入 `src/data_interface/sesno_increment.rs`，以及开发计划 `docs/plans/2026-07-22-attribute-model-impact-reconciliation-dev-plan.md`。
