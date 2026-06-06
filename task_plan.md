# Release 包 Nginx 客户入口修复计划

## Goal

让 Windows release 部署包能稳定提供客户可直接访问的 `plant3d-web` 根站点入口：

`http://<host>/?output_project=<project>&show_dbnum=<dbnum>`

同时保留现有 `web_server` 内置 `/viewer/` fallback，确保没有 Nginx 或 Nginx 配置失败时仍可本机/局域网验证。

## Current Phase

Phase R5

## Phases

### Phase R1: 现状定界与部署契约锁定

- [x] 分析当前源码 Nginx 自动配置能力：`managed_project_sites.rs` 已有 Windows/Linux 分支、配置生成、`nginx -t`、reload/start 逻辑。
- [x] 分析当前已打 release 包：包内 `viewer/index.html` 仍以 `/viewer/` 为 base，`BUILD_INFO.json` 的 `viewerUrl` 仍是 `http://127.0.0.1:3100/viewer/`。
- [x] 确认当前包内没有 `nginx*` 文件，也没有启动脚本自动设置 `AIOS_NGINX_*` / `AIOS_VIEWER_*`。
- [x] 使用 `grill-me` 收敛关键决策：Windows 开箱即用、Linux 接管系统 Nginx、客户根路径 `/`、保留 `/viewer/` fallback。
- **Status:** complete

### Phase R2: Release 包前端产物布局

- [x] 修改 `scripts/package/build-windows-bundle.ps1`，打包两份前端静态产物：
  - `viewer/`：`VITE_BASE_PATH=/viewer/`，继续服务内置 `/viewer/` fallback。
  - `viewer-root/`：`VITE_BASE_PATH=/`，作为 Nginx 客户根入口。
- [x] 更新 `BUILD_INFO.json` / `README-安装说明.md`，区分本地 fallback URL 和客户 Nginx URL。
- [x] 构建新包后确认 `viewer-root/index.html` 引用 `/assets/...`，`viewer/index.html` 引用 `/viewer/assets/...`。
- **Status:** complete

### Phase R3: Runtime Nginx 配置适配 release 包

- [x] 在 `managed_project_sites.rs` 中新增/复用 `AIOS_VIEWER_STATIC_ROOT`，Nginx root 优先指向包内 `viewer-root/`，不再只假设 `plant3d-web/dist`。
- [x] Windows Nginx 配置 listen 端口改为 `viewer_base_listen_port(site)`，不要硬编码 `80`。
- [x] Nginx 配置增加 `/admin/` 代理到主控 `web_server` 端口，`/api/`、`/files/`、`/ws/` 继续代理到当前站点 `site.web_port`。
- [x] Windows Nginx 失败默认降级为 fallback；仅在显式 `RequireNginx`/等价环境变量时 fatal。
- **Status:** implementation complete

### Phase R4: Windows 启动脚本与可选 Nginx 打包

- [x] 修改 `start-plant3d.ps1`：自动检测包内或外部 Nginx，设置 `AIOS_NGINX_BIN`、`AIOS_NGINX_ROOT`、`AIOS_VIEWER_STATIC_ROOT`、`AIOS_VIEWER_BASE_URL`。
- [x] 增加启动参数：`-EnableNginx auto|on|off`、`-ViewerHost`、`-ViewerPort`、`-RequireNginx`。
- [x] `build-windows-bundle.ps1` 支持可选复制 `tools/nginx/windows/nginx.exe`；缺失时 warning，不阻断打包.
- [x] `install-service.ps1/.bat` 传递 Nginx 相关参数，计划任务启动路径与交互启动一致。
- **Status:** implementation complete

### Phase R5: Windows release 包验收

- [x] 构建新的 Windows release 包，确认目录包含 `viewer/`、`viewer-root/`、启动脚本和可选 `bin/nginx/nginx.exe`。
- [x] 无 Nginx 场景：运行 `start-plant3d.bat`/`start-plant3d.ps1`，验证 `http://127.0.0.1:<web_port>/viewer/` 可用。
- [x] 有 Nginx 场景：运行启动脚本，验证 `/`、`/assets/`、`/api/`、`/files/`、`/admin/` 关键路径。
- [ ] 使用真实 quick deploy 站点打开客户 URL，确认模型加载且无 `backend=` 参数。
- **Status:** in progress; formal release package with bundled nginx passed startup smoke, per-site Nginx proxy smoke including `/files/` passed, real quick deploy model load pending

### Phase R6: Linux/system Nginx 文档与远端部署对齐

- [ ] 明确 Linux 包不捆绑 Nginx，继续生成 `/etc/nginx/conf.d/plant3d-web-<site_id>.conf` 或用户指定目录。
- [ ] 文档说明多站点限制：一个 `host:port` 只绑定一个客户站点，多站点需不同域名/IP/端口。
- [ ] 更新部署指南和 Nginx 示例，覆盖 `viewer-root/`、`/admin/` 代理、fallback 行为。
- **Status:** pending

## Grill-Me Decision Tree

### Question 1: release 包 Nginx 目标是开箱即用还是接管已有 Nginx？

**推荐答案：Windows 开箱即用，Linux 接管系统 Nginx。**

理由：Windows 客户现场更需要双击启动闭环；Linux 服务器通常已有系统 Nginx、权限和运维规范，不应强行捆绑。

### Question 2: 是否接受两份 plant3d-web 静态产物？

**推荐答案：接受。**

理由：`/viewer/` fallback 和客户根路径 `/` 的 base path 天然不同。两份产物能避免 rewrite/alias 复杂度，也不破坏现有单进程包。

### Question 3: Nginx listen 端口如何决定？

**推荐答案：跟 `AIOS_VIEWER_BASE_URL` 保持一致。**

理由：Windows 当前硬编码 `listen 80` 会导致 `AIOS_VIEWER_BASE_URL=http://host:8080` 时 URL 与实际监听不一致。

### Question 4: `/admin/` 是否也走 Nginx？

**推荐答案：走，但代理到主控 web_server。**

理由：客户 Viewer 的 `/api/`、`/files/`、`/ws/` 应代理当前站点 `site.web_port`；管理后台应回到主控端口（默认 3100），方便同一 host 排查和维护。

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Windows 包保留 `/viewer/` fallback | 无 Nginx 或 Nginx 失败时仍可验证 Viewer。 |
| 新增 `viewer-root/` 给 Nginx 根路径 | 客户 URL 需要 `/`，不能依赖 `/viewer/` base。 |
| Nginx root 不再强依赖 `plant3d-web/dist` | release 包只有静态产物，不一定有前端源码项目和 npm 环境。 |
| 一个 `host:port` 绑定一个客户站点 | 同源 `/api/` 无法仅靠 query 参数区分多个站点。 |
| Nginx 自动化先 `nginx -t` 再 reload/start | 避免坏配置影响已有服务。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| 用户级 `planning-with-files/scripts/session-catchup.py` 不存在 | 按 skill 首步执行 catchup | 记录为非阻塞；继续读取现有 planning 文件并前置本计划。 |
| `Glob` 在 Windows 绝对路径搜索时返回 `os error 3` | 搜索 release 包和 nginx 文件 | 改用 `rg`、精确 `ReadFile`、包内路径读取完成分析。 |

## Notes

- 关键文件：
  - `scripts/package/build-windows-bundle.ps1`
  - `scripts/package/start-plant3d.ps1`
  - `scripts/package/start-plant3d.bat`
  - `scripts/package/install-service.ps1`
  - `scripts/package/install-service.bat`
  - `src/web_server/managed_project_sites.rs`
  - `src/web_server/mod.rs`
  - `shells/deploy/nginx-plant3d-web.conf.example`
- 当前已验证包样本：
  - `runtime/codex-validation/full-deploy-aveva-package-18572/Plant3D-AIOS-win-x64`
  - `runtime/codex-validation/full-deploy-aveva-package-kv-18572/Plant3D-AIOS-win-x64`
- 当前判断：源码已有 Nginx 自动配置雏形，但已打 release 包不能保证正确配置 Nginx；需要打包、运行时和启动脚本三处协同修复。

## Archived Previous Plan

# AMS BRAN quick deploy 解析放大修复计划

## Goal

让 admin 站点自动部署能稳定服务 AvevaMarineSample 单 BRAN 验证：使用 `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams7999_0001` 作为 dbfile，通过 quick deploy/admin 归档配置创建站点，避免 `wait=true` 长请求超时和 `auto_parse_related_dbnums` 粗粒度 CATA 放大，最终验证 `BRAN 24383/73930` 的生成、导出和前端显示。

## Current Phase

Phase 4

## Phases

### Phase 1: 代码审查与问题定界

- [x] 审查 `QuickDeployTestRequest.wait` 默认值及同步 pipeline 行为，确认大项目会阻塞 HTTP 请求直到 parse/generate/start 完成。
- [x] 审查 `quick_deploy_site()` 与 `quick_deploy_test()` 复用关系，确认 admin 入口当前复用测试部署语义。
- [x] 审查 `should_run_db_index_prescan()` 与 `manual_db_nums` 的关系，确认单库 quick deploy 不会触发精确 db_index 预扫。
- [x] 审查 `resolve_included_db_files_detailed()` 的 `auto_parse_related_dbnums` fallback，确认精确依赖为空/失败时显式单库路径会回退纳入 CATA 依赖。
- **Status:** complete

### Phase 2: Grill-Me Contract Decisions

- [x] 决定 admin quick deploy 是否应默认 `wait=false`。推荐：是；admin 入口应立即返回 `site_id/task_id`，长流程用任务状态和日志观察。
- [x] 决定单库 quick deploy 是否应在有 `manual_db_nums` 时也跑 db_index prescan。推荐：是；`auto_parse_related_dbnums=true` 的目的就是精确补依赖，不能因 manual scope 跳过。
- [x] 决定精确依赖为空/失败时是否允许默认全 CATA fallback。推荐：不允许默认；仅当请求显式开启粗粒度 fallback 时才纳入 CATA。
- [x] 决定测试 quick deploy 与 admin quick deploy 是否需要拆分。推荐：需要；admin 部署不应复用固定 `quicktest / QuickTest@2026` 凭据语义。
- **Status:** complete

### Phase 3: 最小代码修复

- [x] 将 admin quick deploy 默认改为后台任务模式：鉴权版 `/api/admin/sites/quick-deploy` 强制走 `quick_deploy_admin()`，响应包含 `site_id` 和 `task_id`。
- [x] 调整 `should_run_db_index_prescan()`：当 `auto_parse_related_dbnums=true` 时允许预扫，包括存在 `manual_db_nums` 的单库 quick deploy。
- [x] 调整 `resolve_included_db_files_detailed()` fallback 策略：精确依赖为空/失败时默认不回退全 CATA，仅解析目标库与必要系统/字典库。
- [x] 将 admin quick deploy 与 quick deploy test 的凭据语义拆开：test 保留 `quicktest / QuickTest@2026`，admin 使用 per-dbnum 默认凭据。
- **Status:** complete

### Phase 4: AMS BRAN 重部署验证

- [ ] 停止或隔离当前长时间解析的 AMS 旧任务，避免它继续占用端口/CPU/站点状态。
- [ ] 用 `db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams7999_0001` 重新创建唯一站点，确认 project/site 名称不重名、DB 端口和 Web 端口都通过可用性检查。
- [ ] 确认 parse plan 只包含目标库、必要系统/字典库和精确依赖闭包，不再默认纳入大量 CATA 文件。
- [ ] 等待后台 parse/generate 完成，记录 `parse_status`、`generate.log`、Parquet 输出和 SQLite spatial index 统计。
- [ ] 验证 `BRAN 24383/73930` 可通过 API/导出数据定位，并在 viewer 中显示。
- **Status:** pending

### Phase 5: 文档与验收收口

- [ ] 更新 quick deploy/admin 部署契约文档：默认后台、端口必须预检、依赖解析优先 db_index 精确闭包。
- [ ] 更新 `findings.md` / `progress.md`，记录 AMS BRAN 部署证据、剩余风险和复跑命令。
- [ ] 输出最终结论：修复点、验证证据、如何用 admin 站点复现。
- **Status:** pending

## Grill-Me Decision Tree

### Question 1: admin quick deploy 能不能默认同步等待完整部署完成？

**推荐答案：不能。**

理由：解析和生成是分钟到小时级后台任务，HTTP 同步等待会制造客户端超时，并让调用方误判“部署失败/卡死”。admin 入口应该返回任务句柄，让状态页和日志承担观察职责。

### Question 2: 单库 quick deploy 有 `manual_db_nums` 时是否还要做 db_index 预扫？

**推荐答案：要。**

理由：`manual_db_nums` 表示目标库范围，不表示依赖库已经解析完。`auto_parse_related_dbnums=true` 时应先用 db_index 精确闭包补依赖，否则会落入粗粒度 CATA fallback。

### Question 3: 精确依赖为空时是否可以默认回退全 CATA？

**推荐答案：不能默认。**

理由：全 CATA fallback 对 APS 小样本可能可接受，但对 AMS 会把单 BRAN 验证放大成大范围解析。fallback 应成为显式风险选项，默认给出“依赖为空/预扫失败”的可操作错误或警告。

### Question 4: admin quick deploy 是否可以复用 quick deploy test 的固定凭据？

**推荐答案：不应复用。**

理由：测试入口可以使用固定 quicktest 凭据，但 admin 站点部署会归档配置并长期运行，应使用 per-site 凭据或配置来源，避免把测试语义写进正式部署链路。

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| admin quick deploy 默认后台化 | 避免 AMS 这类大库解析超过 HTTP 客户端超时。 |
| 单库自动依赖也需要 db_index prescan | quick deploy 的目标是“单目标 + 精确依赖”，不是“单目标 + 全 CATA”。 |
| 粗粒度 CATA fallback 必须显式 | 默认 fallback 会隐藏索引缺失/预扫失败，并造成解析范围失控。 |
| admin/test 部署语义需要拆分 | 测试固定凭据和正式站点归档不应混用。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| AMS BRAN quick deploy 请求 900s 超时 | 1 | 定界为客户端同步等待超时；后台 parse 仍继续运行。计划改为 admin 默认后台任务。 |
| 单库 AMS 解析纳入大量 CATA 文件 | 1 | 定界为 `manual_db_nums` 跳过 db_index prescan 后触发 CATA fallback。计划改为单库也预扫，且默认禁用粗粒度 fallback。 |
| 当前 MCP skill catchup 脚本路径不存在 | 1 | 记录为非阻塞；继续读取现有 planning 文件并更新 active plan。 |

## Notes

- 关键文件：
  - `src/web_server/models.rs`
  - `src/web_server/admin_handlers.rs`
  - `src/web_server/managed_project_sites.rs`
  - `src/web_server/admin_task_handlers.rs`
- 当前目标 BRAN：`24383/73930`
- 当前目标 dbfile：`D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams7999_0001`
- 旧问题站点：`avevamarinesample-bran-24383-73930-20260603-215256-8081`

## Archived Previous Plan

# AABB Parquet 导入 SQLite Index 开发计划

## Goal

把 `aabb.parquet` / `instances.parquet` / `tubings.parquet` 到 `output/spatial_index.sqlite` 的链路从“代码已接入”推进到“可独立重建、可真实验收、可被房间计算稳定消费”：明确 Parquet 是生产主路径，补齐独立 CLI 入口与 feature/文档契约，跑 quick deploy 真实导出，验证 SQLite RTree 行数、dbnum 替换语义、BRAN/HANG/EQUI 聚合和房间计算粗筛行为。

## Current Phase

Phase 7

## Phases

### Phase 1: Context & Implementation Audit

- [x] 确认默认项目根目录为 `D:/work/plant-code/plant-model-gen`。
- [x] 读取 `src/sqlite_index.rs`，确认 `refresh_dbnum_from_parquet_dir()` 已实现 Parquet 到 SQLite RTree 的核心导入。
- [x] 读取 `src/cli_modes.rs`，确认 `export_dbnum_instances_parquet_mode()` 已在 Parquet 导出成功后自动刷新 SQLite spatial index。
- [x] 读取 `src/fast_model/room_model.rs`，确认生产主路径注释已切到 Parquet 导出后的预建 SQLite RTree，legacy `inst_relate_aabb` 刷新仅作显式/debug 入口。
- [x] 读取 `src/fast_model/gen_model/orchestrator.rs`，确认生成后不再走 SurrealDB 刷新，等待 Parquet 导出成功后刷新 SQLite。
- **Status:** complete

### Phase 2: Grill-Me Contract Decisions

- [x] 决定是否把独立 `parquet-dir -> sqlite index` CLI 纳入本轮。推荐：纳入，否则索引损坏/缺失时只能重跑模型导出。
- [x] 决定 `tubings.parquet` 缺失是否 fatal。推荐：新导出包必须包含空表或实表，导入缺失时 fatal；旧包兼容另起显式 flag，不进入默认路径。
- [x] 决定刷新粒度。推荐：按 `dbnum` 替换该 dbnum 的旧 rows，不删除其他 dbnum，沿用 `replace_dbnum_aabbs_with_items_and_spec_values()`。
- [x] 决定房间计算是否允许自动 legacy fallback。推荐：默认不 fallback；缺失预建 index 时给明确错误，显式 debug 命令才允许 legacy 重建。
- **Status:** complete

### Phase 3: CLI & Feature Gate Closeout

- [x] 增加或暴露独立 CLI：输入 Parquet 目录、`dbnum`、可选 SQLite 输出路径，调用 `refresh_dbnum_from_parquet_dir()`。
- [x] 更新 `main.rs` help 文案，区分旧 `instances.json` 导入和新 Parquet 导入。
- [x] 确认默认/目标构建 feature 同时包含 `sqlite-index` 与 `parquet-export`；缺少 feature 时输出明确错误。
- [x] 保留 `--import-spatial-index` 的 JSON 兼容入口，不改变已有用户脚本语义。
- **Status:** complete

### Phase 4: Real Quick Deploy Export Validation

- [x] 先完成 quick deploy test 当前阻塞：同名旧站点端口复用 `409 Conflict` 与 `query_ptset_export_data()` fatal。
- [x] 使用 quick deploy test 生成并归档的 admin 站点配置跑 scoped Parquet 导出，不直接拼临时 `DbOption.toml`。
- [x] 确认输出目录包含 `aabb.parquet`、`instances.parquet`、`tubings.parquet` 和 `manifest.json`。
- [x] 确认 quick deploy 生成后刷新默认 `output/spatial_index.sqlite`；自动导出路径未在 tail 中显式打印 inserted 行，改用 SQLite 统计确认。
- [x] 记录默认 SQLite 索引文件、大小、行数和样本 refno。
- **Status:** complete

### Phase 5: SQLite Index Correctness Verification

- [x] 查询 SQLite `items` 与 `aabb_index` 总数，确认和导入 `inserted` 一致。
- [x] 抽样验证 `instances.parquet` 的真实 `refno_u64` 被保留为 RTree id，旧的 `250160/<refno>` 伪 id 已被清理。
- [x] 验证历史包空 `aabb_hash` 行会被跳过并计数，非空 hash 缺失仍 fatal；`250164` 跳过 243 行后导入成功。
- [x] 使用全仓可发现 Parquet 包做 clean rebuild：13 个数字 dbnum 目录导入成功，`aabb_index=41270`、`items=41270`、`orphan_aabb=0`、`duplicate_item_ids=0`。
- [ ] 验证 TUBI rows 能写入 `TUBI` 项，并能合并到 BRAN/HANG owner AABB；已扫描 13 个可发现 `tubings.parquet`，全部为 0 行，当前无可用样本。
- [x] 验证 EQUI owner 聚合语义：样本写入 3 条 EQUI 聚合 row。
- [x] 重复导入同一 dbnum，确认按 `items.dbnum` 替换该 dbnum 的旧 rows，同时保留其他 dbnum 既有行。
- **Status:** partial

### Phase 6: Room Compute Consumption Smoke

- [x] 使用预建 `output/spatial_index.sqlite` 跑一次 spatial query 粗筛路径，确认 `SqliteSpatialIndex::with_default_path()` 可直接命中候选。
- [ ] 确认默认 `RoomComputeOptions` 不触发 legacy `inst_relate_aabb` rebuild。
- [x] 对一个已知 refno 记录候选数量：`2013286704/431`，`distance_mm=1000`，返回 496 个候选并包含自身。
- [x] 若预建 index 缺失，确认错误信息指向“先跑 Parquet 导出/导入”，而不是静默 fallback。
- **Status:** partial

### Phase 7: Documentation & Progress Closeout

- [x] 更新 `findings.md` / `progress.md`，记录命令、日志、SQLite 统计和样本路径。
- [ ] 在合适的开发文档或 README 中说明两条入口：导出后自动刷新、独立 Parquet 导入刷新。
- [ ] 输出最终中文结论：已完成实现、验证证据、残留风险和下一条命令。
- **Status:** in_progress

## Grill-Me Decision Tree

### Question 1: 是否需要独立 Parquet 导入 CLI，而不是只依赖导出后自动刷新？

**推荐答案：需要。**

理由：`refresh_dbnum_from_parquet_dir()` 已经是纯函数式导入入口，暴露 CLI 成本低；真实部署中 `spatial_index.sqlite` 可能丢失、损坏或需要重建，如果只能靠重新导出模型，会把索引修复和模型生成耦合在一起。

### Question 2: `tubings.parquet` 缺失时是否应该继续导入？

**推荐答案：默认 fatal。**

理由：当前导入逻辑依赖 TUBI 细粒度 rows 以及 BRAN/HANG owner 聚合。新导出包应始终写出空 `tubings.parquet` 或实表；旧包兼容可以做显式 flag，不能进入默认生产路径。

### Question 3: 房间计算能否在缺失 SQLite index 时自动回退 legacy 刷新？

**推荐答案：不能默认回退。**

理由：生产主路径已经切换为 Parquet 导出后预建 SQLite RTree。默认回退会掩盖部署包缺索引的问题，也可能重新引入 SurrealDB 依赖；legacy rebuild 应保留在显式 debug/repair 命令。

### Question 4: 多 dbnum 共用一个 `spatial_index.sqlite` 时如何刷新？

**推荐答案：按 dbnum 局部替换。**

理由：当前 `replace_dbnum_aabbs_with_items_and_spec_values()` 已支持按 `items.dbnum` 删除旧 rows 后插入新 rows，可以避免刷新一个 dbnum 时误删其他 dbnum 的索引。RTree `id` 保留真实 PDMS `refno_u64`，所以 `id >> 32` 是 `ref0`，不能作为导出 dbnum 使用。

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Parquet 是 AABB SQLite index 的生产主路径 | 源码已将房间计算主路径切到预建 SQLite RTree，legacy `inst_relate_aabb` 刷新只保留为显式/debug 入口。 |
| 导出后自动刷新必须保留 | `export_dbnum_instances_parquet_mode()` 已接线，quick deploy 完成模型导出时应顺带产出可消费的 SQLite index。 |
| 独立 CLI 应作为本轮缺口补齐 | 便于索引重建、部署包修复和验证，不需要重跑完整模型生成。 |
| 不运行 Rust test 作为默认验证 | 遵守仓库规则；验证使用 CLI、quick deploy、Parquet/DuckDB/SQLite 检查和房间计算 smoke。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `planning-with-files` 项目 skill 缺少 `scripts/session-catchup.py` | 1 | 记录为非阻塞；继续读取现有 planning 文件与源码上下文制定计划。 |
| 当前 planning 文件 active 主题不是 AABB SQLite index | 1 | 将本计划前置为新的 active plan，原 quick deploy 计划保留归档，因为它仍是 Phase 4 验收依赖。 |
| APS `refno_str` 被误判为 dbnum_refno | 1 | `src/sqlite_index.rs` 改为优先使用 Parquet 数值列 `refno_u64` 构造 SQLite id；clean SQLite 导入验收通过。 |
| 默认 `output/spatial_index.sqlite` 有历史错误 id 残留 | 1 | quick deploy 复跑后局部替换会按 `items.dbnum` 清理真实 rows，并额外清理旧版 `(dbnum << 32)` 错误 id range；默认索引 smoke 已通过。 |
| 历史 `instances.parquet` 存在空 `aabb_hash` | 1 | 导入端跳过空 hash 并计数；非空 hash 缺失仍 fatal。`250164` 与全仓 clean rebuild 均通过。 |
| 空 SQLite index smoke 报 db_meta/cache 错误 | 1 | `spatial_query_refno_mode()` 增加空索引前置检查，直接提示先完成 Parquet 导出自动刷新或运行 `--import-spatial-index-parquet` 重建。 |

## Notes

- 关键文件：
  - `src/sqlite_index.rs`
  - `src/cli_modes.rs`
  - `src/main.rs`
  - `src/fast_model/gen_model/orchestrator.rs`
  - `src/fast_model/room_model.rs`
- 当前实现状态：核心导入函数、独立 Parquet CLI、quick deploy Parquet 产出、空 AABB 历史包兼容、clean SQLite 导入和 spatial query smoke 已落地。
- 下一步第一条动作：找一个带非空 `tubings.parquet` 的样本补齐 TUBI/BRAN-HANG owner 聚合验证，或生成最小 fixture 覆盖该导入路径。

## Archived Previous Plan

# quick deploy test 部署失败修复计划

## Goal

让 admin 站点里的 quick deploy test 成为默认、可重复的部署验证路径：先通过站点名/项目名/端口冲突检查生成并归档可行配置，再跑解析、生成、后置 Parquet 导出；当前要修复 `quicktest-250160-8080` 的两层失败：复用旧站点时 DB 端口 `8020` 冲突导致 `409 Conflict`，以及生成成功后 `query_ptset_export_data()` 的 ptset/cata_hash 查询失败导致部署任务退出 1。

## Current Phase

Phase 4

## Phases

### Phase 1: Repro Loop & Failure Split

- [x] 启动本地 `web_server` 并开启 `AIOS_ENABLE_QUICK_DEPLOY_TEST=1`。
- [x] 通过 `POST /api/admin/quick-deploy-test` 复跑 APS 单库 `aps250160_0001`。
- [x] 确认当前复跑失败为 `409 Conflict`，尚未进入生成阶段。
- [x] 通过 admin runtime 确认 `quicktest-250160-8080` 旧归档 DB 端口是 `8020`，当前被主 `web_server` 自启动的 SurrealDB 占用。
- [x] 通过历史 `generate.log` 确认第二层失败在 `export_parquet_after_gen` 的 `query_ptset_export_data()`。
- **Status:** complete

### Phase 2: Quick Deploy Port Reuse Fix

- [x] 设计 quick deploy 复用同名旧站点时的端口策略：旧站点 stopped/failed 且端口被外部进程占用时，应重新分配 DB/Web 端口，或明确要求 `force_recreate=true`。
- [x] 保留用户显式传入端口的严格冲突失败语义；只对自动分配端口的 quicktest 站点做自动重分配。
- [x] 确保 `project_name` / `site_name` 唯一性检查和 admin 站点归档配置仍在创建/更新事务中完成。
- [x] 更新 `runtime/admin_sites/<site_id>/DbOption*.toml` 写盘逻辑，使重分配后的端口落到归档配置。
- **Status:** complete

### Phase 3: Ptset Parquet Query Robustness

- [x] 修改 `query_ptset_export_data()`，避免在 SurrealQL 中使用不稳定的 `record::id(out[0])` fallback。
- [x] 只在 SQL 层读取稳定字段：`in as refno`、`out[0].cata_hash`、`out[0].ptset`；缺失/非法 `cata_hash` 在 Rust 层计入 `missing_cata_hash_refnos` 后跳过。
- [x] 保留 `ptset_export` 诊断字段，确保缺失 ptset/cata_hash 不再让部署 fatal。
- [x] 如果真实数据需要 record id 兜底，改为后续单独验证的兼容路径，不放在 quick deploy 的主查询里。
- **Status:** complete

### Phase 4: End-To-End Quick Deploy Validation

- [ ] 重新运行 quick deploy test，确认不再因端口复用返回 `409 Conflict`。
- [ ] 确认解析阶段 `parse_status=Parsed`，生成阶段不因 ptset Parquet 导出退出 1。
- [ ] 检查站点 runtime：`status`、`parse_status`、`db_port_conflict`、`last_error`。
- [ ] 检查 `generate.log`：模型生成完成、Parquet manifest 写出、`ptset_export` 统计合理。
- [ ] 确认 admin 站点部署配置已归档且可复现。
- **Status:** pending

### Phase 5: Closeout & Regression Guard

- [ ] 更新 `findings.md` / `progress.md`，记录命令、payload、响应、日志路径和结论。
- [ ] 如有合适 seam，补最小 CLI/HTTP smoke；不运行 Rust test，遵守仓库规则。
- [ ] 输出最终结论：修复点、验证结果、残留风险。
- **Status:** pending

## Grill-Me Decision Tree

### Question 1: quick deploy 复用同名 failed 站点时，是否允许自动重分配端口？

**推荐答案：允许，但只限自动分配端口的 quicktest 场景。**

理由：当前 `quicktest-250160-8080` 是自动生成站点，旧 DB 端口 `8020` 被外部进程占用时，复用路径继续保留旧端口会让 quick deploy 卡死在 `409`，无法发挥“快速生成可行配置”的作用。若用户显式指定了端口，则冲突应继续失败，避免悄悄偏离用户配置。

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| quick deploy test 是默认部署验证入口 | 它负责生成可行配置并归档 admin 站点部署配置，不能绕过站点/端口校验直接拼 `DbOption.toml`。 |
| 当前失败分两层处理 | 先解决当前复跑的端口冲突 `409`，再解决历史生成后 Parquet 导出 fatal。 |
| ptset/cata_hash 缺失不应导致部署 fatal | 缺失应进入 `ptset_export` 诊断统计；部署成功与 ptset 覆盖率验收分离。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `POST /api/admin/quick-deploy-test` 返回 `409 Conflict` | 1 | 当前复用旧站点 `quicktest-250160-8080`，旧 DB 端口 `8020` 被主 `web_server` 自启动的 SurrealDB 占用；下一步修复复用时的端口重分配策略。 |
| 历史部署任务退出码 1 | 1 | `generate.log` 显示模型生成成功后，`export_parquet_after_gen` 的 `query_ptset_export_data()` 使用 `record::id(out[0])` 查询失败；下一步修复 ptset 查询健壮性。 |
| `cargo check --features parquet-export --lib` 首次等待 build directory lock 且耗时较长 | 1 | 保持等待，最终 EXIT=0；输出仅有既有依赖 warning。 |

## Notes

- 当前复跑 payload 已保存到 `runtime/quick-deploy-last-payload.json`。
- 当前复跑响应文件 `runtime/quick-deploy-last-response.json` 为空，因为服务返回了 `409 Conflict` 且 PowerShell 没拿到响应体。
- 关键现场：
  - `runtime/admin_sites/quicktest-250160-8080/DbOption.toml`
  - `runtime/admin_sites/quicktest-250160-8080/logs/generate.log`
  - `src/web_server/managed_project_sites.rs`
  - `src/fast_model/export_model/export_dbnum_instances_parquet.rs`

## Archived Previous Plan

# ptset parquet measurement snapping 下一步开发计划

## Goal

把 ptset 测量捕捉从“核心代码已接入”推进到“真实数据包可验收”：确认 `cata_hash` 的 PE/实例侧来源，补齐后端导出契约风险，跑 scoped Parquet 导出，检查 `manifest` / `instances.parquet` / `ptsets.parquet`，再用 `plant3d-web` 浏览器联调四种测量模式，确保所有测量落点都来自 `ptset:*`。

## Current Phase

Phase 2

## Phases

### Phase 1: Contract Correction & Source Of Truth

- [x] 确认 `cata_hash` 的权威来源是 PE/refno 对应的实例信息：`EleGeosInfo.cata_hash` / instance cache / raw inst info，而不是从 `ptsets.parquet` 反推。
- [x] 复核 `export_dbnum_instances_parquet.rs::query_ptset_export_data()` 当前 `pe -> inst_relate -> inst_info(out)` 查询是否稳定等价于 PE 实例侧 `cata_hash`。
- [x] 如果真实数据里 `out[0].cata_hash` 可能为空，补更直接的 PE/instance-info 来源，或在 Surreal 查询中增加受控兜底；不要把 `ptset` 本身作为 `cata_hash` 来源。
- [x] 用最小静态验证确认后端改动不破坏现有 Parquet 导出 schema。
- **Status:** complete

### Phase 2: Scoped Parquet Export Verification

- [ ] 使用 quick deploy test 创建/复用 AvevaPlantSample 测试站点，先校验 `project_name` 不重名、DB 端口/站点端口不冲突，再生成可行配置。
- [ ] 确认 quick deploy test 生成的 admin 站点部署配置已归档，可从 `runtime/admin_sites/<site_id>/DbOption*.toml` 复现。
- [ ] 基于 quick deploy test 产出的站点配置恢复或启动 AvevaPlantSample SurrealDB 环境，避免删除 LOCK 或强杀未知进程。
- [ ] 跑 scoped 导出：使用站点配置对应的有效 APS `dbnum/root-refno`，输出到 `runtime/ptset-parquet-validation`。
- [ ] 检查 `manifest.json` / `manifest_7997.json` 是否包含 `tables.ptsets`、`ptset_unit`、`ptset_export`。
- [ ] 检查 `instances.parquet` 的 `cata_hash` 非空率，重点关注命中实例 refno 是否都能映射到 cata key。
- [ ] 检查 `ptsets.parquet` 是否按 `cata_hash + point_number` 写出局部关键点，行数和样本内容合理。
- **Status:** pending

### Phase 3: Frontend Loader Smoke

- [ ] 使用新导出的模型包，在 `plant3d-web` 中确认 `manifest.tables.ptsets` 能懒注册。
- [ ] 对一个已知有 ptset 的 refno 调用/触发 `queryPtsetByRefnoFromParquet(dbno, refno)`，确认返回 `success=true`、`ptset` 非空、`world_transform` 和 `unit_info` 正确。
- [ ] 对旧包或缺失 `ptsets.parquet` 的包确认 loader 返回明确不可用错误，不影响普通模型实例渲染。
- **Status:** pending

### Phase 4: Browser Measurement Acceptance

- [ ] 浏览器联调距离测量：起点/终点都必须登记 `ptset:<refno>#<point_number>`。
- [ ] 浏览器联调角度测量：起点/拐点/终点都必须登记 `ptset:*`。
- [ ] 浏览器联调点标高和高差测量：点击模型表面但未靠近 ptset 时不创建测量，只显示原因。
- [ ] 验证测量面板“ptset 测量捕捉”关闭后只暂停捕捉，不回退到表面测量。
- [ ] 验证 hover ptset 显示层在退出测量、切换构件、切换模式时正确清理。
- **Status:** pending

### Phase 5: Delivery & Issue Closeout

- [ ] 更新 GitHub issue `#26/#27/#28/#29` 或本地进度，记录命令、数据包路径、样本 refno、验收截图/结果。
- [ ] 如真实导出发现 `cata_hash` 缺失，记录根因和修复路径。
- [ ] 如前端 loader 或测量链路发现回归，补最小 smoke 脚本或类型/行为守护。
- [ ] 输出最终中文结论：已完成项、剩余风险、下一次首条命令。
- **Status:** pending

## Key Questions

1. `export_dbnum_instances_parquet.rs` 当前从 `inst_relate.out[0].cata_hash` 取值，是否覆盖所有需要测量的 PE/refno？
2. 真实 AvevaPlantSample 数据中，`EleGeosInfo.cata_hash` 是显式字段、record id、instance cache 字段，还是多路径并存？
3. `ptsets.parquet` 是否应该包含空表也写出，还是无 ptset 时省略表并禁用测量？当前方案倾向写出并通过 manifest 明确行数。
4. 旧模型包缺失 `ptsets.parquet` 时，UI 是禁用整个 xeokit 测量，还是仅显示“ptset 测量不可用”？当前实现倾向后者，但联调后再收口。

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| `cata_hash` 按 PE/实例侧数据处理 | 前端 hover 的事实源是命中的 refno/PE；`ptsets.parquet` 只提供 cata 定义下的局部关键点。 |
| 测量落点强制来自 `ptset:*` | 已确认普通表面点只能用于识别实例和触发加载，不能登记为测量结果。 |
| `ptsets.parquet` 按 `cata_hash + point_number` 组织 | 减少重复存储，符合同 cata 定义复用局部关键点的模型结构。 |
| 旧 `/api/pdms/ptset` 不作为测量正常兜底 | 测量链路强依赖当前模型包契约；API 保留给调试/点集面板。 |
| 不运行 Rust test / 前端 test 作为默认验证 | 遵守仓库规则；优先用 `cargo check`、CLI 导出、DuckDB/Parquet 检查和浏览器联调。 |
| 部署验证默认走 quick deploy test | quick deploy test 会先生成可行配置并归档到 admin 站点部署配置；不能绕过站点唯一性和端口冲突检查直接使用手写 `DbOption.toml`。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `planning-with-files` session-catchup / glob 前一轮被用户中断 | 1 | 改为直接读取项目根目录 `task_plan.md`、`findings.md`、`progress.md`，并增量更新顶部 active plan。 |
| 真实 scoped Parquet 导出尚未完成 | 1 | 历史阻塞为 `D:/backup-dbs/ams-8020.db LOCK` 或 `127.0.0.1:8020` 拒绝连接；下一步先恢复数据库环境，不删除锁、不强杀未知进程。 |
| 首次 `cargo check --features parquet-export --lib` 失败 | 1 | 失败点是 `aws-lc-sys` 找不到 NASM，非业务代码错误；临时把 `C:\Program Files\NASM` 加入 PATH 后重跑通过。 |
| 直接用 `DbOption-aps.toml` + 临时端口导出不符合部署验证路径 | 1 | 已停止临时 `18020` SurrealDB；后续改走 quick deploy test，使用其生成并归档的 admin 站点配置。 |

## Notes

- 关键文件：
  - `plant-model-gen/src/fast_model/export_model/export_dbnum_instances_parquet.rs`
  - `plant3d-web/src/composables/useDbnoInstancesParquetLoader.ts`
  - `plant3d-web/src/composables/useXeokitMeasurementTools.ts`
  - `plant3d-web/src/components/tools/MeasurementPanel.vue`
- 当前前端三处文件仍有未提交改动：loader、measurement tools、measurement panel。
- 后端目标文件当前工作树无 diff，但代码已包含 `ptsets.parquet` 写出和 manifest 声明。
- 下一步第一优先级不是 UI，而是确认 `cata_hash` 取值路径和真实导出数据。

## Archived Previous Plan

# DuckLake ModelWriter 下一步开发计划

## Goal

把 `plant-model-gen` 中已存在的 `DuckLakeModelWriterBackend` 与独立 `plant-model-ducklake` crate 的存储适配能力收敛为可验证的下一阶段方案：先完成现有 in-repo DuckLake writer 的编译、CLI、HTTP 与 9 张 raw 表写入/对账验证，再决定是否把 schema / writer planning 下沉复用到 `plant-model-ducklake`。

## Current Phase

Phase 4 (Phase 3 已完成)

## Phases

### Phase 1: Context & Scope Lock

- [x] 读取 `plant-model-ducklake` README，确认独立 crate 当前负责 DuckDB/DuckLake schema、write planning、attach flow、schema manifest 与 JSON/core smoke。
- [x] 读取 `plant-model-gen` DuckLake goal 文档，确认当前 goal 范围是 `ModelWriterBackend` 第三后端，不复用 `pe_transform_store::register_ducklake` stub。
- [x] 读取 `model_writer_ducklake.rs` / `options.rs` / `Cargo.toml` 相关片段，确认 `ModelWriterMode::DuckLake`、`duckdb` optional dependency 与 9 张 raw 表实现方向已经存在。
- **Status:** complete

### Phase 2: Implementation Gap Audit

- [x] 对 `src/fast_model/gen_model/model_writer_ducklake.rs` 做函数级审计：8 个 trait 生命周期方法均有实现面；`cleanup` 与 `boolean_bridge` 按范围 skipped，`reconcile_missing_neg_relations` 是保守 sentinel 行。
- [x] 对比 `plant-model-ducklake/src/schema.rs` 的 canonical schema 与 `model_writer_ducklake.rs::create_table_ddl()` 的 in-repo schema，确认当前存在字段名、主键、snapshot/run 语义与 payload_json 临时字段分叉。
- [x] 明确短期策略：本轮先修 in-repo DuckLake writer 的验收阻塞，不立即跨 crate 重构；跨 crate 复用单独列为后续 Phase。
- **Status:** complete

### Phase 3: Verification Surface

- [x] 启动 `plant-model-gen` DuckLake feature 编译预检：`cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`。
- [x] 复查 DuckDB feature：`duckdb-1.10502.0` 的 `default = []`，当前阻塞不是由 `default-features = false` 引起。
- [x] 阻塞解除：2026-05-17 重检 `D:` 已恢复 128.42GB，`target-ducklake-verify` 已被前轮失败链路清理；同命令重跑 `cargo check`，**EXIT=0**，`Finished dev profile in 1m 22s`，0 error / 110 warning（仅依赖库 dead_code）。
- [x] 预检 CLI 验证面：`model_writer_verify --mode ducklake --json` 只输出静态 contract evidence；真正打开 DuckLake 需加 `--exec`。
- [x] 预检 web_server 验证面：`POST /api/model/writer-verify {"mode":"ducklake"}` 当前只返回 `model_writer_contract_evidence()`，不会执行 DuckLake init / attach。
- [x] CLI 静态路径验证：`cargo run --bin model_writer_verify --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify -- --mode ducklake --json`。EXIT=0，输出 8 stages（7 implemented + 1 skipped: boolean_bridge）和 6 known_gap_tables。
- [x] CLI 执行路径验证：直接运行已 build 的 `target-ducklake-verify/debug/model_writer_verify.exe --mode ducklake --exec --json`。EXIT=0，elapsed_ms=599，`init: executed item_count=9`，6 个 known_gap stages 全部 skipped 且 reason 指向 phase1 trait gap；落盘确认 `metadata.ducklake` 3,084 KB + 9 张 raw 表目录就绪。
- [ ] (可选) 扩展 `/api/model/writer-verify` 支持安全 exec 模式；当前 HTTP 仅静态 contract evidence。本轮不强制，未引入到验收路径。
- [ ] (可选) feature gate 反例验证：未启用 `model-writer-ducklake` 时 `ducklake` mode 走 `model_writer_contract_evidence` static 路径；exec 路径已有 `anyhow::bail!` 守卫（见 `src/bin/model_writer_verify.rs:188`）。本轮不强制额外编译反例。
- **Status:** complete (核心三项 PASS，两项 optional 留待 Phase 5/6)

### Phase 4: Real Data Smoke & SQL Evidence

- [ ] 选择小样本 dbnum（优先沿用 goal 文档的 `1112`，若本机数据不可用则记录原因并请求替代样本）。
- [ ] 跑一次 Surreal baseline 与一次 DuckLake writer 生成，避免任何破坏性清理；只写 DuckLake 本地 output 路径。
- [ ] 为 9 张 raw 表输出行数、关键字段非空率、主键样本；Known Gap 表只在报告中列明，不纳入失败项。
- **Status:** pending

### Phase 5: Cross-Crate Convergence Plan

- [ ] 判断 `plant-model-ducklake` 的 `RawBatchWriter` / `PlannedWriteBatch` 是否能承载 `ModelWriterBackend` 阶段输出。
- [ ] 如可复用，设计一个小步迁移：先共享 schema manifest / DDL，再共享 writer planning，最后再替换 in-repo SQL adapter。
- [ ] 如不可复用，写明分界：`plant-model-ducklake` 继续作为 storage adapter 实验场，`plant-model-gen` 保留运行期后端实现。
- **Status:** pending

### Phase 6: Delivery & Archive

- [ ] 更新 `goals/ducklake-model-writer/progress.jsonl` 或当前 planning 文件，记录命令、输出摘要、artifact 路径和阻塞项。
- [ ] 输出最终中文结论：已通过项、未通过项、下一次应执行的第一条命令。
- [ ] 将本计划标记完成，历史 RUS-248 / pe_transform 内容继续保留在归档段。
- **Status:** pending

## Key Questions

1. 当前 `DuckLakeModelWriterBackend` 是否已经完成 Slice 2-4 的真实写入，还是仍有阶段只返回 skipped / placeholder？
2. `plant-model-ducklake` 的 schema manifest 与 `plant-model-gen` 当前 9 张 raw 表 DDL 是否能对齐，还是已经出现两套 canonical 定义？
3. 首轮真实数据 smoke 使用哪个 dbnum 和哪个输出目录，才能既可复现又不污染现有 pe_transform / SurrealDB 数据？
4. DuckLake extension 在本机 bundled DuckDB 下是否能稳定 `INSTALL ducklake; LOAD ducklake; ATTACH ...`？

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| 下一步先做 in-repo DuckLake writer 验收，不马上跨 crate 重构 | `plant-model-gen` 已有 `ModelWriterBackend` 接入面和 goal 验收路径，先把真实可运行性闭合，避免同时改运行链路与 crate 边界。 |
| `plant-model-ducklake` 作为对照和后续收敛目标 | 独立 crate 已有 storage-neutral write planning、schema manifest 与 smoke examples，可作为 schema/adapter 复用候选。 |
| 不运行 `cargo test` | 遵守仓库规则；验证使用 `cargo check`、CLI JSON、web_server POST 和 DuckDB SQL。 |
| 不触碰 `pe_transform_store::register_ducklake` | pe_transform DuckLake stub 与 ModelWriter DuckLake 后端是不同路径，本轮不混合。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| 用户级 `planning-with-files` 的 `session-catchup.py` 路径不存在 | 1 | 记录为非阻塞；已读取项目内 planning skill、现有 planning 文件和 DuckLake 相关上下文后继续制定计划。 |
| `libduckdb-sys v1.10502.0` bundled 编译失败 | 2 | 首次失败只有 custom build exit code 1；第二次用 `-j 1` 暴露 `LINK : fatal error LNK1114 ... 错误代码 112`，结合 `D:` 复查仅剩约 0.03GB，先按磁盘空间不足处理。 |
| `cargo check` 重跑后 PowerShell pipeline 在 `Checking duckdb v1.10502.0` 处异常断开（exit_code unknown） | 1 | 改用 `Out-File` 替代 `Tee-Object` 重跑，EXIT=0，`Finished in 1m 22s`；推断首次断开是 PowerShell pipe 缓冲问题，不是 cargo 真实失败（`libduckdb-sys` bundled C++ 已成功，否则不会到 `Checking duckdb`）。 |

## Notes

- 验证时必须避免 Rust test target；优先使用 CLI、HTTP POST、DuckDB SQL。
- 启动 web_server 时留意历史锁定问题，可用独立 target dir 与非 3100 端口规避。
- DuckLake output 路径应避免写入 `output/AvevaMarineSample/pe_transform/`。
- Phase 2 审计发现源码顶部 Slice 1 注释仍写“后续阶段 bail / placeholder”，但文件下方已有 Slice 2/3/4 写入实现；后续应顺手修正陈旧注释，避免误导。
- Phase 2 审计发现 `plant-model-ducklake` 的 schema 更接近 append-only canonical raw 设计（`snapshot_id/run_id/written_at/is_deleted` + primary key manifest），而 in-repo writer 当前偏运行期临时 DDL，应优先用 schema diff 指导 parity SQL，而不是立即替换 DDL。
- Phase 3 当前阻塞在 DuckDB bundled C++ 编译层，不是 Rust 业务代码层；目前已明确首要问题是 `D:` 空间不足导致 `libduckdb.a` 归档失败，下一步先释放空间或把 `--target-dir` 指向有足够空间的盘，再继续 CLI evidence / smoke。
- 本计划是当前 active plan；下面内容为历史归档。

## Archived Previous Plan

# RUS-248 批注后驳回流转修复计划

## Goal

修复 PMS 外部校审流程中“批注后无法流转”的问题：`pms.workflow_pre_action` 校验通过后，`pms.workflow_changed` 的实际落库必须统一走 `/api/review/workflow/sync` external mutation，而不是前端内部 `/api/review/tasks/{id}/return|approve` 路径。

## Current Phase

Complete

## Phases

### Phase 1: Plan & Contract

- [x] 梳理现有被驳回后的处理链路。
- [x] 明确修复边界：仅改 PMS iframe/postMessage 外部流程路径，保留内部按钮路径。
- [x] 创建本轮 planning-with-files 计划。
- **Status:** complete

### Phase 2: Frontend External Sync Mutation

- [x] 在 `plant3d-web/src/api/reviewApi.ts` 增加 workflow sync mutation API。
- [x] 扩展 `pms.workflow_changed` 消息类型支持 `nextStep`。
- [x] 将 `useReviewStore.applyExternalWorkflowChange()` 改为调用 `/api/review/workflow/sync`。
- [x] 为旧 PMS 消息增加 `nextStep` fallback 推导。
- **Status:** complete

### Phase 3: Simulator & Contract Alignment

- [x] 更新 PMS simulator 消息构造与类型，优先传递 `nextStep`。
- [x] 检查/补充 postMessage ack/synced 返回字段，便于 PMS 侧展示失败原因。
- **Status:** complete

### Phase 4: Backend History Consistency

- [x] 检查 `workflow_sync` 写入 history 字段是否满足 UI 查询和排查。
- [x] 如有必要，补齐 `form_id`、`target_node`、`source` 字段，保持旧字段兼容。
- **Status:** complete

### Phase 5: Verification

- [x] 使用 CLI/真实接口方式验证，不运行测试套件。
- [x] 验证 JH return 后任务变为 `sj/draft` 且保存 `return_reason`。
- [x] 验证 SJ 处理后 active 到 `jd/submitted` 且清空 `return_reason`。
- [x] 记录具体命令、payload 和响应结果。
- **Status:** complete

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| PMS 外部流程统一走 `/api/review/workflow/sync` | 后端 external sync 已承载 actor/next_step 契约，可绕开内部 JWT owner 命名空间问题。 |
| 内部按钮路径暂不改 | 避免影响非 PMS 内部校审页面和既有权限模型。 |
| `nextStep` 优先由 PMS 显式传入 | 外部流程平台是下一处理人事实源。 |
| 前端保留 fallback 推导 | 兼容当前 simulator/旧 PMS 消息，降低联调切换风险。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `session-catchup.py` 用户级/项目级路径均不存在 | 1 | 记录为非阻塞；已读取现有 planning 文件并前置新 active plan。 |
| 默认 target 的 `web_server.exe` 被旧 3100 服务占用 | 1 | 改用 `target-rus248` 独立 target-dir，并用 `WEB_SERVER_PORT=3199` 启动当前代码。 |
| 独立 target 首次全量编译缺 NASM | 1 | 将 `C:\Program Files\NASM` 临时加入 PATH 后重试成功。 |

## Verification Log

| Step | Command / Payload | Result |
|------|-------------------|--------|
| Static frontend | `npm run type-check` in `plant3d-web` | PASS |
| Static backend | `cargo check --bin web_server --features web_server` | PASS（仅既有依赖 warning） |
| Start current backend | `WEB_SERVER_PORT=3199 cargo run --target-dir target-rus248 --bin web_server --features web_server -- --config db_options/DbOption-cursor` | PASS，`/api/version` 当前代码服务可用 |
| Create task | `POST /api/review/tasks` with `formId=RUS248-VERIFY-20260514110621`, `SJ`, `checker=JH` | PASS，task `task-a19fe2cc-bd6e-4b6e-9f7f-2288c0a7f6be`, `sj/draft` |
| Active to JD | `POST /api/review/workflow/sync` action `active`, `next_step={assignee_id:JH,roles:jd}`, source `rus248-cli-verify-active` | PASS，`current_node=jd`, `task_status=submitted`, `return_reason=null` |
| Rejected annotation | `POST /api/review/annotation-states/apply` action `reject` by `JH` | PASS，annotation `open/rejected` round 1 |
| Return to SJ | `POST /api/review/workflow/sync` action `return`, `next_step={assignee_id:SJ,roles:sj}`, source `rus248-cli-verify-return` | PASS，`current_node=sj`, `task_status=draft`, `return_reason=RUS-248 verify return to SJ` |
| Fixed annotation | `POST /api/review/annotation-states/apply` action `fixed` by `SJ` | PASS，annotation `fixed/pending` round 2 |
| Reactive to JD | `POST /api/review/workflow/sync` action `active`, `next_step={assignee_id:JH,roles:jd}`, source `rus248-cli-verify-reactive` | PASS，`current_node=jd`, `task_status=submitted`, `return_reason=null` |
| History fields | `surreal sql ... SELECT task_id, form_id, node, target_node, action, operator_id, actor_id, actor_role, source, comment ...` | PASS，3 条 history 均含 `form_id/target_node/source/actor_*` |

## Archived Previous Plan

# pe_transform 后端重构计划

## Goal

在 `feat/pe-transform-backends` worktree 中，为 `pe_transform` 增加 feature-gated 的读写后端抽象，支持 SurrealDB、Parquet、DuckLake 与对比模式，并保持默认生成路径行为不变。

## Current Phase

Phase 13

## Phases

### Phase 1: Requirements & Discovery

- [x] 安装 `planning-with-files` 到 Cursor/Codex。
- [x] 创建独立 worktree：`.worktrees/pe-transform-backends`。
- [x] 确认现有 `pe_transform` 刷新、查询和 feature 校验入口。
- **Status:** complete

### Phase 2: Feature & Runtime Surface

- [x] 在 `Cargo.toml` 增加 `transform-store-parquet`、`transform-store-ducklake`、`transform-store-compare`。
- [x] 在 `DbOptionExt`/CLI 增加 `transform_write_backend`、`transform_read_backend`、`transform_compare_backend` 及输出路径配置。
- [x] 复用 `validate_model_writer_features` 的模式新增 transform backend feature 校验。
- **Status:** complete

### Phase 3: Backend Abstraction

- [x] 新增 `PeTransformSink` / `PeTransformSource` 抽象。
- [x] 将现有 SurrealDB 写入封装为默认 sink/source，不改变当前默认行为。
- [x] 支持 `dual` sink，用于 SurrealDB + Parquet 双写对比。
- **Status:** complete

### Phase 4: Parquet Backend

- [x] 定义 `pe_transform.parquet` schema，覆盖 `refno/dbnum/local/world/hash/updated_at`。
- [x] 在 refresh batch flush 后按配置写 Parquet。
- [x] 支持从 Parquet 按 refno 加载，生成阶段 cache miss 可按配置读取并 prime 到 `transform_cache`。
- **Status:** complete

### Phase 5: DuckLake Backend

- [x] 使用 DuckLake 管理 Parquet 元数据，优先走"写 Parquet + `ducklake_add_data_files` 注册"的低耦合路径。
- [x] 默认按 `project_name, dbnum` 分区，避免过细 refno 分区。
- [ ] 提供 DuckLake 原生查询入口用于加载与版本对比；当前 ducklake 读路径先复用 Parquet source。
- **Status:** in_progress

### Phase 6: Compare & Benchmark

- [x] 增加 CLI 对比模式，读取同一批 refno/dbnum 的两个 backend。
- [x] 比较 local/world 矩阵误差、缺失数量、加载耗时。
- [x] 输出结构化摘要，便于比较 SurrealDB、Parquet、DuckLake 路径。
- [x] 固定首轮基准为刷新 `dbnum=7997` 的 transform。
- [x] 对比前清理历史 `pe_transform` 数据，避免旧 transform 污染 backend 对比。
- **Status:** complete

### Phase 7: Verification & Handoff

- [x] 按项目规则优先使用 CLI/真实接口验证，不新增 test。
- [x] 验证流程：清理 dbnum=7997 历史 -> 刷新 -> dual 写入 -> SurrealDB/Parquet 对比。
- [x] 在 Rust 工具链可用时执行 `cargo check` 和 `cargo build`。
- [x] 记录验证命令、输入 dbnum/refno、输出耗时和剩余风险。
- **Status:** complete

## Key Questions

1. DuckLake 首版是否只做注册和查询，还是需要 Rust 侧直接依赖 DuckDB/DuckLake 写入？
2. Parquet schema 是否采用完全展开矩阵列，还是保留 hash + 单独 transform 表做规范化？
3. 对比基线使用哪些 dbnum/root_refno，是否固定 `DbOption-cli.toml` 当前样本？（已定：首轮使用 `dbnum=7997`）

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| 默认行为保持 SurrealDB | 避免影响现有生成、Web API 和 `pe_transform` 依赖查询。 |
| feature 控制能力、CLI/配置控制本次 backend | 保持编译依赖可控，同时支持同一二进制做多种实验。 |
| 生成热路径统一 prime 到 `transform_cache` | 对比加载/预热成本，避免几何生成逻辑分叉。 |
| DuckLake 首选"外部 Parquet + add_data_files" | 与 `ducklake` 示例/测试一致，降低 Rust 侧直接集成风险。 |
| 首轮对比固定刷新 `dbnum=7997` | 用户指定该 dbnum，便于控制样本和复现实验。 |
| 对比前必须清理历史 `pe_transform` | 避免 SurrealDB 中旧 transform 与新 Parquet/DuckLake 数据混用，导致误判。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `cargo` not recognized | 1 | 当前 PowerShell PATH 无 Rust 工具链，已用 `ReadLints` 和 `git diff --check` 做静态检查；需在 cargo 可用环境补跑 `cargo check`。 |
| git dependency update stalled | 1 | 使用 `D:/Rust/.cargo/bin` 后 `cargo check` 卡在多个 git 依赖；已为 indextree/miniacd/rvm-rs/surrealdb/calamine/cavalier_contours/id_tree 增加本地 patch。 |
| `rust-ploop-processor` unavailable | 1 | `rs-core` 依赖 `https://github.com/happyrust/rust-ploop-processor`，本机未找到本地仓库，在线更新长时间无输出；需提供本地仓库或恢复网络。 |

## 下一步详细开发方案

### Phase 8-9: 恢复验证环境 & 编译收敛

- [x] Cargo/Rust 可用（`D:/Rust/.cargo/bin`）
- [x] SurrealDB 可连接（port 8020）
- [x] `cargo check` 通过（修复 5 个编译问题）
- [x] `cargo build` 通过
- **Status:** complete

### Phase 10: SurrealDB vs Parquet 首轮对比

- [x] 执行清理 + 刷新 + 双写 + 对比（724s 完成，143222/176390 节点处理）
- [x] 记录输出：SurrealDB loaded=175337, Parquet loaded=143222, Parquet missing=32115, mismatched=58930, max_delta=0.000854, Parquet elapsed=1711ms, SurrealDB elapsed=16283ms
- [x] mismatch 分析：max_delta=0.000854 为 float 序列化精度差异，工程可接受
- [x] missing 分析：32115 = SurrealDB 历史数据 - 本次刷新数据，非 bug
- **Status:** complete

### Phase 11: Profile 耗时热点

- [x] 在 `pe_transform_store.rs` 添加 `WriteTimings` 结构，区分 SurrealDB/Parquet 写入耗时
- [x] 在 `pe_transform_refresh.rs` 添加 `RefreshProfile`，累计各阶段耗时并输出摘要
- [x] 定位主要瓶颈：Parquet 写入 39.5%（O(n²) read-merge-write），计算 37.1%，SurrealDB 写入 23.4%
- [x] 读取对比已在 compare 阶段有计时：Parquet 1,698ms vs SurrealDB ~14,900ms
- **Status:** complete

### Phase 12: DuckLake 注册验证

- [x] 检查 `register_ducklake` 实现：空 stub `Ok(())`
- [x] 检查 DuckDB CLI：不在 PATH 中
- **Status:** blocked（`register_ducklake` 未实现 + DuckDB CLI 不可用；首版验收不强制）

### Phase 13: 输出对比表与交付

- [x] 在 `progress.md` 记录真实命令、环境版本、输出摘要
- [x] 在 `findings.md` 记录结论性发现
- [x] 生成最终对比表（见下方）
- [x] 标记各 Phase 完成状态
- **Status:** complete

## 最终对比表

### 写入性能（dbnum=7997, 143,222 节点, dual 模式）

| Backend | Write Time (ms) | 占比 | Notes |
|---------|----------------|------|-------|
| 计算 transform | 230,888 | 37.1% | BFS + 逐节点 SurrealDB 查询 |
| SurrealDB 写入 | 145,763 | 23.4% | 批量 INSERT |
| Parquet 写入 | 245,339 | 39.5% | O(n²) read-merge-dedup-write，可优化 |
| **总刷新耗时** | **621,990** | | |

### 读取性能（compare 阶段）

| Backend | Read Time (ms) | Loaded | Missing | Mismatched | Max Delta |
|---------|---------------|--------|---------|------------|-----------|
| SurrealDB | 14,845 | 175,337 | 1,053 | 0 | 0.000000 |
| Parquet | 1,698 | 143,222 | 32,115 | 58,930 | 0.000854 |

### 结论

- **Parquet 读取约 8.8x 快于 SurrealDB**，验证了 Parquet 作为 transform 预热数据源的可行性
- **Parquet 写入当前实现需优化**（O(n²)），优化后预期可降至 <5s
- **Float 精度差异可接受**（max_delta=0.000854 < 0.001mm）
- **DuckLake 首版受限**：注册逻辑未实现 + CLI 缺失，保持后续增强

### 验收标准达成情况

- ✅ `cargo check` 通过
- ✅ `dbnum=7997` 对比前清理历史 `pe_transform`
- ✅ SurrealDB 与 Parquet 对比输出包含 loaded/missing/mismatched/max_delta/elapsed_ms
- ✅ Parquet 输出路径 `output/AvevaMarineSample/pe_transform/pe_transform.parquet`
- ❌ DuckLake CLI 不可用，注册脚本未实现（首版不强制）
- ✅ 验证结果写入 `progress.md` 和 `findings.md`

---

# Viewer 独立站点 URL 与 Nginx 入口计划

## Goal

把部署站点里的 Viewer 入口从管理端调试包装 URL：

`/viewer/?backend=...&output_project=...&show_dbnum=...`

改为客户可直接访问的独立 `plant3d-web` 根站点 URL：

`http://<viewer-host>/?output_project=AvevaMarineSample&show_dbnum=7997`

其中后端访问由同源 Nginx 代理 `/api/`、`/files/`、`/ws/` 完成，URL 不暴露 `backend` / `backendPort`。

## Current Phase

Phase V4: 运行态验证与部署闭环；新增 Phase V6 覆盖 Nginx 自动配置与自动启动。

## Phases

### Phase V1: URL Contract 决策

- [x] 明确客户 URL 只保留业务参数 `output_project` / `show_dbnum`。
- [x] 明确 `backend` / `backendPort` 属于管理端调试参数，不进入客户 URL。
- [x] 明确 `plant3d-web` 作为独立根站点运行，不再依赖部署路由 `/viewer/`。
- [x] 明确一个 Viewer Base URL 绑定一个 web_server 后端；多站点通过域名、端口或 Nginx vhost 隔离。
- **Status:** complete

### Phase V2: Viewer Base URL 解析优先级

- [x] 最高优先级：`AIOS_VIEWER_BASE_URL`，支持完整 URL（scheme / host / port / domain）。
- [x] 站点级优先级：复用现有 `public_base_url` / `public_entry_url`，不新增 `viewer_public_base_url` 字段。
- [x] 默认值：自动探测机器本机 IPv4，生成 `http://<local-ip>`。
- [x] 兜底值：本机受管 Viewer 端口 `http://127.0.0.1:<viewer_port>`，只用于开发/探测失败。
- **Status:** complete

### Phase V3: Code Changes

- [x] 后端受管 Viewer URL 改为独立根站点 URL，只拼 `output_project` / `show_dbnum`。
- [x] 管理端 `buildViewerUrl()` 改为独立 URL fallback，移除 `backend` / `backendPort` / `data_source` 拼接。
- [x] 抽取共享 `get_local_ip_via_udp()`，避免 IP 探测逻辑重复。
- [x] `/api/admin/app-config` 未配置 env 时默认返回 `http://<local-ip>`。
- [x] 新增 `shells/deploy/nginx-plant3d-web.conf.example`，描述根站点 + 同源代理契约。
- **Status:** complete

### Phase V4: Verification & Runtime Smoke

- [x] 静态验证：`ReadLints` 无诊断。
- [x] 静态验证：`git diff --check` 无空白错误。
- [x] 启动/重启管理端 web_server，确认 `/api/admin/app-config` 返回 `viewer_base_url=http://<local-ip>` 或配置值。
- [x] 读取历史部署站点，确认旧 `backend=` viewer_url 在 API 响应中归一化为 `http://<host>/?output_project=...&show_dbnum=...`。
- [x] 启动一个部署站点，确认新写入的 `viewer_url` 形态为 `http://<local-ip>:<viewer_port>/?output_project=...&show_dbnum=...`，且 HTTP 可达。
- [x] 硬化管理端前端 fallback：当后端 `viewer_base_url` 只是默认本机 IP 时，优先结合站点 `viewer_port` 生成 `http://<local-ip>:<viewer_port>/...`，避免无 Nginx 的 Windows 本机误跳 80 端口。
- [ ] 按 Nginx 示例部署 `plant3d-web` 根站点，验证 `/`、`/api/health`、`/files/output/...` 均同源可达。
- [ ] 用真实已导出的模型包打开客户 URL，确认模型能加载。
- **Status:** in_progress

### Phase V5: Hardening & Documentation

- [x] 在部署指南中补充 `AIOS_VIEWER_BASE_URL`、`public_base_url`、默认本机 IP、`viewer_port` fallback 的关系。
- [x] 明确多站点并行时必须配置不同 vhost / port / domain。
- [ ] 如果远端部署流程也上传 Viewer，确认远端 `public_base_url` 与 Viewer 根站点一致。
- [x] 补充 Nginx 静态入口 + 同源反代说明，并记录受管 Viewer `AIOS_VIEWER_BIND_HOST` fallback。
- **Status:** pending

### Phase V6: Nginx 自动配置与自动启动

- [x] 在 Windows 受管 Viewer 启动流程中增加可选 Nginx 自动配置入口。
- [x] Windows 未检测到 Nginx 时不失败，保留受管 `vite preview` fallback，保证当前本机自动部署可用。
- [x] Windows 可选 Nginx 配置输入包括：`AIOS_NGINX_BIN`、可选 `AIOS_NGINX_ROOT` / `AIOS_NGINX_CONF_DIR`、plant3d-web `dist` 路径、目标 web_server 地址、站点 id、可选 server_name。
- [x] Windows 可选 Nginx 模式使用站点 runtime 下独立 prefix（未设置 `AIOS_NGINX_ROOT` 时），自动生成 `conf/nginx.conf` 和 `conf/conf.d/plant3d-web-<site_id>.conf`。
- [x] 增加 OS 分支：Linux 使用 system nginx；Windows 默认不强制 Nginx，优先受管 `vite preview`，可选支持用户指定 `nginx.exe`。
- [x] Linux 远端部署时写入 `/etc/nginx/conf.d/plant3d-web-<site_id>.conf` 或用户指定目录。
- [x] Windows 可选 Nginx 模式写入 `<prefix>/conf/conf.d/plant3d-web-<site_id>.conf`，路径统一转换为 Nginx 可读的 `/` 格式。
- [x] Windows 可选 Nginx 模式自动执行 `nginx.exe -p <prefix> -t` 做配置验证；失败时阻止 reload/start。
- [x] Linux 自动执行 `nginx -t` 做配置验证；失败时保留错误日志并阻止 reload。
- [x] 自动执行 `systemctl reload nginx`，未运行时尝试 `systemctl enable --now nginx`，再使用 `nginx -s reload` fallback。
- [x] Windows 可选 Nginx 模式通过 `nginx.exe -p <prefix> -s reload` reload；未运行时以受管进程启动 `nginx.exe -p <prefix>`。
- [ ] 自动检查 `/`、`/api/health`、`/files/` 关键路径；把结果写入部署验收报告。
- [x] 明确权限边界：无权限时给出可复制的配置路径和命令，不静默失败，并继续受管 Viewer fallback。
- [x] Windows 本机开发不强制安装 Nginx，只保留受管 `vite preview` fallback；若用户配置了 `AIOS_NGINX_BIN` / `AIOS_NGINX_ROOT` 再启用 Windows Nginx 自动化。
- **Status:** in_progress

#### Phase V6 OS Matrix

| OS / Mode | Default Behavior | Auto Config Path | Validate | Start / Reload | Fallback |
|-----------|------------------|------------------|----------|----------------|----------|
| Linux remote/root | Nginx as customer entry | `/etc/nginx/conf.d/plant3d-web-<site_id>.conf` | `nginx -t` | `systemctl enable --now nginx` / `systemctl reload nginx` | output config + commands if no privilege |
| Linux without systemd | Nginx as customer entry | configurable conf dir | `nginx -t` | `nginx -s reload` | output config + commands |
| Windows local default | no Nginx requirement | none | n/a | managed `vite preview` | `http://127.0.0.1:<viewer_port>` |
| Windows optional Nginx | user-provided Nginx | `<nginx_root>/conf/conf.d/plant3d-web-<site_id>.conf` | `nginx.exe -t -p <nginx_root>` | `nginx.exe -s reload` or start managed process | managed `vite preview` |

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| URL 只保留 `output_project` / `show_dbnum` | 客户 URL 不应暴露管理端内部后端地址。 |
| 配置完整 Viewer Base URL 而不是只配置 IP | scheme、端口、域名、HTTPS 都不是单独 IP 能表达的。 |
| 未配置时默认 `http://<local-ip>` | 满足“默认使用机器本机 IP”的产品预期。 |
| 复用 `public_base_url` | 推荐 Nginx 模式下 Viewer、API、files 是同一个 origin，避免新增重复字段。 |
| Nginx 负责 `/api` / `/files` / `/ws` 同源代理 | plant3d-web 可以不依赖 `backend` query 参数启动。 |
| Nginx 自动化必须先 `nginx -t` 再 reload | 避免生成坏配置导致已有服务不可用。 |
| 无 root/sudo 时降级为输出配置与命令 | Nginx 自动启动是部署便利能力，不应掩盖权限失败。 |
| Windows 默认不强制 Nginx | Windows 开发机常无系统级 Nginx；只有显式配置 `AIOS_NGINX_BIN` / `AIOS_NGINX_ROOT` 才启用自动化。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| 用户级 `planning-with-files/scripts/session-catchup.py` 不存在 | 按 skill 首步执行 session catchup | 记录为非阻塞；继续读取现有 `task_plan.md` / `findings.md` / `progress.md` 并追加本计划。 |
