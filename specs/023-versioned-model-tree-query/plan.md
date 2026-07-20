# Implementation Plan: 按版本实时查询模型树（versioned pe_owner）

**Branch**: `023-versioned-model-tree-query` | **Date**: 2026-07-19 | **Spec**: `specs/023-versioned-model-tree-query/spec.md`

**Input**: Feature specification from `/specs/023-versioned-model-tree-query/spec.md`

## Summary

在 specs/022 的 versioned 存储与 sesno 锚点体系之上，把模型树做成**可按版本实时查询**：层级用 `pe_owner` 边 + `VERSION $t` 现场查出（不物化任何版本树产物），节点属性用 PE 的 VERSION 点查。核心工程是把增量落库补齐为"层级变更与 PE/ATT 同批版本提交"（现状增量完全不维护 pe_owner），读侧给 `/api/e3d/*` 树接口加可选 `sesno` 参数。能力验证已完成（见 research.md）：图遍历 + VERSION 语义正确（选用）；**id 区间扫 + VERSION 语义错误（禁用）**；`pe.children` 点查为保底。

## Technical Context

**Language/Version**: Rust（workspace 现行 toolchain）

**Primary Dependencies**:
- fork `surrealdb`（dev-3.1，实测 3.2.0-nightly）——VERSION + 图遍历（research C1 已验证）
- `aios_core`（rs-core dev-3.1）——`resolve_anchor` / `fn::sesno_version` / `project_primary_db()`（022 交付，本 feature 只读复用）
- plant-model-gen 本仓——增量边维护、树 API 版本分支、重建 CLI

**Storage**: SurrealDB over RocksDB `versioned=true`（022 既有）；`pe_owner` 关系表复用既有 schema（数组 id `[owner, 序号]`），不新增表；新增一条元记录标记"pe_owner 自哪个 sesno 起可信"（见 data-model.md）

**Testing**: 仓库规则——不写 cargo test；能力 smoke（已交付 `scripts/smoke/pe_owner_version_capability_smoke.ps1`）+ 端到端 HTTP smoke（M4）+ CLI `--json`

**Target Platform**: Windows 开发机 + Linux 部署（versioned 站点）

**Project Type**: 常驻服务（web_server 树 API）+ CLI（增量/重建）

**Performance Goals**: 版本 children（单父 ≤500 子）P95 ≤ 1s；ancestors（深度 ≤20）P95 ≤ 1s；不传 sesno 路径零回归（SC-002/SC-003）

**Constraints**:
- 版本入参只接受可解析锚点的 per-dbnum sesno；错误语义对齐 `/api/model-history/*`（404 AnchorMissing / 410 Expired）
- **禁止 `pe_owner:[..]..` id 区间扫 + VERSION**（research C3：语法接受但返回当前态，静默错误数据）
- 边写入幂等策略：先删后插（`INSERT RELATION` 撞 id 且值不同直接报错；`INSERT IGNORE RELATION` 语法不存在——research C6/C7）
- 增量边变更必须与 PE/ATT 同一 `mutation_sqls` 批次（进 fingerprint 与 counts），失败不落锚点（022 语义）
- PDMS 语义假设：子列表变化时 owner 必以 Modified（带 children 全量）出现在同一 op 流

**Scale/Scope**: 单站点 pe 千万级、单 owner 通常 ≤ 数百子（大 ZONE 数千）；增量批次小（日常几十~几千 op）

## Constitution Check

- 不使用 cargo test：验证全部走 smoke 脚本 + HTTP/POST + CLI `--json` ✅
- web_server 相关用运行中服务 + HTTP 验证 ✅
- 不写入任何密钥到仓库 ✅

## Project Structure

### Documentation (this feature)

```text
specs/023-versioned-model-tree-query/
├── spec.md              # 已完成
├── plan.md              # 本文件
├── research.md          # 已完成（FR-011 能力验证结论：C1 选用 / C3 禁用 / 先删后插）
├── data-model.md        # 实体与元记录定义
├── contracts/
│   └── tree-version-api.md   # /api/e3d/* 版本参数契约
├── quickstart.md        # 端到端验证指引
└── checklists/requirements.md  # 已完成（全过）
```

### Source Code (repository root)

```text
d:\work\plant-code\plant-model-gen\
├── src\data_interface\sesno_increment.rs   # M1：增量 op → pe_owner 边维护 SQL（同批提交）
├── src\versioned_db\version_commit.rs      # M1：VersionCommitCounts 增加 pe_owner_rows
├── src\versioned_db\pe.rs                  # M2：save_pe_relates 幂等（先删后插）
├── src\versioned_db\database.rs            # M2：确认边写入先于 full 锚点固化
├── src\web_api\e3d_tree_api.rs             # M3：?sesno= 版本分支（children/ancestors/subtree/node/world-root）
├── src\web_api\pe_att_history_api.rs       # M3：错误语义对齐参考（不改动）
├── src\version_management\cli.rs           # M5：model-version rebuild-pe-owner 子命令
├── scripts\smoke\
│   ├── pe_owner_version_capability_smoke.ps1   # 已交付（能力验证，可重跑）
│   └── tree_version_smoke.ps1                  # M4：端到端验收
└── db-data\
    ├── smoke_023_pe_owner_version.surql        # 已交付
    └── smoke_023_result.json                   # 能力验证原始结果
```

**Structure Decision**: 不新增模块文件；边维护逻辑内聚在 `sesno_increment.rs`（与 pe/att 写入同源同批），树查询版本分支内聚在 `e3d_tree_api.rs`（handler 内按 `sesno.is_some()` 分流），与现有分层一致。锚点/版本换算继续由 rs-core 提供，本仓不重复实现。

## Milestones

### M1 — 增量维护版本化 pe_owner（P1，US2 / FR-001）

1. `persist_pdms_increment_grouped` 的 op 循环中，按 sesno 顺序追加边维护 SQL 到 `mutation_sqls`：
   - Add/Modified（`current_ele_for_persist` 命中，带 `ele.children`）：
     `DELETE pe:<x><-pe_owner;` → `INSERT RELATION INTO pe_owner [ {id:[<x>,0],in:<c0>,out:<x>}, ... ];`（children 为空则只删）
   - Deleted：`DELETE pe:<x>->pe_owner; DELETE pe:<x><-pe_owner;`（membership 边 + 名下子边）
2. `VersionCommitCounts` 增加 `pe_owner_rows`（serde default，兼容旧记录）；stats 计数
3. fingerprint 无需专门处理（`compute_commit_fingerprint` 输入即 mutation_sqls 文本，自动覆盖）
4. 首次成功维护后写入 `pe_owner_version_meta`（见 data-model.md：`maintained_since_sesno`，create-once）
5. 验证：fixture 跑一次含层级变更的增量 → 当前态 `pe:<x><-pe_owner.in` 与 `pe:<x>.children` 一致；按增量前锚点 VERSION 查询返回旧层级；commit 摘要含边计数

### M2 — 全量写入幂等与时序（P1，US2 / FR-002）

1. `save_pe_relates` 生成的批量插入改为"先删后插"：每个 owner 段前置 `DELETE <owner键><-pe_owner;`（同一 owner 的边在同一批内成段发送）
2. 确认（并以注释固化）sender 流水线中 PERelateJson 全部落库先于 `commit_version(source='full')`；不满足则把边写入挪到 commit 前显式 flush
3. 全量完成后 UPSERT `pe_owner_version_meta.maintained_since_sesno = <本次 full sesno>`（full 重灌语义上重置可信起点）
4. 验证：同库重复 parse 同一 dbnum 不报 id 冲突、无重复/残留边

### M3 — 树接口版本分支（P1，US1/US3 / FR-003~FR-008、FR-010）

1. `e3d_tree_api.rs`：`children / node / ancestors / subtree-refnos / world-root` 增加可选 `?sesno=`；不传走现有 TreeIndex 路径（零改动）
2. 版本分支入口：`TreeIndexManager::resolve_dbnum_for_refno` 取 dbnum → rs-core `resolve_anchor(dbnum, sesno)` → 无锚点 404 AnchorMissing；`LET $t = fn::sesno_version(...)`；GC 越界翻译 410 Expired（对齐 `pe_att_history_api.rs`）
3. 数据源选择：`requested_sesno >= pe_owner_version_meta.maintained_since_sesno` → 主路径 `SELECT VALUE in FROM pe:<owner><-pe_owner VERSION $t`（显式 `ORDER BY record::id(id)[1]` 兜底顺序）；否则回退 `SELECT VALUE children FROM pe:<owner> VERSION $t`；响应 `version.source` 标注 `pe_owner` / `pe_children_fallback`
4. 节点属性：页内批量 `SELECT name, noun, owner FROM [pe:a, pe:b, ...] VERSION $t`；children_count 版本模式返回 null（边界已在 spec 声明）
5. ancestors：`SELECT VALUE owner FROM pe:<x> VERSION $t` 逐级点查（深度上限 20）
6. subtree-refnos：children BFS 逐层，沿用现有 max_depth/limit/truncated 语义
7. `site-nodes` / `visible-insts` 带 sesno → 明确 unsupported 错误（FR-010）
8. 响应统一附 `version: {requested_sesno, resolved_sesno, exact, source}`
9. 验证：对 fixture 手工 HTTP 核对（正式验收在 M4）

### M4 — 端到端验收 smoke（SC-001~SC-005）

1. `scripts/smoke/tree_version_smoke.ps1`：在 versioned fixture 上构造两个锚点（增/删/移/改名各至少一例）→ 分别调 `children?sesno=` / `ancestors?sesno=` 全量比对；验 404（无锚点 sesno / 非 versioned 站点）、410（窗外，如可构造）、不传 sesno 与现状一致、fallback source 标注
2. 性能抽样：单 owner ≥200 子的 children 版本查询计时，核对 SC-002
3. 结果回填 spec Success Criteria 勾选

### M5 — 存量站点重建 CLI（P2，US4 / FR-009）

1. `model-version rebuild-pe-owner --dbnum <D> [--json]`：从当前 `pe.children` 全量重建该 dbnum 的 pe_owner（分批：每 owner 先删后插），完成后 UPSERT `pe_owner_version_meta`
2. quickstart.md 记录：存量站点接入顺序（升级二进制 → 跑重建 → 增量接管 → 老锚点自动 fallback）
3. 验证：重建后抽样 owner 边与 children 字段一致率 100%

## 风险与对策

| 风险 | 对策 |
|------|------|
| id 区间扫 + VERSION 语义错误（已实测） | 明令禁用（本 plan Constraints + 代码注释）；只用图遍历/点查 |
| 大 owner 增量重写写放大（数千子边先删后插） | 增量批次小、可接受；计数进 commit 摘要可观测；必要时后续 diff 化 |
| 图遍历 + VERSION 在真实规模下性能未知 | M4 用真实规模 fixture 抽样核对 SC-002；不达标则版本模式 children 切 `pe.children` 点查为主路径（读侧开关，写侧不变） |
| 功能上线前历史锚点 pe_owner 不可信 | `pe_owner_version_meta.maintained_since_sesno` 分界 + 自动 fallback（FR-008），source 显式标注 |
| 增量 op 流缺 owner Modified（假设破裂） | M4 smoke 用 pe.children 与 pe_owner 双源交叉校验；不一致计入告警而非静默 |
| fingerprint 因新增 SQL 与旧 pending 不匹配 | 仅影响新提交；`--recover-pending` 重放要求同版本二进制（quickstart 注明升级前先清 pending） |

## 遗留验证项（进 tasks.md）

- 同值重插（in/out 完全一致）的 INSERT RELATION 行为确认（先删后插策略下为确认性验证）
- 图遍历返回顺序恒等于边 id 序的确认（实现已显式 ORDER BY 兜底）
- 真实规模性能抽样（M4）
