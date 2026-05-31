# 多工程合并站点（Multi-Project Site）实施计划

> 日期：2026-05-31
> 分支：`feat/multi-project-site`
> 项目路径：`D:/work/plant-code/plant-model-gen`
> 关联：路线图 `docs/plans/2026-05-09-next-iteration-roadmap.md` 的 **S6 / Sprint F2「多项目支持」**
> 上游能力基线：`docs/plans/2026-04-26-site-admin-next-steps.md`（Sprint C/D 已落地）

---

## 0. 背景与目标

Admin 站点（`ManagedProjectSite`）原本是 **单工程站点**：一个 site 绑定一个 `project_path`。
本计划把它升级为 **多工程合并站点**：一个 site 可包含多个工程条目（`SiteProject`），
其中恰好一个为 primary，并区分「设计工程（Design）/ 元件库工程（Library）」角色，
解析与生成按「跨工程根聚合 + 单趟合并」执行。

**最终目标（端到端可用）**：管理员能在 `/admin` 前端创建一个站点、扫描并添加多个工程、设置 primary/角色/排序，
后端完成白名单校验 → dbnum 冲突预检 → 派生配置 → 跨根文件聚合 → 解析生成，全链路打通。

### 非目标
- 不改异地协同 / 远程部署语义（那是另一条线）。
- 不在本计划内做 Sprint E 的审计日志 / RBAC（见「风险」第 4 条，单独排期）。
- 不重写 `managed_project_sites.rs` 既有单工程链路，只做兼容扩展。

---

## 1. 数据模型（Phase 1 · 已完成）

`src/web_server/models.rs`：

- `enum ProjectRole { Design, Library }`，`Default = Design`。
- `struct SiteProject { path, name, role, is_primary, sort_order }`，多工程最小单元。
- `ManagedProjectSite` 新增 `site_name` + `projects: Vec<SiteProject>`（事实源，持久化为 `projects_json` 列）。
- `CreateManagedSiteRequest` / `UpdateManagedSiteRequest` / `PreviewManagedSiteParsePlanRequest` 均补 `site_name` + `projects`。

持久化与兼容：`projects_json TEXT NOT NULL DEFAULT '[]'` + `ensure_column_exists(conn, "projects_json")`，
旧单工程站点升级后 `projects` 为空，读路径回落 `project_path`。

---

## 2. 后端核心（Phase 2 · 已完成，编译通过）

| 任务 | 内容 | 状态 | 代码位置 |
|---|---|---|---|
| T2.1 | 白名单校验 `validate_and_canonicalize_projects`：逐条 canonical + 断言（≥1 工程 / ≥1 Design / 恰 1 primary / 名唯一） | ✅ | `managed_project_sites.rs:932`，create/update 调用 `:3020 / :3311` |
| T2.2 | dbnum 冲突预检 `precheck_dbnum_conflicts`：读文件头建 `dbnum→工程`，冲突 `bail`；create/update/preview 都调 | ✅ | `:1033` |
| T2.3 | `site_id` 由 `site_name` 派生 | ✅ | 持久化字段含 `site_name`（insert `:2268`） |
| T2.4 | 派生（included_projects / 站点级配置入口） | ✅（已报） | 解析路径取 `resolve_included_db_files` `:1688` |
| T2.5 | 配置生成（多工程 DbOption / 运行目录） | ✅（已报） | — |
| T2.6 | 跨根文件聚合 `resolve_included_db_files` | ✅ | `:1136`，调用点 `:1688 / :3280 / :4580` |
| T2.7 | 解析走「档 A 单趟合并」（满足验收） | ✅（已报） | — |
| T2.8 | 删旧字段（纯清理） | ⏳ 待办 | **必须排在 Phase 4 之后**，详见风险第 2 条 |

> 验证方式：本计划按仓库约定不写测试，使用 `cargo check` + HTTP + CLI。
> 结构级验证已在 2026-05-31 完成（上列函数与迁移均存在）；编译固化见「验收」。

---

## 3. Phase 3 · 工程扫描 API（✅ 已实现，2026-05-31，编译通过）

**目标**：前端给一个根路径，后端自动发现候选工程并预标冲突，减少手工录入。

- 端点：`GET /api/admin/projects/scan?root=<path>`（经 `admin_auth_middleware`）。
- 实现：
  1. 路径走现有白名单 + `canonicalize`（`canonical_project_path`）。
  2. 候选 = root 直接子目录中递归含 db 文件者；无子目录命中则 root 自身作单候选。
  3. 读 db 文件头（`parse_file_basic_info`）收 `(dbnum, db_type)`，推断 `role`：含 DESI=design / 仅 CATA=library / 其余回退 design。
  4. 跨候选 dbnum 冲突**只标注不 bail**（`ScanProjectsResult.conflicts`），保存时由 `precheck_dbnum_conflicts` 兜底。
  5. 稳定排序后首个 design 候选标 `is_primary`，给出 `sort_order`。
- 代码位置：
  - `models.rs`：`ScannedProject` / `ScannedDbnumConflict` / `ScanProjectsResult`。
  - `managed_project_sites.rs`：`collect_project_db_entries` / `infer_scanned_role` / `scan_projects_under_root`。
  - `admin_handlers.rs`：`ProjectScanQuery` + `scan_projects` + 路由 `/api/admin/projects/scan`。
- 返回示例字段：`{ root, projects:[{path,name,role,is_primary,sort_order,dbnums,db_types}], conflicts:[{dbnum,projects}], has_conflict }`。
- 待真实验收：`curl /api/admin/projects/scan?root=<白名单内根>`（需起 web_server + admin 鉴权）。

---

## 4. Phase 4 · 前端工程组成（✅ 已实现，2026-05-31，构建通过）

**目标**：让多工程模型可被用户录入与编辑，达成端到端可用。

- `types/site.ts`：新增 `ProjectRole` / `SiteProject` / `ScannedProject` / `ScannedDbnumConflict` / `ScanProjectsResult`，并给 `ManagedProjectSite` 及 create/update/preview 请求补 `site_name` + `projects`。
- `api/sites.ts`：新增 `scanProjects(root)` → `GET /api/admin/projects/scan`。
- `SiteDrawer.vue`：新增「工程组成（多工程，可选）」区：
  - 站点名称 `site_name`。
  - 扫描根目录 + 「扫描 / 手动添加」：调 `scanProjects` 自动导入候选（按 path 去重），回显 dbnum 冲突。
  - 工程行：name / path / role(design|library) / 主工程单选 / 删除。
  - 即时校验 `multiProjectError`（路径非空 / ≥1 design / 恰 1 primary / 名唯一），未通过禁用保存；后端 `validate_and_canonicalize_projects` 兜底。
  - 提交时注入 `site_name` + `projects`（projects 为空则退回单工程语义）。
- `SiteConfigSections.vue`：详情页新增只读「工程组成（多工程）」卡片（仅多工程站点显示，含 站点名/各工程名·角色·主工程·路径），单工程站点不受影响。
- 构建：`cd ui/admin && npm run build`（`vue-tsc -b && vite build`）EXIT=0，产物写入 `src/web_server/static/admin`。
- 待真实验收：`/admin/#/sites` 新建抽屉手动走一遍扫描 + 保存。

---

## 5. 前置收口（进 Phase 3 之前，约 0.5 天）

1. **固化编译**：`cargo check --bin web_server --features web_server`（环境需 `D:\Rust\.cargo\bin` + NASM PATH）。
2. **提交当前改动**：`feat/multi-project-site` 上 `models.rs`(+50) / `managed_project_sites.rs`(+388) 分批提交（**需用户显式确认后提交**）。
3. **本计划文档入库**：即本文件，作为多工程线的唯一治理文档。

---

## 6. 风险

| # | 风险 | 缓解 |
|---|---|---|
| 1 | 大块 WIP（~438 行）悬空 | 固化 cargo check 后尽快分批提交，避免冲突/丢失 |
| 2 | **T2.8 删旧字段顺序错误** | 删 `project_path/project_name` 前，必须保证 Phase 4 前端已切到 `projects` 且读路径双读兼容；T2.8 排在 Phase 4 之后 |
| 3 | 旧站点迁移 | `projects` 为空时回落 `project_path`；必要时提供一次性回填脚本 |
| 4 | Sprint E（审计 G18 / RBAC G19）仍缺位，而多工程扩大了配置可变面与扫描入口 | 配置面扩张的同时，把审计/权限债排进 Sprint F 并行推进 |
| 5 | 概念去重债（G4：三套 site 同住一个 sqlite） | 多工程叠加远程部署会加重；保持 `projects_json` 仅在 Admin 站点语义内扩展 |

---

## 7. 验收 / 完成定义

- [x] `cargo check --bin web_server --features web_server` EXIT=0（固化「编译通过」，2026-05-31，33.9s，0 error）
- [x] Phase 3：`/api/admin/projects/scan` 已实现并编译通过（真实 HTTP 验收待起服务）
- [x] Phase 4：`/admin/#/sites` 新建抽屉可录入多工程、设 primary、切角色、接扫描（`npm run build` 通过；真实点击验收待起服务）
- [ ] 端到端：创建多工程站点 → 解析 → 生成成功，旧单工程站点行为不回退
- [ ] T2.8 仅在 Phase 4 前端切换完成后执行

---

## 8. 推荐执行顺序

```
1. 前置收口：cargo check 固化 → 提交 ~438 行（待确认）→ 本文档入库
2. Phase 3：工程扫描 API（复用 precheck_dbnum_conflicts）         ✅ 已实现
3. Phase 4：前端工程组成（复用 SiteDrawer / SiteConfigSections）   ✅ 已实现
4. 端到端真实验收（见 §9 Runbook，需运行环境）
5. T2.8：删旧字段清理（端到端验收通过后）
```

---

## 9. 端到端验收 Runbook（需运行环境）

> 现状探测（2026-05-31）：本机无运行中的 web_server、无 admin token、toml 未配
> `admin_allowed_project_roots`。以下步骤需在具备运行环境 + 真实 AVEVA 工程数据时执行。

### 9.0 前置
1. 配置白名单（否则 `canonical_project_path` 会拒绝）：在所用 `DbOption-*.toml` 加
   `admin_allowed_project_roots = ["D:/path/to/aveva-projects-root"]`
   （本地开发可临时 `AIOS_ALLOW_WEAK_DB_CREDS=1` / `AIOS_ADMIN_ALLOW_ANY_PROJECT_PATH=1`，生产勿用）。
2. 启动：`cargo run --bin web_server --features web_server -- --config <DbOption>`。
3. admin 登录拿 token（Bearer / cookie），后续请求带上。

### 9.1 Phase 3 扫描 API smoke
```bash
# git-bash / curl
curl -sS -H "Authorization: Bearer $ADMIN_TOKEN" \
  "http://127.0.0.1:3100/api/admin/projects/scan?root=D:/path/to/aveva-projects-root" | jq .
```
```powershell
# PowerShell
$h = @{ Authorization = "Bearer $env:ADMIN_TOKEN" }
Invoke-RestMethod -Headers $h -Uri "http://127.0.0.1:3100/api/admin/projects/scan?root=D:/path/to/aveva-projects-root" | ConvertTo-Json -Depth 6
```
**期望**：返回 `data.projects[]`（含 `path/name/role/is_primary/sort_order/dbnums/db_types`）；
含 DESI 的工程 `role=design`、仅 CATA 的 `role=library`；首个 design 标 `is_primary`；
有重复 dbnum 时 `conflicts[]` 非空且 `has_conflict=true`，但 HTTP 仍 200（只标注不报错）。
未在白名单内的 root → 明确报错（非 500 崩溃）。

### 9.2 Phase 4 前端手工验收（`/admin/#/sites`）
1. 新建站点 → 「工程组成」填扫描根目录 → 点「扫描」→ 候选自动入列、角色/主工程已预填。
2. 调整 role / 主工程单选 / 删除某行 → 「保存」成功（projects 非法时保存按钮禁用）。
3. 重开该站点编辑 → 工程列表回显（`projects` 持久化生效）。

### 9.3 旧站兼容核对（关键）
- 一个 `projects` 为空的旧单工程站点：解析/启动行为与改造前一致（读路径回落 `project_path`）。
- 升级后 `projects_json` 默认 `'[]'`，不阻断旧站点。

### 9.4 Pass 标准
- [ ] 扫描返回候选 + 角色推断正确 + 冲突标注正确
- [ ] 抽屉可录入/编辑/回显多工程并保存成功
- [ ] 多工程站点解析 → 生成成功
- [ ] 旧单工程站点行为不回退
