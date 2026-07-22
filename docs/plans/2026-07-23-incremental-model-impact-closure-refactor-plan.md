# 增量模型影响闭包重构方案

> 日期：2026-07-23  
> 状态：grill-with-doc 已定案，待 ADR/spec 冻结后实施  
> 依据：
> - `docs/reverse/core_dll_incremental_update_flow.md`
> - `docs/reverse/core_dll_noun_att_model_update.md`
> - `docs/reverse/incremental_update_vs_core_dll.md`
> - `docs/adr/0006-model-generation-watermark-debt-catchup.md`
> - `docs/adr/0010-initialization-and-incremental-version-boundary.md`
> - `specs/026-incremental-model-gen-debt-catchup/`
> - `specs/027-version-single-source-refactor/`

## 1. 目标

把当前“属性过滤 → 五桶 `IncrGeoUpdateLog` → per-dbnum 欠账 → 增量生成”
重构为可解释、可追赶、支持跨库引用闭包的模型影响规划流程。正确性目标是：

1. 已确认会影响模型的变化不能漏生成；
2. CATR/SPRE、owner/层级、删除等间接影响可形成确定闭包；
3. 生成失败后复用同一影响计划重试，不因代码路径或当前态变化而漂移；
4. 同一轮多库生成共享一个 `VERSION AT`，并可解释“为何重算这些目标”；
5. 不复制第二套模型生成器，最终仍进入现有 `GenerationTargets` /
   `execute_generation_targets`。

## 2. 已核实现状

当前主链是：

```text
PDMS sesno operations
  → classify_operation
  → apply_pdms_operation
  → apply_critical_model_expansion
  → IncrGeoUpdateLog（五桶）
  → data commit + per-dbnum model_gen_debt
  → catch_up_model_generation
  → resolve_incremental_generation_targets
  → execute_generation_targets
```

需要据新逆向证据纠正的旧口径：

- `wnoevt` 只是 core.dll 的数据库事件门；Rust 已从 history 得到变化，
  不需要复刻该门。
- Rust 的属性白名单更接近 Core3D 的 `DCHC != 0`，但 Core3D 的
  `DCHC/EVALAT` 还携带目标重定向与传播强度，不能长期只压成一个 bool。
- 当前已实现 OWNER 旧/新 owner 与 children 差集的局部扩展，不再是“完全没有
  层级传播”；仍缺删除旧 owner、通用 SignificantOwner/Members、CATR/SPRE
  反向引用、克隆/绑定副本以及显式 effect 分类。
- `model_gen_debt:[dbnum,to_sesno]` 和 catch-up 的
  `manual_db_nums=[dbnum]` 假设“变化库就是生成目标库”；CATALOG/SPEC →
  DESIGN 的跨库引用会打破该假设。

## 3. Grill 决策

| 编号 | 决策 | 状态 |
|---|---|---|
| Q1 | 跨库一致性边界 | 已定：项目级 `model_generation_run` + 输入水位向量 |
| Q2 | 影响分类的数据模型 | 已定：静态规则注册表 + 类型化 `EffectSet` |
| Q3 | 反向引用索引与闭包时机 | 已定：主库版本化全引用索引 + barrier 后固化计划 |
| Q4 | SignificantOwner / Members 粒度 | 已定：生成单元策略 + 产物依赖 DAG |
| Q5 | placement 与 mesh 是否分离 | 已定：独立失效通道，共享计划与提交屏障 |
| Q6 | 超大闭包的失败与降级策略 | 已定：可检查点精确闭包 + 显式全量提升 |
| Q7 | 迁移、验证与分批上线 | 已定：影子规划 → 兼容投影执行 → lane 灰度 |

### Q1 已定：项目级 generation run + 输入水位向量

采用项目级 `model_generation_run` 作为一次模型更新的唯一运行身份：

- 记录统一 `read_at`、所有输入 dbnum 的观测水位向量、源变化区间、
  影响规则版本、跨库目标闭包及结果；
- “源变化坐标”（例如 CATA dbnum/sesno）与“生成目标坐标”（例如 DESIGN
  实例及其 dbnum）分离；
- 现有 per-dbnum `model_gen` 锚点保留为覆盖水位，不再独自承担跨库模型历史
  身份；
- 同一 DESIGN sesno 下由不同 CATA/SPEC 变化产生的模型状态用不同 run id /
  `read_at` 区分，禁止覆盖成一个含糊的 per-db 状态；
- 承接 specs/027 已规划的多库 generation barrier 和
  `model_generation_run` 事件台账，不另造并行版本协议。

否决：

- per-db debt 向目标库强行扇出：无法可靠表达“目标库数据 sesno 未变、外部依赖
  已多次变化”的模型历史；
- 只实现同库闭包：不覆盖 CATA/SPEC → DESIGN 的主要真实场景。

### Q2 已定：静态规则注册表 + 类型化 EffectSet

不再让 `attribute_affects_model -> bool` 和五桶同时承担“是否更新、更新什么、
传播到哪里”三种职责。新增可序列化且带版本的影响模型：

```text
ImpactRule
  key: operation + optional noun + optional attribute
  output: direct effects + propagation modes + rule_id

ImpactSeed
  source change identity + before/after context
  matched rule_ids + EffectSet + propagation requests

EffectSet
  MeshRebuild | TransformRefresh | DeleteOutput | DataOnly

Propagation
  Direct | SignificantOwner | Members | OldAndNewOwner
  | ReverseReference(attribute family) | CloneOrBinding
```

约束：

- 规则匹配支持 `(noun, attr)`，用于 `SPCO.PRTREF` 等已确认的 noun-scoped
  特例；不能把所有影响继续塞进一张无上下文属性名单。
- 未知属性/UDA 保持 fail-safe：默认产生直接 `MeshRebuild`，不因规则缺失漏
  模型；明确的 NAME/DESC/PURP/FUNCTION 才是 `DataOnly`。
- `POS/ORI` 表达为 `TransformRefresh`。设计实例自身修改 `CATR/SPRE` 产生
  direct `MeshRebuild` 与引用边替换；目录定义变化才从所属 SCOM 按
  `ReverseReference(CATR/SPRE)` 反查实例，不能把二者混成“一改 CATR 就扇出所有
  同引用实例”。
- 删除可同时表达 `DeleteOutput + OldAndNewOwner/SignificantOwner`。
- 每个 seed/plan 持久化 `rules_version`、`rule_id` 和原因，欠账追赶与审计不再
  依赖当前二进制临时重新解释字符串。
- 现有五桶保留为 `ModelImpactPlan -> IncrGeoUpdateLog` 的兼容投影，供现有
  `GenerationTargets`/生成器消费；它不再是影响真相源。
- 暂不照抄 `DCHC=1..4` 数字：其官方枚举名未恢复。未来取得
  `att_meta.dchc` 后，只替换规则注册表的数据来源，不改变 Effect/Propagation
  领域接口。

否决：

- 在 bool 白名单旁继续堆特殊 `if`：传播语义仍会散落，无法稳定写入 debt；
- 等待完整 DCHC 导出后再动工：会阻塞已由静态证据确认的正确性修复。

### Q3 已定：主库版本化全引用索引 + barrier 后固化计划

建立主 PE/ATT 同源的版本化普通表 `pe_reference_edge`，代替即将随 specs/027
删除的 `generation_replica_reference`：

```text
pe_reference_edge
  source_refno, source_dbnum
  attribute_name, ordinal
  target_refno, target_dbnum
```

至少建立 `(source_refno)` 和 `(target_refno, attribute_name)` 索引。采用普通表而
非依附 target PE 生存期的 graph edge，保证目录定义被删后仍能从未变化的设计源
边反查引用者。

维护与闭包顺序：

1. 初始化/新 dbnum onboarding 从全部 PE/ATT 的 `Ref/RefList` 建索引；
2. 每次增量在同一 data commit 中按 changed source 全量替换其 outbound edges；
3. 数据与 seed debt 全部成功后进入项目 generation barrier，取得统一 `read_at`；
4. `ImpactExpander` 按 seed 的传播请求查询入边并做有环保护的 BFS；
5. 目录内部引用可以继续反向传播，直到命中 DESIGN 侧生成单元；设计实例自己修改
   `CATR/SPRE` 只产生 direct rebuild 和边维护，**不会**误触发同目标所有实例；
6. 先持久化不可变 `ModelImpactPlan`，再启动生成；重试优先复用同一 plan。

持久化分两层：

- `ImpactSeedDebt` 在数据提交后立即写，保留 specs/026“数据成功、模型失败仍可
  追赶”的不变量；
- `ModelImpactPlan` 记录 `run_id/read_at/rules_version`、闭包目标及 provenance。
  若闭包阶段失败，seed debt 保留，下轮仍按原 `read_at + rules_version` 重试；
  plan 一旦落盘不得按新当前态静默重算。

索引收录全部引用属性，传播时再由规则过滤。这样 `PRTREF`、目录内部引用及未来
规则不需要反复改表；只索引 CATR/SPRE 会再次制造硬编码盲区。

否决：

- 临时扫描 `pe.refno.CATR/SPRE`：大库无 target 索引，且不能表达传递引用；
- 采集期跨文件扫描：无法天然绑定多库统一切面，并把重活压进源文件采集热路径。

### Q4 已定：生成单元策略 + 产物依赖 DAG

不把 Core3D 的 `SignificantOwner + Members` 机械翻译为“owner 全子树重新生成”。
Core3D 的概念作为影响语义保留，落到本项目时由版本化 `GenerationUnitPolicy`
映射到最小生成单元：

```text
GenerationUnitPolicy
  key: noun + effect
  output:
    direct artifact kinds
    significant-owner selector
    member dependency selector
    downstream artifact edges

ArtifactTarget
  artifact kind + stable identity
  caused_by seed/propagation path + matched rule_id
```

首版产物种类至少覆盖：

- 叶子/loop mesh；
- catalog geometry 与 boolean 组合；
- design instance/instance relation；
- hierarchy/assembly relation；
- AABB、索引及导出节点。

闭包按有向产物依赖求解，而非把所有作用都压回 refno：

1. `MeshRebuild` 先映射到拥有该 mesh 的最小生成单元；
2. `SignificantOwner` 选择拥有组合产物的 owner；`Members` 只纳入该产物声明依赖
   的成员，不默认遍历 owner 全子树；
3. `TransformRefresh` 只使实例变换及其下游 AABB/导出失效，除非规则同时要求
   `MeshRebuild`；
4. 删除/移动同时使用 before/after owner：清理旧产物，并使旧、新组合 owner 的
   依赖产物失效；
5. 产物 DAG 去重后写入 `ModelImpactPlan`，生成器只消费已固化目标。

`EffectSet` 描述“发生了什么”，`GenerationUnitPolicy` 描述“本项目哪些产物因此
失效”，两者不可合并成新的 noun/attribute 条件链。策略与影响规则一样带版本；
计划持久化命中的 policy id，便于解释为何某个未直接变化的目标被重算。

迁移期间，`ArtifactTarget` 再投影为现有 `IncrGeoUpdateLog` 五桶和
`GenerationTargets`；兼容投影允许过算，但禁止漏掉已固化的 typed target。

否决“上卷 owner 后重算全部目标子孙”：它虽保守，却会在 EQUI/BRAN 等大子树
产生无界放大，而且不能表达 mesh 不变但关系/AABB 失效的情况。

### Q5 已定：placement 与 mesh 独立失效通道

`TransformRefresh` 与 `MeshRebuild` 在同一 `ModelImpactPlan` 中分别形成执行 lane，
共享 `run_id/read_at` 和最终提交屏障，但不再强制共同执行：

```text
cleanup/delete lane
        │
        ├── mesh lane ─────────┐
        │                      ├── relation/AABB/export lane
        └── transform lane ────┘
```

- mesh lane 只接收 `MeshRebuild`，负责 catalog/primitive/loop/boolean 几何及 mesh
  AABB；CATR/SPRE 变化先解析目标 `cata_hash`，已有正确 mesh 时只需改实例绑定；
- transform lane 接收 `TransformRefresh`，定向失效并重算 world transform。
  POS/ORI 影响自身及后代 placement；OWNER 变化覆盖移动子树并同时处理旧、新父级
  关系；
- relation/AABB/export lane 显式依赖它所需的 mesh/transform 结果，不再依靠完整
  mesh 管线的偶然副作用刷新；
- delete lane 清理被删实体的 mesh/实例/transform/空间输出，并向下游传递旧 owner
  和旧绑定的失效。

执行要求：

1. lane 任务以 `(run_id, artifact target)` 幂等，失败只重试未完成 lane；
2. run 未通过最终屏障前不推进覆盖水位，也不把部分 lane 标为完整模型状态；
3. 同时具有两个 effect 的目标可共享一次读取，但完成状态分别记录；
4. 当前“所有增量 refno 先失效 transform，再统一进入 mesh 分类”的路径只作为迁移
   兼容层；最终由 plan 精确驱动；
5. `pe_transform` 不再仅是 mesh 生成的副产品：transform lane 必须可以独立持久化，
   并使相关内存 cache 与数据库行使用相同失效根。

这样 POS/ORI/纯 OWNER placement 变化不再无条件 tessellate mesh，同时 CATR/SPRE
和布尔输入变化仍能明确要求几何 lane。计划层的 effect 分离与执行层一致，避免
“类型上区分、运行时仍全部重算”的长期半迁移状态。

### Q6 已定：可检查点精确闭包 + 显式提升生成范围

节点数、边数、耗时和内存预算只控制调度方式，绝不作为语义截断条件。计划状态机
至少为：

```text
SeedPending
  → Expanding(checkpoint...)
  → Ready(ExactTargets | FullDb | FullProject)
  → Executing(lane checkpoints...)
  → Committed
```

精确闭包：

- `ImpactExpansionCheckpoint` 持久化固定 `read_at/rules_version`、有序 frontier、
  target/edge 游标、已合并 effect、统计量及校验摘要；
- 按确定顺序分页查询 `pe_reference_edge` 与 hierarchy，批次间可安全恢复；
- visited 不是简单 refno 集合，而是节点当前 effect/propagation 状态的单调格；
  新路径带来更强 effect 时重新入队，循环引用最终在有限状态上收敛；
- 暂态查询/存储错误保留 checkpoint 和 seed debt，不创建部分 `Ready` plan。

成本超过阈值时不截断 frontier，而把目标提升为明确的超集范围：

1. 单个生成目标库的受影响比例或计划字节数超过阈值，使用
   `FullDb(dbnum, artifact kinds)`；
2. 多库广泛受影响或逐库全量仍更贵，使用
   `FullProject(artifact kinds)`；
3. scope target 与 exact target 一样写入计划、原因和成本快照；提升只能扩大，
   不得回退为采样目标；
4. full scope 仍遵守 Q5 的 lane：transform 全量不自动等于 mesh 全量。

配置至少包含 soft chunk budget、scope promotion ratio/bytes 和 hard worker memory
limit。soft limit 触发 checkpoint，promotion threshold 选择更便宜的正确超集；
hard limit 只让 worker 可恢复失败，不得提交不完整计划。指标记录 seed 数、
expanded nodes/edges、去重率、最大深度、计划字节数、提升原因和预计/实际成本。

若原 `read_at` 因保留策略不可读，禁止偷偷按当前态完成旧 run。创建一个显式
`SupersedingFullRun`，其输入水位覆盖旧 seed 的 source range；只有该全量 run
成功后才能把旧债标记为 `SupersededBy(run_id)` 并推进覆盖水位。

因此热门 SCOM 可以从几十万条离散 target 提升为一次顺序全库生成，但审计上仍能
区分“精确闭包完成”“成本提升为正确超集”和“旧快照过期后由新全量 run 覆盖”。

### Q7 已定：影子规划 → 兼容投影执行 → lane 灰度

采用四态开关，而不是一次性切换：

```text
legacy
  → shadow
  → planned-compat
  → planned-lanes
```

- `shadow`：新索引、新 seed debt 和 planner 真实运行并持久化计划，但生产输出仍由
  legacy 五桶驱动；对比目标集合、原因、成本与全量重生结果，不消费新 debt。
- `planned-compat`：`ModelImpactPlan` 成为真相源，先投影为
  `IncrGeoUpdateLog/GenerationTargets`，继续复用现有生成器。兼容投影允许多算，
  任一 typed target 无法投影时必须 fail closed，禁止丢弃。
- `planned-lanes`：按项目或 dbnum allowlist 灰度独立 mesh/transform/delete/
  downstream lane；未灰度产物仍走兼容 executor，但共享同一 plan 与最终屏障。
- `legacy` 只用于影子期的即时退回。新 planner 已接管覆盖水位后，退回 legacy
  不能消费其无法表达的跨库 seed；必须保留计划重试或执行 superseding full run。

每级晋升都要求固定 fixture、真实 sesno、全量重生差分、故障注入和性能门通过。
双执行只在 staging/抽样项目短期使用，不在生产长期支付两套几何生成成本。

## 4. 目标架构

### 4.1 端到端数据流

```text
PDMS operations
  │
  ├─ capture before/after context
  ├─ ImpactRuleRegistry.classify
  │      └─ ImpactSeed[]
  └─ derive outbound reference edges
           │
           ▼
  one dbnum data transaction
    PE / ATT / hierarchy
    + replace pe_reference_edge by source
    + incremental data anchor
           │
           ▼
  immutable ImpactSeedDebt (失败仍按 coverage hole 处理)
           │
  all-db generation barrier
           │
           ▼
  ProjectGenerationRun
    read_at + input watermark vector
           │
           ▼
  ImpactExpander
    rules + hierarchy + reverse references + artifact policy
    + checkpoint / scope promotion
           │
           ▼
  immutable ModelImpactPlan
    exact targets | FullDb | FullProject
           │
           ├─ delete/cleanup
           ├─ mesh
           ├─ transform
           └─ relation/AABB/export
           │
           ▼
  final project publication transaction
    successful run event + model_generation_head
    + source coverage anchors + consume/supersede debts
```

分类发生在源 operation 上，闭包发生在 data commit 后的统一 `read_at`。前者保留
删除前/移动前事实，后者保证跨库查询看到一致的 PE/ATT、层级和引用索引。

### 4.2 核心领域类型

建议把现有 `src/version_management/model_impact.rs` 收敛为模块，首版接口如下：

```text
ChangeIdentity
  source_dbnum, sesno, operation_ordinal, refno

ElementImpactContext
  dbnum, refno, noun, owner
  relevant_reference_attributes

ImpactSeed
  seed_id = hash(change identity + canonical payload)
  operation
  before: optional ElementImpactContext
  after: optional ElementImpactContext
  changed_attributes
  matched_rule_ids
  effects: EffectSet
  propagation_requests

ImpactRuleRegistry
  rules_version = hash(canonical rules)
  classify(operation, noun, attribute, context) -> RuleMatch[]

GenerationUnitPolicyRegistry
  policy_version = hash(canonical policies)
  map(seed/effect/node) -> ArtifactTarget[] + dependency requests

ArtifactTarget
  artifact_kind
  stable identity (refno | cata_hash | scoped dbnum/project)
  effects / lane
  provenance_node_id

ModelImpactPlan
  plan_id = hash(run input + rule/policy versions + canonical targets)
  run_id, read_at, input_watermarks, source_ranges
  rules_version, policy_version
  target_scope, targets, dependency edges
  expansion metrics and provenance root
```

`before/after` 只保存规划必需的稳定事实，例如 noun、owner、CATR/SPRE/PRTREF
目标；不复制整条 ATT。删除必须有 before，新增必须有 after，OWNER/NOUN/引用
修改必须同时保存相关 old/new 值。缺少必需 preimage 时不得猜测，seed 标成
`NeedsFullScope` 或 coverage hole。

effect 合并必须满足幂等、交换、结合，并定义强度：

```text
DataOnly < TransformRefresh < MeshRebuild
DeleteOutput 与其余 effect 正交
```

这只是闭包收敛格，不表示 transform 必须执行 mesh；lane 仍按 Q5 分开。

### 4.3 项目级发布与读取可见性

仅有“最后写 model_gen 锚点”不足以防止运行期间读取到部分 lane 新结果。新增
`model_generation_head` 作为项目模型可见性指针：

1. 所有生产模型读取先解析当前成功 head，再以其 `output_committed_at` 执行
   `VERSION AT`；运行中的零散写入对读者不可见。
2. 所有 lane 成功后，在一个最终事务中追加 run success、更新 head、推进所有
   **源变化库**覆盖锚点并追加 debt consumed/superseded 事件。
3. 任一 lane/后处理失败不移动 head；部分写入仅存在于 head 之后的 MVCC 历史，
   重试按同一计划幂等覆盖。
4. per-db `model_gen` 锚点只回答“该源库变化已覆盖到哪个 sesno”，不再独自代表
   项目模型快照。
5. 同一 DESIGN sesno 可能对应多个 CATA/SPEC 输入状态。精确历史 API 接受
   `run_id`；兼容的 `(dbnum, sesno)` API 必须返回其实际选择的 run id，不能声称
   sesno 唯一确定跨库模型状态。

最终事务应给所有覆盖锚点和 head 使用同一数据库提交时刻。正式启用
`planned-compat` 前，所有 latest 模型读路径必须已改成 head-pinned；否则最终屏障
只是审计记录，不是真正的可见性屏障。

同一项目严格按 source range 顺序发布：前一个 pending run 未完成时，后一个 run
不得移动 head。若放弃前一个 run，只能用覆盖其全部 target db/artifact kind 的
superseding full run；否则早期失败 run 的部分 MVCC 写可能在更晚 head 下泄漏。

Surreal 行以 `VERSION AT` 隔离；文件产物必须写 content-addressed/run staging
路径，并由 head 中的 immutable manifest 指针发布。不得在 run 成功前原地覆盖
当前 head 引用的 parquet/导出文件，旧文件清理只能作为 head 发布后的异步 GC。

## 5. 存储、索引与读取接口

### 5.1 新增/演进表

| 表 | 关键身份 | 用途与不变量 |
|---|---|---|
| `pe_reference_edge` | source + attribute + ordinal | 主 PE/ATT 同源的全 Ref/RefList 索引；target 用稳定 refno 值而非 graph edge 生存期 |
| `pe_reference_index_state` | dbnum | backfill/ready 水位、行数、校验摘要；所有活动库 ready 前 planner 不得接管 |
| `model_impact_seed_debt` | seed_id | 不可变 source change/preimage/rule match；仅追加消费或 supersede 事件 |
| `model_impact_seed_event` | seed_id + event ordinal | append-only pending/consumed/superseded 状态，不原地改 seed payload |
| `model_impact_expansion_checkpoint` | run_id + shard | frontier、游标、effect lattice、统计与校验摘要 |
| `model_impact_plan` | plan_id | 不可变 header、输入向量、版本、scope 和内容摘要 |
| `model_impact_plan_target` | plan + lane + artifact key | 大计划分页消费；禁止把几十万 target 塞入单行数组 |
| `model_impact_provenance` | plan + node id | 去重后的原因 DAG，记录 seed/rule/传播边/policy |
| `model_generation_run_event` | run_id + event ordinal | 扩展 specs/027 的 append-only started/checkpoint/terminal 台账 |
| `model_generation_head` | project singleton | 指向最后成功 run 与 `output_committed_at`，是模型读取可见性根 |

`model_gen_debt` 在 shadow/planned-compat 阶段保留双写。新链稳定后停止创建五桶
debt，但保留只读迁移器，直到所有存量行被消费或显式由 full run supersede。

### 5.2 引用索引事务语义

- 新增/修改 source：从提交后的 canonical ATT 提取全部 `Ref/RefList`，在同一数据
  事务内删除 source 当前 outbound rows 并写入新集合。
- 删除 source：同事务 tombstone 其 outbound rows；旧 `VERSION AT` 仍可见。
- 删除 target：不级联删除其它 source 的边。这样删除的 SCOM 仍能反查设计引用者。
- `attribute_name` 写规范大写，数组 ordinal 稳定；重复 target 不得错误去重不同
  attribute/ordinal。
- backfill 在项目 mutation lock 和固定 data anchor 下分页执行；逐库比较
  `AttributeSet::reference_edges()` 的 count/hash，成功后才标记 ready。
- 新 dbnum onboarding 必须把 reference index 完整性纳入 baseline 发布门。

### 5.3 领域读取 API

SQL 继续只存在于 Surreal adapter。为 planner 增加批量、分页能力，而不是在循环中
发 N+1：

```text
ReferenceImpactRead
  load_inbound_references(targets, attribute_families, cursor, limit)
  load_outbound_references(sources)

HierarchyImpactRead
  load_nodes(refnos)
  load_parents(refnos)
  load_children(parents, selector, cursor, limit)

ImpactPlanRepository
  append_seeds / load_contiguous_seed_ranges
  save_checkpoint / publish_immutable_plan
  claim_lane / complete_lane / fail_lane
  commit_run / supersede_run
```

这些接口绑定同一个 `VersionedReadSession/read_at`。planner 不能调用全局数据库，
executor 不能在计划之外重新解释属性并偷偷扩展目标；若执行时发现缺目标，计划
失败并记录 invariant violation，不能原地改 plan。

## 6. 首版规则与传播语义

| 变化 | direct effect | 传播/所需上下文 |
|---|---|---|
| Added | 由 noun policy 决定 mesh/transform | Direct + SignificantOwner；after 必需 |
| Deleted | DeleteOutput | before owner、old binding、SignificantOwner、必要的 reverse refs |
| POS/ORI | TransformRefresh | Direct + placement descendants；不默认 MeshRebuild |
| OWNER/CHILDREN/reorder | TransformRefresh | OldAndNewOwner + moved subtree + relation downstream |
| NOUN/TYPE | DeleteOutput + 新 noun effects | before/after policy 都执行 |
| 设计实例 CATR/SPRE | MeshRebuild 或绑定刷新 | Direct；替换 source outbound edge，不反查同目标兄弟实例 |
| SCOM/目录内部几何属性 | MeshRebuild | 上卷所属定义单元，再 ReverseReference(CATR/SPRE) 到实例 |
| `SPCO.PRTREF` 等 noun 特例 | registry 指定 | `(noun, attr)` rule，不能退化成全局属性名 |
| NAME/DESC/PURP/FUNCTION | DataOnly | 无模型 target |
| 未知属性/UDA | MeshRebuild | Direct + SignificantOwner fail-safe；若落在 CATA/SPEC definition，再按 binding family 反查，并输出 `unknown_fallback` |

CATR/SPRE 目录级闭包按以下顺序，避免误把“实例换引用”和“定义被修改”混为一谈：

1. 从变化元素按 policy 上卷到 catalog/spec definition root（例如 SCOM）；
2. 只对该 definition root 发起允许的 incoming attribute-family 查询；
3. 命中的 catalog 中间节点继续按规则传播，命中的 DESIGN instance 形成 typed
   target；
4. 实例按新 `cata_hash` 判断 mesh 是否缺失/失效，并始终更新 binding、AABB 等
   必需下游；
5. provenance 记录完整 seed → definition root → reference edge → instance 路径。

Core3D 的 DCHC 数字值只作为未来规则数据源，不进入领域枚举。DCHC 官方名称、
`DB_UserChanges → QCHGLS` 单一桥接点和运行时全扇出断点仍未闭合，不阻塞上述
fail-safe 首版。

## 7. 与现有代码的迁移映射

| 当前位置 | 目标变化 |
|---|---|
| `version_management/model_impact.rs` | 拆为 rules/types/classifier/policy；bool API 仅作 legacy wrapper |
| `data_interface/sesno_increment.rs::classify_operation` | 生成 typed seed，并捕获 before/after；不在采集期做跨库闭包 |
| `apply_critical_model_expansion` | shadow 期保留；planned 模式由 planner 的 owner/member propagation 取代 |
| PE/ATT 增量持久化 | 同事务维护 `pe_reference_edge` |
| `versioned_db/model_gen_debt.rs` | 双写/迁移到 seed debt + project coverage；五桶不再是真相源 |
| `model_gen_catchup.rs` | 从 per-db 合并五桶改为按项目输入向量恢复/复用 plan |
| `generation_read::{traits,surreal}` | 增加 versioned inbound-reference 与分页 hierarchy 能力 |
| `gen_pipeline::resolve_incremental_generation_targets` | planned-compat 接受 plan 投影；最终由 typed executor 取代 |
| `orchestrator.rs` 的全量 transform 失效 | 改为 transform lane 的精确 root 与持久化 |
| `write_pipeline/model_writer` | lane 幂等状态、typed artifact 完成证据与 head-pinned 发布 |

兼容投影的硬规则：

- 每个 `MeshRebuild` target 必须能映射到 bran/hang、loop、cate、prim 或显式 full
  scope；无法映射即失败。
- `TransformRefresh` 在 planned-compat 可暂时映射成 legacy 可见 refno 并过算 mesh，
  但指标必须单列 `compat_mesh_overgeneration`。
- `DeleteOutput` 必须保留 tombstone/cleanup 目标和 before owner，不能只投影
  `delete_refnos` 后丢失 owner downstream。
- 投影输出排序、去重并计算 hash；同一 plan 重试必须得到相同 hash。

## 8. 对 specs/026、027 与 ADR 的修订

实施前新增 ADR（建议 `ADR-0011-project-model-impact-plan-and-reference-closure`），
并显式 supersede ADR-0009 的“属性 allowlist 即最终判定”部分。

specs/026 必须修订：

- FR-001/003：五桶 per-db debt 改为 typed seed debt + project plan；五桶降为兼容
  投影。
- FR-002 保留 per-db source coverage，但不再把它解释为完整模型历史身份。
- FR-004/009：数据提交仍 per-db 隔离；模型发布改成项目 run 原子屏障。任一 lane
  失败时整个 run 不推进 head/coverage。
- FR-005：全量不只是人工补洞，也可由 planner 以可审计的 `FullDb/FullProject`
  正确超集自动提升；裸 `--allow-full-regen` 仍禁止。
- FR-008：替换为 versioned rule registry/EffectSet，未知 fail-safe 语义不变。
- FR-011：复用现有管线只约束 planned-compat 阶段，不禁止最终独立 lane。

specs/027 必须修订：

- FR-007/018 的多库统一 `VERSION AT` 与 barrier 作为本方案硬前置。
- FR-011 的“五桶语义 MUST 保留”改为迁移期兼容，不再是最终存储契约。
- FR-019 的 run ledger 增加 rules/policy/plan id、target dbnums、source ranges、
  expansion/lane 状态和 supersession。
- T020–T025 从 per-db catch-up 重排为 project seed coverage/plan recovery。
- T026 的主表 adapter 增加反向引用查询；删除
  `generation_replica_reference` 前必须先有 `pe_reference_edge`。
- T028 同时落地 `model_generation_head`，模型服务读取改为 head-pinned。

上述 spec/ADR 变更先合入再写业务代码，避免实现过程中同时服从互相冲突的
“per-db 五桶最终态”和“项目级 typed plan”两份契约。

## 9. 分阶段实施任务

### M0 — 契约与安全前置

1. 新增 ADR-0011，修订 specs/026、027 的 requirements/tasks/verification。
2. 完成 specs/027 的项目 mutation lock、统一 `GenerationReadSpec` 与 run event
   repository 前置。
3. 盘点所有生产模型读路径；设计并验证 `model_generation_head` 的
   `VERSION AT` 解析，不允许 planned 写入先于可见性隔离上线。
4. 固化当前真实增量 fixture、全量输出 manifest 和性能基线。

门：文档无契约冲突；source snapshot 可恢复；legacy 行为基线可重复。

### M1 — 类型化分类与 seed 双写

1. 落地 EffectSet、Propagation、ImpactRuleRegistry、rules hash 与 explain API。
2. 从 operation 捕获 before/after，生成确定 seed id；覆盖删除、OWNER、NOUN 与
   Ref/RefList old/new。
3. 新建 seed debt repository；与现有五桶 debt 双写、互相记录关联 id。
4. `impact-plan explain --change/--seed --json` 输出命中规则和 fallback。

门：shadow 分类不改变生产目标；相同输入 seed/rules hash 稳定；未知属性不漏。

### M2 — 主引用索引

1. 建 schema/index/state；实现 canonical ATT → edge extractor。
2. 增量 PE/ATT 事务内 replace-by-source，删除 source/target 语义按 §5.2。
3. 受锁 backfill 与 resume，逐库 count/hash 验证；接入新库 onboarding。
4. 提供 `reference-index audit --dbnum --at --json`。

门：所有活动 dbnum ready；随机 source 与全扫描 reference extraction 一致；
`VERSION AT` 前后边集合可复现。

### M3 — 项目 run、planner 与 shadow

1. 实现 project run input vector、contiguous seed coverage 与统一 read session。
2. 实现 owner/member/reverse-reference/artifact policy 闭包、provenance DAG。
3. 实现 checkpoint、effect lattice、scope promotion 与 immutable plan repository。
4. shadow 生成计划并同 legacy/full regeneration 差分；不消费 seed debt。
5. 落地 head-pinned model reads，但生产 head 仍由 legacy finalizer 更新。
6. 在所有 legacy debt 已覆盖且 reference index ready 后建立 `PlannerBaselineRun`，
   绑定当时成功 head 与全输入水位向量；typed coverage 只从该向量之后计算。若
   当前状态无法证明一致，则先做一次受控 FullProject baseline。

门：跨库 CATA/SPEC fixture 命中全部 DESIGN 引用实例；环收敛；重启恢复后 plan id
不变；旧读 API 返回所选 run id。

### M4 — planned-compat 接管

1. 实现 plan → 五桶/`GenerationTargets` 的 fail-closed 投影。
2. project finalizer 在单事务更新 success event、head、source coverage 和 debt。
3. watch/increment/catch-up/repair 统一调用 project planner/executor。
4. 小项目 allowlist 灰度；保留 legacy/shadow 切换和 superseding full run 救济。

门：生成结果与同 read_at 全量重生内容一致；运行中读者仍看到旧 head；任一点失败
不移动 head/coverage；投影过算率可观测。

### M5 — 独立 transform/delete lane

1. 抽出 transform root invalidation/recompute/persist API，支持 OWNER 移动子树。
2. delete cleanup 使用 before owner/binding，并驱动 relation/AABB/export 下游。
3. lane claim/checkpoint/idempotency 与依赖 barrier。
4. 按 dbnum/project allowlist 灰度；POS/ORI fixture 断言 mesh hash/写次数不变。

门：placement-only 不 tessellate；lane 失败可单独重试；最终结果仍与全量一致。

### M6 — typed artifact executor

1. 让 mesh、catalog boolean、instance relation、hierarchy、AABB/export 原生消费
   `ArtifactTarget`。
2. 删除执行期 noun 重分类与隐式下游刷新；所有依赖来自 plan DAG。
3. 启用 `FullDb/FullProject(artifact kinds)` scope executor。
4. 收敛 `GenerationUnitPolicy`，以运行指标调优而不改变 rule 历史。

门：typed target 全覆盖；compat projection 使用率为零；无 N+1 与无界 target 数组。

### M7 — 旧链退役

1. 停止写 `model_gen_debt` 五桶，迁移/消费全部存量 debt。
2. 删除 `attribute_affects_model` 作为权威入口、`apply_critical_model_expansion`
   和 per-db catch-up 假设；可保留只读兼容/诊断工具一个发布周期。
3. 移除 legacy/planned-compat 开关前完成生产观察窗与 full-run 恢复演练。
4. 更新 CONTEXT、ops、CHANGELOG、API schema 与运维手册。

门：无存活 legacy debt；所有项目 reference index ready；回滚演练证据归档。

## 10. 迁移、回退与运维约束

- schema 全部先加后删；shadow 阶段写新表失败按覆盖洞报告，但不篡改 legacy 成功
  结论。planned 模式下新表失败必须阻断模型发布。
- backfill 不从模型副本恢复，只从固定 data anchor 的主 ATT 重建；checkpoint 包含
  anchor 与 extractor version，版本变化必须重开 backfill。
- planned 模式回退不能简单切 `legacy` 后消费 typed debt。安全路径只有：
  继续用已固化 plan 的兼容 executor，或运行覆盖输入向量的 superseding full run。
- 不允许跳过较早的 failed/pending plan 发布更晚的窄增量 run；superseding run
  必须覆盖前者已经写过或计划写入的全部 target db/artifact kind。
- rule/policy 版本部署后保留旧版本解释器，至少覆盖所有未消费 debt 与未终结 plan；
  不允许只保留“当前规则”。
- plan 不可编辑。修正规则需创建新 plan/run，并通过 `supersedes_plan_id` 建立关系。
- 自动 FullDb/FullProject 提升受资源配额和并发控制，但不需要人工正确性授权；人工
  可以暂停/改调度优先级，不能批准截断闭包。

## 11. 验证矩阵

### 11.1 必测行为

1. DESIGN 实例 CATR 从 SCOM-A 改为 B：只直接更新该实例及其下游，不波及仍引用
   A/B 的其它实例。
2. SCOM-A 尺寸/primitive 修改：跨 dbnum 反查所有引用实例；DESIGN sesno 不变，
   run id/read_at 改变。
3. catalog 内部多跳引用与循环：effect lattice 收敛、provenance 可解释。
4. POS/ORI：自身/后代 transform 与 AABB 更新，mesh 内容 hash 和 mesh writer
   count 不变。
5. OWNER 移动：旧、新 owner 关系与移动子树 transform 正确；两侧组合产物刷新。
6. 删除 primitive/loop/SCOM/设计实例：latest 清理，旧 head/run 仍可历史查询；
   before owner/binding 缺失时 fail closed。
7. NAME/DESC：DataOnly no-op run 推进 source coverage，不改模型内容。
8. 未知属性/UDA：direct/significant-owner fail-safe；目录上下文继续反查绑定实例，
   并有 `unknown_fallback` provenance。
9. 热门 SCOM：超过阈值提升 FullDb/FullProject，不遗漏、不生成超大单行计划。
10. 同一 DESIGN sesno 连续两次 CATA 变化：两个 run 均可按 run id 重现，兼容 API
    明示选择哪一个。

### 11.2 故障注入

在以下边界逐一 crash/error：

- data commit 前、reference edge replace 中、data anchor 后 seed 写入前；
- 多库第二库失败、seed coverage hole、generation barrier 后；
- expansion 分页中、checkpoint 写后、plan publish 前后；
- delete/mesh/transform/downstream 各 lane 中；
- 全部输出完成但 final publication transaction 前；
- head 更新事务失败、终端事件写失败、原 read_at 过期。

每个用例断言：data 事实是否保留、seed/plan 是否可恢复、head/coverage 是否未误
推进、重复执行是否同 plan/target hash、是否需要显式 superseding run。

### 11.3 差分与性能门

- 在同一 data `read_at` 分别执行计划生成与全量生成，比较 mesh 内容 hash、instance
  relation、world transform、AABB、boolean 结果和导出 manifest；比较内容，不比较
  运行时间戳。
- 对未命中 target 的产物抽样断言前后 hash 不变；对 cleanup target 断言墓碑可见。
- shadow 报告 legacy-only、planned-only、共同目标、原因与 full oracle 差异，不能
  只比较总数。
- 性能记录 reference query pages、expanded nodes/edges、dedup ratio、plan bytes、
  lane wall time、mesh avoided count 和 scope promotion；禁止按单条 target N+1。
- 遵守仓库验证约束：以 CLI `--json`、`db-data/*.surql` 和
  `scripts/smoke/model_impact_*.ps1` 固化证据；是否运行 Rust test 以当时
  `AGENTS.md` 为准。

### 11.4 晋升门

| 晋升 | 必须满足 |
|---|---|
| legacy → shadow | seed 双写、引用索引 audit、规则 explain 稳定 |
| shadow → planned-compat | 核心 fixture 无 planned 漏项；head-pinned reads 上线；故障注入全绿 |
| planned-compat → planned-lanes | 全量差分一致；兼容过算量有基线；lane 可重试 |
| allowlist → 默认 planned-lanes | 生产观察窗无 coverage/head invariant 违例；全量恢复演练完成 |
| 删除 legacy | legacy debt 清零；旧规则可解释窗口结束；回退只依赖 plan/full run |

## 12. 明确非目标与未解项

- 本轮不恢复 DCHC=1..4 的官方枚举名，也不要求位对位复刻 Core3D 内部容器。
- 本轮不保证与 Core3D 完全相同的过算集合；目标是本项目产物语义下不漏且可解释。
- 不在采集热路径扫描全库引用，不新增第二套长期几何生成器。
- 不把 mesh GC、项目成员版本轴或 dbnum 整库移除纳入本重构。
- 仍建议用 Core3D 运行时断点验证“SCOM 变化 → 所有引用实例重建”的完整动态链；
  新实现的 fail-safe 与差分门不依赖该证据才能开始。
