# Task Plan: MBD 部署前选择与依赖补齐

## Goal

为 Admin 站点部署增加 MBD 选择能力：用户先从候选 MBD 下拉框选择部署范围，系统检查该 MBD 从 SYST 推导出的所有依赖 DB file 是否可定位；缺失时引导补充项目文件，全部可获取后才允许部署。

## Current Phase

Phase 1

## Phases

### Phase 1: Requirements & Discovery

- [x] 梳理用户需求：MBD 候选下拉、选择后根据 SYST 依赖 DB file path 做完整性检查、缺失时提供添加项目文件并刷新、全部可获取后部署。
- [x] 检查当前站点部署模型、parse preview、工程扫描、db_index 和 Admin UI 现状。
- [x] 记录关键发现到 `findings.md`。
- **Status:** complete

### Phase 2: Backend Contract Design

- [ ] 设计 sidecar MBD 候选接口：输入工程组成，输出 MBD 列表、模块、dbnums、DB 文件定位状态。
- [ ] 设计依赖完整性检查结构：`required_db_files`、`missing_db_files`、`ambiguous_db_files`、`ready_to_deploy`。
- [ ] 明确 MBD 选择和现有 `manual_db_nums/manual_db_files/auto_parse_related_dbnums/cata_partial_parse` 的合并规则。
- [ ] 明确缺失必需 DB 文件时 create/update/deploy 的后端阻断规则。
- **Status:** in_progress

### Phase 3: Persistent Site Model & Config

- [ ] 在 `ManagedProjectSite`、create/update/preview 请求和 TS 类型中加入 `mdb_name`、`mdb_module`，必要时加入 `mdb_db_nums` 快照。
- [ ] 将 `mdb_name/mdb_module` 纳入 `parse_plan_inputs_hash()`，防止 preview/db_index 复用旧范围。
- [ ] 修改 `build_site_config()`，优先把用户选择的 MBD 写入 `DbOption.toml` 的 `mdb_name`。
- [ ] 增加数据库 schema/migration 或 JSON 持久化兼容逻辑，确保旧站点缺省仍可工作。
- **Status:** pending

### Phase 4: Sidecar Implementation

- [ ] 在 `parse_sidecar.rs` 新增 MBD 候选/依赖检查端点，保持 web_server 不直接读取 E3D DB 文件。
- [ ] 复用或封装已有 MDB/MBD 查询语义：`src/mdb.rs`、`src/api/project_mdb.rs`、`query_db_nums_of_mdb()`、`query_db_quick_info()`。
- [ ] 将 MBD 中的 dbnums 映射为当前 `projects[]` 中可定位的文件；区分 `available/missing/ambiguous`。
- [ ] 给预览接口返回 MBD 依赖检查结果，并与现有 `entries/warnings` 保持兼容。
- **Status:** pending

### Phase 5: Admin UI Design & Implementation

- [ ] 在 `SiteDrawer.vue` 的“工程组成”区域增加“刷新 MBD 候选”动作。
- [ ] 新增“MBD 选择”区块：下拉框展示 `/MBD · DESI N · CATA N · 缺失 N`，选择后显示摘要卡片。
- [ ] 新增“依赖检查”区块：绿色可部署、黄色可选缺失、红色必需缺失阻断。
- [ ] 缺失项提供“添加项目目录/手动指定文件/刷新检查”操作，并复用现有 `projects[]` 与 `manual_db_files` 能力。
- [ ] 当 `missing_db_files.length > 0` 时禁用提交或自动部署按钮，文案为“依赖未补齐，暂不能部署”。
- **Status:** pending

### Phase 6: Verification

- [ ] 对 `aps250160_0001` 真实站点执行 MBD 候选发现，确认目标 MBD 与 DB 文件覆盖。
- [ ] 验证缺失 DB 文件时 preview 返回阻断状态，UI 禁用部署。
- [ ] 补充项目目录或文件后刷新，确认依赖从 missing 变为 available。
- [ ] 验证选择 MBD 后 parse plan 只解析 MBD 覆盖范围与精确依赖闭包。
- [ ] 验证 `2013286704_476` BRAN 所在部署仍能生成/显示模型。
- **Status:** pending

### Phase 7: Documentation & Handoff

- [ ] 更新 `specs` 或 `docs/plans` 中的设计说明，记录 MBD 选择和依赖闸门契约。
- [ ] 更新 `progress.md` 的验证命令、响应样例和剩余风险。
- [ ] 输出最终中文结论：实现范围、验证证据、已知限制和下一步。
- **Status:** pending

## Key Questions

1. MBD 候选应完全从 SYST/PROJECT_MDB 推导，还是允许用户手动输入一个未发现的 MBD 名称？当前建议：首版只允许选择已发现候选，降低不可复现风险。
2. `manual_db_nums` 与 MBD 选择是否可同时使用？当前建议：首版二选一；额外库通过“补充项目文件”进入依赖检查，不直接混合两个范围事实源。
3. 缺失的 CATA/DICT/GLOB/GLB 是否都阻断部署？当前建议：MBD 明确列出的必需 DB 文件阻断；预览发现的可选依赖可警告但不自动忽略。
4. MBD 候选发现是否依赖已经完成 SYST 解析？当前建议：sidecar 直接基于工程 DB/SYST 文件做离线候选发现；如果需要已解析数据库，应在 UI 明确“需先解析系统库”。
5. `mdb_name` 写入 `DbOption.toml` 后是否会影响模型生成全库模式？需要在 Phase 3/6 通过真实 `DbOption-parse.toml` 与 `DbOption-generate.toml` 验证。

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| MBD 功能做成部署前闸门，而不是普通文本输入框 | 用户目标是部署范围和依赖完整性，不只是配置一个名称。 |
| MBD 候选/依赖解析放在 aios-database sidecar | 现有架构已要求 web_server 不直接扫描 E3D DB 文件，工程扫描和 db file resolve 都通过 sidecar。 |
| `mdb_name/mdb_module` 提升为站点配置字段 | 当前 `DbOption.toml` 的 `mdb_name` 来自模板默认值，无法表达用户在部署 UI 中的选择。 |
| 缺失必需 DB 文件阻断部署 | 否则部署会进入 parse/generate 后才暴露缺库，用户无法在配置阶段闭环补齐。 |
| UI 保持单 Drawer 分区，不强制复杂 wizard | 现有 `SiteDrawer.vue` 已有工程组成、解析预览和部署按钮，分区增强改动更小。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| 根目录已有旧 `task_plan.md` active plan | 1 | 创建独立 `.planning/2026-06-12-mbd-deploy-preflight/`，避免覆盖旧 sidecar 计划。 |

## Notes

- 关键后端文件：
  - `src/parse_sidecar.rs`
  - `src/web_server/parse_sidecar_client.rs`
  - `src/web_server/admin_handlers.rs`
  - `src/web_server/managed_project_sites.rs`
  - `src/web_server/models.rs`
  - `src/mdb.rs`
  - `src/api/project_mdb.rs`
- 关键前端文件：
  - `ui/admin/src/components/sites/SiteDrawer.vue`
  - `ui/admin/src/types/site.ts`
  - `ui/admin/src/api/sites.ts`
- 当前推荐首个实现目标：先做只读候选发现 + 依赖检查预览，再接入持久化和部署阻断。
