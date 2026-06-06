# 一键部署测试功能 开发计划（2026-05-31）

> 任务来源：MCP 通道 `best-mcp-22` 派发
> 一句话目标：提供一个**免鉴权、单次调用**的「一键部署测试」API —— 传入项目路径 + 目标 db 文件（路径 / 文件名 / dbnum），自动完成 **建站 → 解析（单库，可选含关联库）→ 生成该库全部模型 →（可选）启动站点**，并回传进度与结果，用于快速验证单个 db 的解析与建模。
> 关联代码：
> - `src/web_server/admin_handlers.rs`（admin 路由）
> - `src/web_server/managed_project_sites.rs`（建站 / 解析 / 生成 / 启动 pipeline）
> - `src/web_server/models.rs`（请求/站点模型）
> - `src/web_server/stream_generate.rs`（按需流式生成，进度回传参考）
> 验证样本：`D:\AVEVA\Projects\E3D2.1\AvevaPlantSample\aps000\aps250164_0001`（DESI，806912 字节，已存在）

---

## 0. 背景

当前「受管站点（Managed Site）」已具备完整的分步部署能力，但要把"单个 db 文件解析 + 生成全部模型"跑通，需要人工串多步、且要先知道 dbnum。本功能的定位是**快测**：一次调用、零鉴权、最少入参，把链路一把梭。

已确认事实（来自现状盘点）：

| 项 | 结论 |
|---|---|
| 现有端点 | `POST /api/admin/sites`（建站）、`/preview-parse-plan`、`/{id}/parse`、`/{id}/generate`、`/{id}/deploy`、`/{id}/start`、`GET /{id}/runtime` |
| 解析圈定方式 | 按 `manual_db_nums`(dbnum)；`scan_db_file_name` 读 db 文件头比对 dbnum，**不支持直接传文件路径** |
| 生成方式 | `run_generation_pipeline(parse_first)` → `spawn_generation_process` 拉起 `aios-database` CLI（generation 配置，`gen_model=true`） |
| deploy 管线 | `run_deploy_pipeline` = 解析 + 生成 +（启动），由站点 `gen_model/gen_mesh/gen_spatial_tree` 决定是否生成 |
| 系统库 | 解析单 DESI 会自动 bootstrap `SYST`（`REPARSE_REUSE_DB_TYPES`） |
| 关联库开关 | `auto_parse_related_dbnums` 布尔开关**已加（首版）**：开启时粗粒度纳入 `CATA/DICT`（`RELATED_DEPENDENCY_DB_TYPES`），已标 `TODO(后续完善)` 为按引用精确解析 |
| 鉴权 | 用户确认该 API 免鉴权（快测专用） |

---

## 1. 需求拆解与缺口（Gap）

| 编号 | 需求 | 现状 | 缺口 |
|---|---|---|---|
| G1 | 直接用**文件路径/文件名**指定要解析的库 | 只能传 dbnum | 需要 path/filename → dbnum 解析 |
| G2 | **一键**：单次调用完成 建站→解析→生成→(启动) | 需分步或先 create 再 deploy | 缺合并入口 |
| G3 | 免鉴权快测 | admin 路由（用户称免鉴权） | 需确认/落实该端点不挂鉴权中间件 |
| G4 | 进度 / 结果回传 | runtime 轮询存在；deploy 是后台任务 | 需要可观测的 summary / SSE |
| G5 | 关联库自动解析（精确版） | 首版粗粒度 CATA/DICT | 按 SPEC/CATA 引用算出真正关联 dbnum |
| G6 | 幂等 + 清理 | 重复部署同一文件行为未定义 | 稳定 site_id + 复用/重置策略 |
| G7 | 真实回路验证 | 无 | 起 web_server + curl 跑 aps250164_0001 |

---

## 2. 方案设计

### 2.1 新增端点

```
POST /api/admin/quick-deploy-test
```
> 注：最终用 `/api/admin/quick-deploy-test`（而非 `/api/admin/sites/quick-deploy-test`），挂在主路由（无 admin 鉴权中间件），且避开与 `/api/admin/sites/{id}` 参数路由的同层重叠。

**请求体（`QuickDeployTestRequest`）**

```jsonc
{
  "project_path": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample", // 必填：工程根目录
  "project_name": "AvevaPlantSample",   // 可选；缺省时 E3D 项目名取 project_path 目录名，站点显示名用默认递增名 quicktest-N（评审反馈）
  "project_code": 0,                      // 可选
  "db_file": "aps250164_0001",           // 三选一：文件名 / 绝对路径 / 相对 project_path 的路径
  "dbnum": null,                          // 三选一：直接给 dbnum（给了就跳过文件解析）
  "auto_parse_related_dbnums": false,     // 复用已加开关
  "gen_model": true,
  "gen_mesh": false,
  "gen_spatial_tree": false,
  "start_site": false,                    // true=部署后顺带起站点(web+viewer)，false=只产数据
  "web_port": null,                       // 可选，缺省自动分配
  "wait": true,                           // true=同步等到结束返回 summary；false=后台跑，立即返回 site_id
  "force_recreate": false                 // 已存在同 dbnum 站点时是否重建
}
```

**响应体（`QuickDeployTestResponse`）**

```jsonc
{
  "success": true,
  "site_id": "avevaplantsample-aps250164-8081",
  "dbnum": 250164,
  "resolved_db_file": "aps000/aps250164_0001",
  "parse_status": "Parsed",
  "generated": true,
  "entry_url": "http://127.0.0.1:8081/",   // start_site=true 时
  "duration_ms": 12345,
  "parse_log_tail": ["..."],
  "generate_log_tail": ["..."],
  "warnings": []
}
```

### 2.2 复用 vs 新增

| 能力 | 复用 | 新增 |
|---|---|---|
| 建站 | `managed_project_sites::create_site` | —— |
| 解析 | `run_parse_pipeline` / `spawn_parse_process` | —— |
| 生成 | `run_generation_pipeline(parse_first=false)` | —— |
| 一键编排 | —— | `quick_deploy_test(req)` 编排函数 |
| path/filename → dbnum | `parse_file_basic_info`（pdms-io）/ `scan_db_file_name` | `resolve_dbnum_from_db_file()` |
| 路由 | `create_admin_routes` | `.route("/quick-deploy-test", post(quick_deploy_test_handler))` |

### 2.3 dbnum 解析（G1）

```rust
/// 解析用户给的 db_file（文件名 / 绝对路径 / 相对路径）为 dbnum。
/// 优先读文件头（parse_file_basic_info）取真实 numbdb；不依赖文件名编号。
fn resolve_dbnum_from_db_file(project_path: &Path, db_file: &str) -> Result<u32> {
    // 1) 归一成绝对路径（绝对/相对 project_path/仅文件名→在 project_path 下递归找）
    // 2) 读文件头 -> DbPageBasicInfo.dbnum
}
```

### 2.4 命名与幂等（G6）

- **默认名递增（评审反馈）**：未提供 `project_name` 时，E3D 项目名取 `project_path` 目录名（解析需要），**站点显示名用默认递增名** `quicktest-{N}`（N 为现有 `quicktest-*` 站点的最大序号 +1）。提供 `project_name` 时站点名沿用之。
- `site_id` 由站点名 + 端口推导；同名重复创建走幂等。
- 已存在：`force_recreate=false` → 复用并重置 parse/gen 状态后重跑；`true` → 删站重建。

### 2.5 进度回传（G4）

- MVP：`wait=true` 同步等 pipeline 结束，返回 summary + 日志 tail（`tail_log`）。
- 增强：`wait=false` 立即返回 `site_id`，前端用现有 `GET /{id}/runtime` 轮询，或复用 admin SSE（`sse_handlers::push_admin_site_*`）。

### 2.6 关联库精确解析（G5，后续完善）

把 `RELATED_DEPENDENCY_DB_TYPES` 粗粒度替换为：从目标 dbnum 的 DESI 元素读取其 `SPEC`/`CATA REF` 引用 → 反查所属 dbnum → 仅纳入被引用的依赖库。落在 `resolve_included_db_files` 的 `auto_parse_related_dbnums` 分支（已留 TODO 锚点）。

---

## 3. 分阶段实施

### Phase 1 — 一键 endpoint MVP（G1+G2+G3）
- 改动：`models.rs`（请求/响应类型）、`managed_project_sites.rs`（`resolve_dbnum_from_db_file` + `quick_deploy_test` 编排）、`admin_handlers.rs`（路由 + handler）
- 行为：`wait=true` 同步跑「建站→解析→生成」，返回 summary
- 验收：`cargo check --lib --features web_server` 通过；curl 单次调用返回 `parse_status=Parsed, generated=true`

### Phase 2 — 进度/结果回传（G4）
- 改动：handler 增加 `wait=false` 分支 + 返回日志 tail（复用 `tail_log`）；可选接 admin SSE
- 验收：`wait=false` 立即返回 `site_id`；`GET /{id}/runtime` 能看到 `parse_status/last_error`

### Phase 3 — 幂等 + 清理 + 失败诊断（G6）
- 改动：稳定 `site_id` 推导；`force_recreate` 分支；失败时回传 `parse_log_tail` 关键行
- 验收：同一文件连续两次调用不报"站点已存在"；`force_recreate=true` 能重建

### Phase 4 — 关联库精确解析（G5，后续完善）
- 改动：`resolve_included_db_files` 的 `auto_parse_related_dbnums` 分支换精确实现
- 验收：开启开关时，只纳入目标 DESI 实际引用到的 CATA/DICT dbnum（对比粗粒度版文件数下降）

### Phase 5 — 真实回路验证（G7）
- 起 `web_server`（debug），curl 打 `quick-deploy-test`，目标 `aps250164_0001`
- 验收：见 §5

---

## 4. 文件变更总览

| 操作 | 文件 | 说明 |
|---|---|---|
| 修改 | `src/web_server/models.rs` | `QuickDeployTestRequest` / `QuickDeployTestResponse` |
| 修改 | `src/web_server/managed_project_sites.rs` | `resolve_dbnum_from_db_file()` + `quick_deploy_test()` 编排；Phase4 精确关联解析 |
| 修改 | `src/web_server/admin_handlers.rs` | 路由 `/api/admin/sites/quick-deploy-test` + `quick_deploy_test_handler` |
| 复用 | `create_site` / `run_parse_pipeline` / `run_generation_pipeline` / `tail_log` | 不改语义 |

---

## 5. 验证计划（真实回路，免 test）

```bash
# 1) 起 web_server（debug 增量）
cd D:\work\plant-code\plant-model-gen
cargo run --bin web_server --features web_server   # :3100

# 2) 一键部署测试（同步等结果）
curl -s -X POST http://127.0.0.1:3100/api/admin/quick-deploy-test ^
  -H "Content-Type: application/json" ^
  -d "{\"project_path\":\"D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample\",\"db_file\":\"aps250164_0001\",\"gen_model\":true,\"wait\":true}"

# 期望：success=true, dbnum=<解析出的>, parse_status=Parsed, generated=true
```

验收点：
- 响应 `parse_status=Parsed`、`generated=true`
- 站点 SurrealDB 落库（`inst_relate` 有数据）
- `auto_parse_related_dbnums=true` 时日志能看到额外纳入 CATA/DICT（Phase4 后变为精确集合）

---

## 6. 风险与回退

| 风险 | 触发 | 回退 |
|---|---|---|
| 单 DESI 缺元件库导致管道/元件类生成不全 | `auto_parse_related_dbnums=false` 且引用外部 CATA | 开启开关；Phase4 精确纳入 |
| 一键编排长耗时阻塞 HTTP | 大库 `wait=true` | 默认/大库用 `wait=false` + runtime 轮询 |
| 文件名编号≠dbnum 解析错 | 仅按文件名猜 | 强制读文件头取 numbdb |
| 端口/站点冲突 | 自动分配撞车 | 复用现有 `resolve_create_ports` + 幂等 site_id |
| 免鉴权被误用到生产 | 该端点裸奔 | 仅 debug/测试环境暴露；文档标注「快测专用」 |

---

## 7. 不做（Out of Scope）

- 不引入鉴权（按需求保持免鉴权快测）。
- 不做远端部署（`123.57.182.243`）。
- 不替代正式多步部署 UI / 流程。
- 不在本功能内重构 `config` crate / `OnceCell` 等既有 backlog。

---

## 8. 时间预算

| 阶段 | 预估 |
|---|---|
| Phase 1 MVP | 0.5–1 天 |
| Phase 2 进度回传 | 0.5 天 |
| Phase 3 幂等/清理 | 0.5 天 |
| Phase 4 关联精确解析 | 1 天 |
| Phase 5 验证 | 0.5 天 |
| 合计 | ~3 天 |

---

## 9. 立即行动

确认本计划后，从 **Phase 1** 开始：在 `models.rs` 定义 `QuickDeployTestRequest/Response`，在 `managed_project_sites.rs` 实现 `resolve_dbnum_from_db_file` + `quick_deploy_test`，在 `admin_handlers.rs` 挂路由，`cargo check --lib --features web_server` 验证后用 §5 的 curl 跑真实回路。
