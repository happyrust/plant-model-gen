# Feature Specification: 按版本实时查询模型树（versioned pe_owner）

**Feature Branch**: `023-versioned-model-tree-query`

**Created**: 2026-07-19

**Status**: Draft

**Input**: User description: "分析现在模型树的接口，加上版本后按版本显示模型树：通过指定某个 version，模型树实时地按指定版本查询出来；继续使用 pe_owner 的 edge 来实现子节点的查询；在解析时也要使用版本化的 pe_owner 来存储。"

## 修订记录（2026-07-21）

> GenPipeline 清理（`docs/superpowers/specs/2026-07-21-gen-pipeline-cleanup-rename-design.md`）已退役 TreeIndex / `.tree` 运行时与生产路径。  
> **FR-005 / 决策 #5 原「不传 sesno 走 TreeIndex」已被取代**：latest（不传 sesno）与 versioned 路径均走 pe_owner（及 pe.children 回退）。下文历史表述保留为起草时上下文，实现以 pe_owner 为准。

## 背景与决策记录（2026-07-19 会话结论）

本 spec 由模型树接口分析会话产出，关键决策与已核实事实如下：

| # | 决策点 | 结论 |
|---|--------|------|
| 1 | 实现路线 | **实时版本查询**：树的层级用 versioned 存储里的 `pe_owner` 边按 `VERSION $t` 现场查出，**不物化 per-sesno tree 产物**（按锚点拷贝 `.tree` 文件的方案已被否决） |
| 2 | 层级数据源 | 继续使用既有 `pe_owner` 关系表：`id = pe_owner:[<owner键>, <子序号>]`，`in`=子、`out`=父，id 自带同胞顺序 |
| 3 | 写路径 | 解析（全量 + 增量）都必须维护 pe_owner；实例级 `versioned=true`（specs/022）使其写入自动带版本，无需新表 |
| 4 | 版本入参 | 沿用 specs/022 锚点体系：业务入参为 per-dbnum 的 `sesno`，经 `sesno_version_anchor` / `fn::sesno_version(dbnum, sesno)` 换算存储时间戳后发起 VERSION 查询；禁止绕过锚点裸查 |
| 5 | 兼容行为 | 树接口不传 sesno 时行为与性能完全不变（继续走 TreeIndex `.tree` 路径）；版本模式是增量能力 |
| 6 | 兜底 | `pe.children` 字段（全量与增量两条路径一直在维护、含顺序）作为版本查询的保底/校验数据源；功能上线前的历史锚点自动回退到该字段 |

**已核实的现状事实**：

- `pe_owner` 边已存在，但**只有全量解析写入**（`versioned_db/pe.rs::save_pe_relates` → `INSERT RELATION INTO pe_owner`）；增量链路（`sesno_increment.rs::persist_pdms_increment_grouped`）只写 pe/ATT/ATT_UDA/dbnum_info，**完全不维护 pe_owner**——这是本 feature 要补的核心缺口，否则任何一次增量之后 pe_owner 的历史即不可信。
- pe 记录本身带 `children` 数组字段（含顺序），全量（pe.rs 注入）与增量（`inject_children_into_pe_json`）都在维护。
- 当前树接口（`/api/e3d/*`）层级查询统一走 `.tree` 文件（TreeIndex），单版本（最近一次解析）；名称运行时取 pe 当前值；均无版本参数。
- specs/022 已交付：实例级 versioned 存储、`sesno_version_anchor`（不可变、带 fingerprint）、`resolve_anchor`/`snapshot_at`/history CLI、`/api/model-history/*`、retention 语义（窗外报"历史已过期"）。
- 旧的手工备份机制（`fn::backup_owner_relate`/`old_pe`）为 022 之前的遗留，本 feature 不复用。

**待验证的技术前提**（实现前必须先跑能力 smoke，用 `db-data/run_surrealkv_versioned.ps1` fixture）：

1. fork surreal 对 `VERSION $t` + 图遍历（`pe:X<-pe_owner`）的支持度；
2. `VERSION $t` + record id 区间扫（`pe_owner:[<owner>,0]..`）的支持度；
3. 已存在 id 上 `INSERT RELATION` 的行为（决定全量重灌幂等策略）。

若 1/2 均不支持，children 查询退化为 `SELECT VALUE children FROM pe:<owner> VERSION $t` 纯点查（022 已验证的用法），功能不受阻，pe_owner 边路线保留图语义与反向查询价值。

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 指定版本浏览模型树 (Priority: P1)

设计审查人员在前端树组件上选择某个已固化的版本（dbnum + sesno 锚点），逐层展开模型树，看到的层级结构、节点名称与类型均为该 sesno 时刻的状态——包括当前已被删除的节点与后来被移动的子树。

**Why this priority**: 这是本 feature 的核心价值——不重扫 PDMS 源文件、不物化产物，实时回答"那个版本的树长什么样"。

**Independent Test**: 在 versioned fixture 上造两个锚点（sesno N 改层级前、N+1 改层级后：含新增子、删除子、改名），分别用 `children?sesno=N` 与 `children?sesno=N+1` 查询同一父节点，返回的子节点集合、顺序、名称与各自时刻一致。

**Acceptance Scenarios**:

1. **Given** 元素 P 在 sesno N 有子 [a,b,c]、在 N+1 删除 b 并新增 d，**When** 按 sesno=N 查询 P 的 children，**Then** 返回 [a,b,c]（含 b 当时的名称/类型）；按 sesno=N+1 查询返回 [a,c,d]
2. **Given** 子节点在两个版本间被改名，**When** 按旧版本查询，**Then** 返回旧名称（而非当前名称）
3. **Given** 同一接口不传 sesno，**When** 查询，**Then** 走现有 TreeIndex 路径，行为与现状完全一致
4. **Given** children 返回的子节点，**Then** 顺序与该版本 PDMS 中的同胞顺序一致（边 id 序号保序）

---

### User Story 2 - 增量落库同步维护版本化 pe_owner (Priority: P1)

运维照常执行增量同步（`incremental-sesno` / watch-incremental），层级关系变更（新增、删除、移动、重排）随本批 PE/ATT 变更进入**同一个版本提交批次**，共享同一锚点时刻；此后按任意锚点查询 pe_owner 均可信。

**Why this priority**: 写路径不闭环，读路径的历史就是错的。这是数据正确性的地基，必须与 US1 同期交付。

**Independent Test**: 对 fixture 跑一次含层级变更的增量，验证：pe_owner 当前态与 pe.children 一致；按增量前锚点 VERSION 查询返回旧层级；版本提交 fingerprint/counts 覆盖了边变更。

**Acceptance Scenarios**:

1. **Given** 一次增量中元素 X 的子列表变化（op 流含 X 的 Modified 记录及其 children 全量列表），**When** 落库，**Then** X 名下的 pe_owner 边区间被重写为新列表，且与 pe/ATT 写入同批提交、共享同一锚点
2. **Given** 一次增量中元素 Y 被删除，**When** 落库，**Then** Y 的 membership 边（Y→owner）与 Y 名下的子边全部删除；按删除前锚点查询 Y 仍出现在其父的 children 中
3. **Given** 增量落库中途失败，**When** 观察锚点表，**Then** 不产生本批锚点（现有 022 语义），pe_owner 半成品状态不会被当成一致版本暴露
4. **Given** 全量重灌到同一 versioned 库，**When** 重复执行，**Then** pe_owner 写入幂等（不因 id 冲突失败、不产生重复边）

---

### User Story 3 - 祖先链与子树按版本查询 (Priority: P2)

审查人员定位到某个历史版本中的节点后，需要查看它当时的祖先链（用于面包屑/定位）与子树 refno 集合（用于按范围继续分析）。

**Why this priority**: 补齐树浏览的配套查询，但单独的 children 已构成可用 MVP。

**Independent Test**: 对移动过父级的元素分别按移动前后两个锚点查祖先链，返回各自时刻的链路。

**Acceptance Scenarios**:

1. **Given** 元素 E 在 sesno N 属于 ZONE-A、在 N+1 被移到 ZONE-B，**When** 按 sesno=N 查 ancestors，**Then** 链路含 ZONE-A；按 N+1 查含 ZONE-B
2. **Given** 按版本查询子树 refnos，**When** 子树规模超过 limit/max_depth 上限，**Then** 与现有接口相同的截断语义（truncated 标记）

---

### User Story 4 - 存量站点接入与历史兜底 (Priority: P2)

管理员在已运行的 versioned 站点上启用本能力：先一次性重建当前态 pe_owner（从 pe.children），此后增量接管；对功能上线前的历史锚点，系统自动回退到 pe.children 版本查询并在响应中注明数据源。

**Why this priority**: 没有迁移与兜底路径，能力只对新建站点可用。

**Independent Test**: 对一个已有若干锚点的 versioned 站点执行重建，重建后当前态 pe_owner 与 pe.children 一致；按上线前锚点查询 children 仍返回正确层级（source 标注 fallback）。

**Acceptance Scenarios**:

1. **Given** 存量 versioned 站点（pe_owner 仅全量时写过、已被增量甩开），**When** 执行一次性当前态重建，**Then** pe_owner 与 pe.children 抽样一致，此后增量正常维护
2. **Given** 请求的 sesno 早于本功能上线（该区间 pe_owner 历史不可信），**When** 查询 children，**Then** 自动使用 pe.children VERSION 查询返回正确结果，响应 `version.source` 标注回退
3. **Given** 非 versioned 站点（无锚点表），**When** 树接口带 sesno 请求，**Then** 返回明确的"无锚点"错误，不静默退化为当前态

---

### Edge Cases

- **锚点缺失/回退命中**：请求的 sesno 无精确锚点时按"最近不大于"回退，响应必须携带 `resolved_sesno` 与 `exact` 标记；完全无锚点返回 AnchorMissing（404），与 `/api/model-history/*` 语义对齐
- **retention 窗外**：锚点时刻低于 GC 水位线时返回明确"历史已过期"（410），不 panic、不返回空集冒充
- **多 dbnum 世界树**：版本粒度是 per-dbnum 的 sesno，不存在全局版本号；一个"树版本"= `{dbnum → sesno}` 组合。第一期按单 dbnum 子树浏览为主，WORL 层跨 dbnum 的组合选择由前端按 dbnum 分别传参
- **大 owner 写放大**：增量重写子列表时按 owner 整区间重写（如大 ZONE 数千子边）；增量批次通常很小，可接受；后续可 diff 化优化
- **删除子树的深层展开**：按旧版本展开已删除子树的深层节点时，所有节点数据都来自 VERSION 查询，不得混入当前态记录
- **children_count 成本**：版本模式下逐子节点统计孙辈数量代价高；允许返回 null 或按需延迟统计，不得为凑数拖垮响应时间
- **同批 add→delete**：一个增量区间内先增后删的元素，按 sesno 顺序重放（现有机制），最终边状态与源一致
- **版本模式不支持的接口**：`site-nodes`（scene_node 非版本化）、`visible-insts` 及几何/instances 数据（latest-only）在带 sesno 请求时必须显式报 unsupported，不得静默返回当前态数据

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 增量落库 MUST 在同一版本提交批次内维护 pe_owner：对 op 流中带 children 的 Add/Modified 元素重写其名下边区间（先删后建、保序），对 Deleted 元素删除其 membership 边与名下子边；相关 SQL MUST 参与 commit fingerprint 计算与 counts 校验（`VersionCommitCounts` 扩展边计数）
- **FR-002**: 全量解析 MUST 继续写入 pe_owner，且对同一 versioned 库的重灌/重解析 MUST 幂等（不因已存在 id 失败、不产生重复或残留边）；边写入 MUST 在该批 `source='full'` 锚点固化之前完成
- **FR-003**: 模型树接口（world-root / node / children / ancestors / subtree-refnos）MUST 接受可选 `sesno` 参数；带 sesno 时经锚点解析换算时间戳后**实时** VERSION 查询，不生成、不依赖任何物化的版本树产物
- **FR-004**: 版本模式的 children MUST 保持该版本时刻的同胞顺序（以 pe_owner 边 id 序号为准；回退数据源时以 pe.children 数组顺序为准）
- **FR-005**: ~~不传 sesno 时上述接口 MUST 保持现状行为与性能（TreeIndex 路径零改动零回归）~~ **（2026-07-21 修订）** 不传 sesno 时 MUST 走 pe_owner latest 路径（与带 sesno 同源图语义；无 `.tree` / TreeIndex 依赖）
- **FR-006**: 版本入参解析 MUST 复用 specs/022 锚点体系：只接受可解析到锚点的 sesno；回退命中时响应携带 `requested_sesno / resolved_sesno / exact`；锚点缺失返回 AnchorMissing、retention 窗外返回 Expired，错误语义与 `/api/model-history/*` 一致
- **FR-007**: 节点名称/类型/owner 等展示属性在版本模式下 MUST 来自对应版本的 PE 快照（VERSION 点查或批量点查），不得混用当前态
- **FR-008**: 对 pe_owner 历史不可信的区间（本功能上线前的锚点、或站点尚未完成重建），children 查询 MUST 自动回退到 pe.children 的 VERSION 查询，并在响应 `version.source` 中标注实际数据源（`pe_owner` / `pe_children_fallback`）
- **FR-009**: 系统 MUST 提供存量 versioned 站点的一次性当前态 pe_owner 重建入口（从 pe.children 重建，CLI 或管理端任务），重建完成前版本查询走 FR-008 回退
- **FR-010**: 版本模式明确不覆盖的接口（site-nodes、visible-insts 及其他几何/实例数据）在带 sesno 请求时 MUST 返回明确的 unsupported 错误
- **FR-011**: 实现前 MUST 先以 fixture smoke 验证 fork 对 VERSION+图遍历 / VERSION+id 区间扫 / INSERT RELATION 冲突行为三项能力，并按结果选定 children 查询写法（边查询 vs pe.children 点查保底）；验证与验收全程走 CLI `--json` + HTTP smoke（仓库规则：不使用 cargo test）

### Key Entities

- **pe_owner（既有关系表，本 feature 的核心数据源）**: `id = pe_owner:[<owner pe 键>, <子序号>]`，`in` = 子 pe、`out` = 父 pe；同胞顺序由 id 第二段承载；versioned 实例下写入自动带版本，历史由存储引擎按 `VERSION $t` 回答
- **sesno_version_anchor（specs/022 既有）**: 业务版本（dbnum, sesno）→ 存储时间戳的唯一桥梁；本 feature 只读复用，不改其写入语义
- **pe.children（既有字段）**: 父节点上的有序子列表冗余，全量与增量均维护；作为版本查询的保底数据源与一致性校验基准

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 在含层级变更（增/删/移/改名）的 fixture 上，对改前/改后两个锚点的 children 与 ancestors 查询结果与各自时刻的真实层级 100% 一致（含顺序与节点名称）
- **SC-002**: 版本模式 children 查询（单父节点、子节点 ≤ 500）P95 延迟 ≤ 1 秒；ancestors（深度 ≤ 20）P95 ≤ 1 秒
- **SC-003**: 不传 sesno 的现有树接口在功能合入前后行为与延迟无可观测回归（对照 smoke 全通过）
- **SC-004**: 任意一次增量完成后，抽样 owner 的 pe_owner 当前态与 pe.children 一致率 100%；版本提交摘要含边变更计数
- **SC-005**: 存量站点重建完成后，上线前历史锚点的 children 查询经回退路径返回正确结果且 `version.source` 标注正确；非 versioned 站点带 sesno 请求 100% 返回 AnchorMissing 而非错误数据

## Assumptions

- 站点已按 specs/022 运行在实例级 `versioned=true` 存储上；非 versioned 站点不提供版本树能力（仅现状行为）
- PDMS 会话语义保证：元素子列表变化时，owner 元素必然以 Modified（携带 children 全量列表）出现在同一增量 op 流中，因此增量只需重写受影响 owner 的边区间，无需全库比对
- 版本粒度为 per-dbnum 的 sesno；跨 dbnum 的"全局版本"不在本 feature 范围
- 几何/实例数据（mesh、instances json/parquet、visible-insts、scene_node）保持 latest-only，"按版本显示三维模型"是独立的后续 feature
- fork surreal（D:\work\plant-code\surrealdb, dev-3.1）的 VERSION 点查能力已被 022 验证；图遍历/区间扫支持度按 FR-011 先行验证，不支持时以 pe.children 点查保底，不阻塞交付
- retention 配置沿用站点现有值；窗外历史的兜底仍是 PDMS 源 db 重扫（022 既定策略）
