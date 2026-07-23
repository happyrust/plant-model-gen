---
status: accepted
date: 2026-07-23
---

# 目录引用反向索引与波及闭包

## 背景与决策

增量模型生成需要在元件库定义（SCOM 及其下几何/尺寸）发生几何影响变更时，**反查所有引用它的设计实例**并纳入重生成目标（`目录反向波及闭包`，见 `CONTEXT.md`）。当前实现只把「被改 refno 自身」按 noun 入桶——改一个共享 SCOM 只重算被改元素，漏掉所有引用它的设计实例（`docs/reverse/incremental_update_vs_core_dll.md` §4.4 记为影响面最大的缺口）。

我们决定新建**主 PE/ATT 同源、MVCC 版本化**的引用边索引 `cata_ref_index`，存 **as-written 语法引用边** `{source_refno, source_dbnum, attribute(大写), ordinal, target_refno}`，覆盖**除 OWNER 外的全部 Ref/RefList**；反查读层只提供**一跳分页原语**，多跳传递闭包（`SPRE→CATR→…→SCOM`）与环/深度/effect 收敛由上层 expander 负责；接入以「索引 + shadow 先上、接管等项目 run barrier」的节奏推进，shadow 阶段以**项目级 ready 门 + 解析全扫描差分**验证，不改生成目标。

它是大计划 `pe_reference_edge`（`docs/plans/2026-07-23-incremental-model-impact-closure-refactor-plan.md` Q3/§5）的**先行目录子集**：列名对齐，将来收敛为其视图/改名，不重抽取；不与其冲突。

## 关键取舍（Considered Options）

- **边目标语义**：存 as-written 语法边（选）而非 resolved SCOM——不把 `get_or_create_scom_info` 解析逻辑烘焙进索引、与 `pe_reference_edge` 同构、解析规则变更不必重建索引。
- **属性覆盖**：全 Ref/RefList（除 OWNER，选）而非仅 CATR/SPRE 子集——避免硬编码盲区与二次 backfill；OWNER 已由 `pe_owner` 关系边承载。
- **版本化**：随主库 MVCC（选）而非 latest-only——被删 SCOM 仍能在 `read_at` 反查、历史 run 可复现，符合 `生成读取时刻` 一致性。
- **传递闭包位置**：读层一跳、BFS 归 expander（选）——读层可分页 / 可 checkpoint，对齐大计划 `ReferenceImpactRead` / `ImpactExpander` 分工。
- **ready 门**：项目级（选）而非 per-dbnum——跨库闭包下，任一 design 库未索引即会静默漏判。
- **shadow oracle**：解析全扫描差分（选）而非全量几何重生差分——定向、便宜、只证明索引找对实例；全量重生差分留作 P3 晋升门。

## 后果（Consequences）

- 存储只增、写放大：每次增量按 changed source `replace-by-source`，且**必须与 data anchor 同一 `persist_pdms_increment_grouped → commit_version` 事务**（AGENTS.md：禁止旁路直写）。删 source→tombstone；删 target→**不**级联（被删目标仍可反查引用者）。
- 语义红线：「设计实例自身改 `CATR/SPRE/PRTREF`」只 direct 重算该实例，**不**经此闭包向同引用兄弟扇出；只有「目录定义被改」才反向扇出。区分二者是正确性关键。
- 本 ADR 只固化索引与闭包语义；跨库 debt / 项目级 run 发布沿用大计划，P3 接管等项目 run barrier 就绪后按项目级落，本轮不建临时 per-db 扇出。
- 超集降级（热门 SCOM → `FullDb/FullProject`）在 P3/M4 处理，本 ADR 不含。
- 与 ADR-0009 关系：ADR-0009 定「属性→是否影响」的三态判定；本 ADR 定「目录定义变更→反查哪些实例」的波及闭包，二者正交互补。
