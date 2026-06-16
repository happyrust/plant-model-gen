# Progress Log: MBD 部署前选择与依赖补齐

## Session: 2026-06-12

### Phase 1: Requirements & Discovery

- **Status:** complete
- **Started:** 2026-06-12 23:17 UTC+8
- Actions taken:
  - 使用 `planning-with-files` skill 组织本轮开发计划。
  - 读取项目根目录既有 `task_plan.md`、`progress.md`、`findings.md`，确认旧 active plan 属于 release 包 sidecar job 终态竞态修复。
  - 运行 `session-catchup.py`，无输出，视为无额外 unsynced context。
  - 检查当前 Admin 站点部署相关代码和 UI：
    - `SiteDrawer.vue` 的工程组成、parse preview、解析范围、提交条件。
    - `models.rs` 的 `ManagedProjectSite`、`PreviewManagedSiteParsePlanRequest`、`ManagedSiteParsePlan`。
    - `managed_project_sites.rs` 的 `build_site_config()`、`parse_plan_inputs_hash()`、`preview_request_from_site()`。
    - `parse_sidecar.rs` 的工程扫描、DB 文件解析和 preview 范围生成。
    - `mdb.rs`、`api/project_mdb.rs` 的 MDB/MBD 查询参考。
  - 创建独立 planning 目录：`.planning/2026-06-12-mbd-deploy-preflight/`。
  - 写入 `task_plan.md` 和 `findings.md`。
- Files created/modified:
  - `.planning/2026-06-12-mbd-deploy-preflight/task_plan.md`
  - `.planning/2026-06-12-mbd-deploy-preflight/findings.md`
  - `.planning/2026-06-12-mbd-deploy-preflight/progress.md`

### Phase 2: Backend Contract Design

- **Status:** in_progress
- Actions taken:
  - 初步提出 `POST /api/admin/projects/mdb-candidates` 接口形态。
  - 初步提出 preview response 增加 MBD 依赖检查结构。
  - 初步决策：MBD 选择和手动 DB Nums 首版二选一，降低范围合并歧义。
- Files created/modified:
  - `.planning/2026-06-12-mbd-deploy-preflight/task_plan.md`
  - `.planning/2026-06-12-mbd-deploy-preflight/findings.md`

## Test Results

| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| planning skill 读取 | `C:\Users\dpc\.agents\skills\planning-with-files\SKILL.md` | 确认流程和文件约定 | 已读取 | PASS |
| existing planning 恢复 | 根目录 `task_plan.md/findings.md/progress.md` | 确认是否已有 active plan | 已确认旧计划属于 sidecar job 修复 | PASS |
| session catchup | `python C:\Users\dpc\.agents\skills\planning-with-files\scripts\session-catchup.py D:\work\plant-code\plant-model-gen-cata-closure` | 输出未同步上下文或空 | 空输出，非阻塞 | PASS |
| 计划文件创建 | `.planning/2026-06-12-mbd-deploy-preflight/` | 三文件可用于后续恢复 | 已创建并写入 | PASS |

## Error Log

| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-12 23:17 UTC+8 | 根目录已有旧 `task_plan.md`，直接覆盖会丢失历史 active plan | 1 | 使用独立 `.planning/2026-06-12-mbd-deploy-preflight/` 存放本轮计划。 |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Phase 1 已完成，Phase 2 后端契约设计进行中。 |
| Where am I going? | 下一步细化 sidecar MBD 候选接口、依赖检查结构、站点模型字段和 UI 交互。 |
| What's the goal? | 让站点部署先选择 MBD，并在所有必需 DB file 可获取后才允许部署。 |
| What have I learned? | 当前站点模型没有持久化 MBD；`DbOption.toml` 的 `mdb_name` 仍来自模板；工程扫描和 DB 文件解析应继续放在 sidecar。 |
| What have I done? | 已完成代码级现状梳理，并创建独立 planning 文件记录计划、发现和进度。 |

---

*Update after completing each phase or encountering errors.*
