# 开发计划：属性→模型影响判定（`attribute_affects_model`）对齐加固

> 日期：2026-07-22 ｜ 方式：grill-with-docs（关键决策见文末 ADR 0009、术语见 `CONTEXT.md`）
> 依据分析：`docs/reverse/core_dll_noun_att_model_update.md`（core.dll/Core3D 逆向 + §13 几何输入属性 + §14 三方交叉校验）
>
> **2026-07-23 语义修订**：`wnoevt` 只代表 core 事件边界；模型影响基线应近似
> `DCHC/EVALAT`。本计划的 M1/M2 实施事实保留，旧函数名与后续方向按当前代码修正。

## 1. 背景与现状（事实，来自代码）

"按属性门控增量重算"**已实现并接入管线**，本计划是**对齐加固**而非新建：

- 判定函数：`src/version_management/model_impact.rs::attribute_affects_model(name)` —— 一张"几何影响属性白名单"（`matches!` + `starts_with("PARA"/"PARAM")`），带 `att.`/`ATT.` 前缀规整与大写归一。
- 接入点：`src/data_interface/sesno_increment.rs`
  - `classify_modified_element()` 汇总普通/explicit/UDA 属性差异，并实施
    trigger / known-neutral / unknown-fallback 三态；
  - `apply_pdms_operation()` 只跳过全 known-neutral 的 `Modified`；
    `apply_critical_model_expansion()` 另补旧/新 owner 与 children 差集。
- 数据基础：`pdms-io` 已提供属性级 delta，**无需额外采集**。
- 另一入口：`field_path_affects_model(path)`（PE/ATT 历史 diff 路径）复用同一函数。
- 对照语义（逆向所得）：`wnoevt`（字段 `299311034`）控制 core 事件；
  `DCHC`（字段 `596407`）经 `IDCHNG/EVALAT` 控制 Core3D 设计变化及传播。
  `att_meta` 当前两者都未同步，白名单只是离线、宁多勿漏的模型影响基线。

## 2. 目标（范围：对齐/加固 + 校验）

1. 把 `attribute_affects_model` 白名单**补齐 §13/§14 发现的几何相关缺口**，并明确边界（宁多勿漏）。
2. 增加**可复现的校验**（CLI+JSON / HTTP，**不写 `cargo test`**，遵 `AGENTS.md`）：白名单 vs 权威属性字典 `att_meta` 覆盖体检 + 行为回归（几何编辑触发、元数据编辑剔除）。
3. 明确**不在本计划**的项（完整波及闭包、placement-vs-mesh、
   DCHC/effect 动态管线）为记录在案的后续项。

**取舍原则（见 ADR 0009）**：白名单保持"硬编码 + 宁多勿漏"。漏判 = 模型陈旧（正确性 bug），误判 = 多算一次（成本可控）；故对"可能影响几何"的属性一律纳入。

## 3. 缺口清单（现有白名单缺、§13.2/§14 认定影响几何 → 本次补入）

| 分组 | 补入属性 | 依据 | 备注 |
|---|---|---|---|
| 顶点/坐标 | `PX` `PY` `PZ` `DX` `DY` | §13.2 D | SPVE/SVER/PVER 顶点改坐标时其 `modified_attrs` 为 PX/PY/PZ；父 SPRO 不一定重列子表 → 现在会漏。**优先级最高** |
| 定位变体 | `POSL` `POSS` `POSE` `NPOS` `CPOS` | §13.2 A | 现仅 `POS`；管件/分支端点位置变体会漏 |
| 弯角 | `BANG` | §13.2 A | 弯头角度 |
| 管路布线/几何 | `ZDIS`(坡降) `LEAV` `CURD` `CURTYP` `OPDI` `ROUT` `DRNS` `DRNE` | §13.2 E | 坡降/离开点/曲率/外径/路由/排水端点，影响管路几何路径（宁多勿漏） |
| 规格/类型 | `PSPE` `CTYP` `JFRE` `JLIN` | §13.2 B/F、§14.3 | `CTYP`/`JFRE` 系 Core3D VDESPT (noun,attr) 特例；`PSPE` 规格；`JLIN` 布线定位 |
| 设计参数/表 | `DELP` `RINS` `PKDI` | §13.2 C/G | 增量位置/保温半径/P-line 方向键 |

> `att_meta`(702) 交叉校验：以上除派生项 `XDIR`（X 轴由 ORI+Y/Z 派生）、`RAD`（实为 `RADI`）外均为真实 dabacon 属性（逆向文档 §14.1）。`XDIR`/`RAD` **不纳入**。

**实现动作**：把上述属性并入 `attribute_affects_model` 的 `matches!` 相应分组臂（保持注释分组结构）。

## 4. 具体改动（T-任务）

- **T1｜白名单补缺**：编辑 `src/version_management/model_impact.rs`，按 §3 分组把缺口属性加入 `matches!` 臂；`CURTYP` 为 6 字母，确认归一后按整名匹配（非 `starts_with`）。
- **T2｜边界注释**：在函数上方说明清单交叉校验自 core.dll/Core3D 逆向，
  目标语义近似 `DCHC/EVALAT`，`wnoevt` 只属于事件边界。
- **T3｜字典覆盖体检（CLI/JSON，非 cargo test）**：新增一个只读校验入口（复用现有 CLI 子命令或加一个 `--verify-model-impact-attrs` 模式），对运行中 SurrealDB `att_meta` 做：
  - 报告白名单里"形似 dabacon 名(4–6 大写)但不在 `att_meta`"的项（typo/不存在守卫）；
  - 报告 §13.2 几何输入属性对白名单的覆盖率（应 100%）。
  - 输出 JSON，便于 CI/人工核对（对齐 `AGENTS.md`：aios-database 用 CLI+JSON 验证）。
- **T4｜行为回归（CLI/HTTP，非 cargo test）**：用现成增量 CLI 对一段已知 sesno 范围跑 `collect_pdms_increment_*`，检查产物 `element_changes[].{impact_decision, impact_reason, classified, model_category}` 与 `IncrGeoUpdateLog`：
  - 断言：几何编辑（如改 `PZ`/`ZDIS`/`CTYP` 的元素）进入相应桶；
  - 断言：纯元数据编辑（`NAME`/`DESC`）被剔除、不进桶。
- **T5｜文档**：更新逆向文档 §13.4/§14 交叉引用到本计划与 `model_impact.rs`；本计划与 ADR 0009、CONTEXT 术语同批提交。

> 注：`model_impact.rs` 现有 `#[cfg(test)]` 单测保留但**不新增/不运行 cargo test**（`AGENTS.md` 硬约束）；新校验一律走 T3/T4 的 CLI+JSON/HTTP。

## 5. 不在本计划（记录在案的后续项/风险）

1. **波及闭包（cascade）**：当前已补旧/新 owner、children 差集和 transform 子树失效，
   但仍缺 `CATR/SPRE/SCOM`→所有引用实例、克隆/分布式副本及通用
   SignificantOwner + Members。→ 后续单独计划。
2. **placement-vs-mesh**：`POS`/`ORI` 改动理论上只需**变换刷新**（`pe_transform`）而非**网格重算**；当前 `attribute_affects_model` 把它们计为几何变化会触发重算。可与 `2026-03-29-transform-refresh-cross-repo-fix.md` 合并优化。→ 后续。
3. **DCHC/effect 权威管线**：从活 E3D 或扩展字典导入
   `DCHC/PLCF/wnoevt`，记录 `(noun, attr, DCHC, QCHGLS ref/code)`；分类升级为
   `(noun, attr, effect, raw_dchc)`。`wnoevt` 仅用于事件兼容。→ 见 ADR-0009 与逆向 §14.2。

## 6. 验收标准

- `attribute_affects_model` 覆盖 §3 全部缺口；§13.2 对白名单覆盖率 100%（T3 报告）。
- T3 报告无"白名单项不在 att_meta"的意外 typo（派生项除外并注明）。
- T4：几何编辑→入桶、元数据编辑→剔除，均符合预期（JSON 证据留档）。
- 不引入 `cargo test`；不改动 §5 所列范围。

## 7. 里程碑

- ✅ **M1（2026-07-22）**：T1+T2 白名单补缺与注释。补入 `src/version_management/model_impact.rs`：PX/PY/PZ/DX/DY、POSL/POSS/POSE/NPOS/CPOS、YDIR/ZDIR/BANG、ZDIS/LEAV/CURD/CURTYP/OPDI/ROUT/DRNS/DRNE/DETR、PSPE/CTYP/JFRE/JLIN、DELP/RINS/PKDI（其中 YDIR/ZDIR/DETR 系 M2 校验补抓）。无 lint 错误。
- ✅ **M2（2026-07-22）**：T3 字典覆盖体检 `scripts/verify_model_impact_attrs.py`（从源解析白名单 → 查 att_meta(702) → JSON）。结果：`geom_13_2_coverage_ok=true`（§13.2 100% 覆盖）、`new_gaps_all_in_allowlist=true`（全命中 att_meta、真实 dabacon 属性带 hash）；白名单 223 项 / 133 在 att_meta。
- ⏳ **M3**：T4 行为回归（挑一段有已知几何+元数据编辑的真实 sesno 区间跑增量、核对 `element_changes` 分类与 `IncrGeoUpdateLog`）——待有已知 sesno 区间时执行。
