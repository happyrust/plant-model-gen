# Release 包 sidecar job 终态竞态修复发现

## 2026-06-08 Discovery

- [任务] 用户要求使用 `planning-with-files` 制定开发计划，目标是解决 release 包站点部署中 `aios-database sidecar 解析作业失败: error sending request for url http://127.0.0.1:<port>/jobs/<job_id>`。
- [环境] 当前运行目录为 `D:\work\plant-code\plant-model-gen\dist\package\Plant3D-AIOS-win-x64\release`，不是源码目录直接运行。
- [证据] `netstat` 显示主控 `3100` 和主控 SurrealDB `8020` 正在运行；截图中的 sidecar 临时端口 `53081` 已无监听。
- [证据] `runtime/admin_sites/quicktest-250160-8080/logs/parse.log` 显示 parse job `f29566ec-dc7b-47da-a223-9e5deaafbb21` 已完成：
  - `sidecar 解析 job ... status=submitted`
  - `sidecar 解析 job ... status=running`
  - `sidecar 解析 event job_done: status=succeeded, exit_code=0`
- [结论] 这不是 dbfile 路径错误、SurrealDB 连接失败、文件头读取失败或解析 CLI 失败；解析实际成功，UI 的失败发生在主控取 job 终态阶段。
- [代码证据] `src/web_server/parse_sidecar_client.rs::run_cli_job_with_status()` 在 submit 后每 500ms 通过 `GET /jobs/{job_id}` 轮询终态；若该 HTTP 请求失败，会直接返回 `SidecarProxyError`。
- [代码证据] `src/web_server/parse_sidecar_client.rs::spawn_sidecar()` 对 job sidecar 传入 `--shutdown-after-job --shutdown-delay-ms 1000`。
- [代码证据] `src/parse_sidecar.rs::schedule_shutdown_after_job()` 在 job 完成后按 `shutdown_delay_ms` 发送 shutdown 信号，关闭 sidecar HTTP 服务。
- [根因] Windows release 包现场存在竞态：sidecar 已通过 websocket event 发出 `job_done/succeeded`，但主控下一次 `/jobs/{id}` HTTP 轮询发生时 sidecar 已自动退出，导致 `error sending request` 被误判为解析失败。
- [短期修复] 将 job sidecar shutdown delay 从 `1000ms` 提高到 `10000ms`，并支持 `ADMIN_SIDECAR_JOB_SHUTDOWN_DELAY_MS` 环境变量覆盖。
- [修复] `src/web_server/parse_sidecar_client.rs` 已新增 `DEFAULT_JOB_SIDECAR_SHUTDOWN_DELAY_MS = 10_000` 与 `JOB_SIDECAR_SHUTDOWN_DELAY_ENV = "ADMIN_SIDECAR_JOB_SHUTDOWN_DELAY_MS"`；`job_sidecar_shutdown_delay_ms()` 会读取环境变量中的正整数毫秒值，否则回退默认 10 秒。
- [修复] 只有 `key.starts_with("job:")` 的 sidecar 使用延迟关闭配置；preview/scan/resolve/db-index 常驻 sidecar 行为不变。
- [长期修复] 让 websocket terminal event (`job_done/job_failed/job_cancelled`) 参与最终判定；当 HTTP `/jobs/{id}` 失败但已经收到 terminal event 时，以 terminal event 为准。
- [修复] `src/web_server/managed_project_sites.rs::run_sidecar_cli_job_with_site_events()` 已记录 submitted job_id 与 websocket terminal status；如果 `run_cli_job_with_status()` 因 HTTP 轮询失败返回错误，会短暂等待 terminal event，若已有 `succeeded/failed/cancelled` 则转换为 `RunCliJobResponse`。
- [修复] terminal event 兜底会写入日志：`HTTP 终态轮询失败，但已收到 websocket 终态事件；按 event status=... 继续`，便于现场确认是否命中兜底路径。
- [语义] 如果 HTTP 轮询失败且没有收到 websocket terminal event，仍保留原始 `SidecarProxyError`，避免掩盖真正的 sidecar 通信失败。
- [部署注意] 源码修复提交后不会自动影响 `dist/package/.../release` 中的 exe；必须重建 release 包或替换 `bin/web_server.exe` / 必要的 `bin/aios-database.exe`。
- [验证] 已使用 `cargo build --bin web_server --features web_server --release --target-dir target-sidecar-fix-release` 构建新 `web_server.exe`，SHA256 `96934A1F697E4A4863ABE081DEEDC45FA765325C46AC45474BF837163C2DFFBD`。
- [验证] 已备份并替换 release 包内 `bin/web_server.exe`；旧文件备份为 `bin/web_server.exe.bak-20260608-180944`。
- [验证] 重启 release 包后 `/api/version` 返回 commit `cd50f3c865ce778d2206f779e1eb328124f05482`。
- [验证] 新建 `sidecarracefixonly-8080` 并执行完整部署，最终状态 `Running / Parsed`，未再出现 `/jobs/{id}` error sending request 误失败。
- [验证] `parse.log` 与 `generate.log` 都显示 websocket event 和 HTTP 轮询终态均成功记录 `status=succeeded, exit_code=0`。
- [验证] 部署校验 `blocking=0 warning=1`，唯一 warning 为 synthetic root `api_e3d_subtree_refnos`，符合此前诊断策略。

## Archived Previous Findings

# Release 包 Nginx 客户入口开发发现

## 2026-06-05 Discovery

- [任务] 用户要求使用 `planning-with-files` 制定“release 部署包能否正确配置 Nginx，以及如何修复”的开发计划；本轮将 release 包 Nginx 客户入口修复计划前置为 active plan。
- [已确认] 当前源码 `src/web_server/managed_project_sites.rs` 已有 Windows/Linux Nginx 自动配置分支：生成 `plant3d-web-<site_id>.conf`、执行 `nginx -t`、reload/start，并在缺少 Nginx 时 fallback。
- [已确认] 当前已打出的 release 验证包 `BUILD_INFO.json` 仍是 2026-05-27 构建，`viewerUrl` 为 `http://127.0.0.1:3100/viewer/`，不是客户根路径 URL。
- [已确认] 当前包内未发现 `nginx*` 文件，启动脚本也没有设置 `AIOS_NGINX_BIN`、`AIOS_NGINX_ROOT`、`AIOS_VIEWER_STATIC_ROOT`、`AIOS_VIEWER_BASE_URL`。
- [不匹配] 打包脚本当前用 `VITE_BASE_PATH=/viewer/` 构建前端，并把 dist 内容复制到包内 `viewer/`；源码 Nginx 配置当前默认使用 `viewer_dir/dist` 作为 root，更像面向前端源码项目而不是 release 包静态产物。
- [不匹配] 包内 `viewer/index.html` 引用 `/viewer/assets/...`；如果 Nginx 直接把它作为根站点 `/` 的 index，会导致资产路径与 `root` 映射不一致，除非额外 alias/rewrite。
- [风险] Windows Nginx 分支当前调用 `render_plant3d_web_nginx_conf(site, viewer_dir, 80)`，硬编码 `listen 80`；如果 `AIOS_VIEWER_BASE_URL` 配置了端口（如 `http://host:8080`），URL 与 Nginx 监听端口会不一致。
- [风险] 若启动脚本提前设置 `AIOS_VIEWER_BASE_URL=http://<local-ip>` 但 Nginx 没有成功启动，管理端会生成指向 80 的 URL，造成“看起来配置成功，实际不可达”。
- [grill-me 决策] Windows release 包推荐“开箱即用”：可选捆绑 Nginx，启动脚本自动配置环境变量；Linux 推荐接管系统 Nginx，不捆绑。
- [grill-me 决策] 推荐打两份前端产物：`viewer/` 继续服务 `/viewer/` fallback，`viewer-root/` 服务 Nginx 客户根入口 `/`。
- [grill-me 决策] 一个 `host:port` 只绑定一个客户站点；多站点必须使用不同域名、IP 或端口，避免重新引入 `backend=` 参数。
- [grill-me 决策] Nginx 的 `/admin/` 推荐代理到主控 `web_server` 端口，`/api/`、`/files/`、`/ws/` 代理到当前客户站点 `site.web_port`。
- [计划] 下一步应优先修改打包布局和运行时静态 root 识别，而不是直接调 Nginx 配置模板；否则 release 包仍缺少可被 Nginx 正确服务的 root-base 前端产物。
- [修复] `build-windows-bundle.ps1` 现在会构建两份 `plant3d-web` 静态产物：`viewer/` 使用 `VITE_BASE_PATH=/viewer/`，`viewer-root/` 使用 `VITE_BASE_PATH=/`；`BUILD_INFO.json` 和安装 README 已区分 fallback URL 与客户入口模板。
- [修复] Windows 包支持可选复制 `tools/nginx/windows/nginx.exe` 到 `bin/nginx/nginx.exe`；缺失时仅 warning，不阻断打包。
- [修复] `managed_project_sites.rs` 的 Nginx 模板现在优先使用 `AIOS_VIEWER_STATIC_ROOT` 或包内 `viewer-root/`，Windows listen 端口跟随 `AIOS_VIEWER_BASE_URL`，并把 `/admin/`、`/api/admin/` 代理到主控 `web_server` 端口。
- [修复] Windows Nginx 配置/校验/reload/start 失败默认写日志并继续 `vite preview` fallback；只有 `AIOS_REQUIRE_NGINX=1` 时才返回 fatal。
- [修复] `start-plant3d.ps1/.bat` 和 `install-service.ps1/.bat` 已增加 `EnableNginx`、`ViewerHost`、`ViewerPort`、`RequireNginx` 参数传递。
- [修复] 当前工作树里 `ManagedProjectSite` / `CreateManagedSiteRequest` 新增生成范围字段后，`managed_project_sites.rs` 的既有构造点已补齐 `generate_db_nums` / `generate_db_files` 默认值，恢复 `web_server` 类型检查。
- [验证] 已构建 debug 包布局 smoke：`runtime/codex-validation/nginx-package-smoke-20260605-165240/Plant3D-AIOS-win-x64/debug`。包内 `viewer/index.html` 引用 `/viewer/assets/...`，`viewer-root/index.html` 引用 `/assets/...`，`BUILD_INFO.json` 标记 `nginxStaticRoot=viewer-root`、`nginxBundled=false`。
- [验证] 无 Nginx 场景下运行包内 `start-plant3d.ps1 -Port 3196 -NoBrowser -NoPortFallback`：脚本提示缺少 `nginx.exe` 并降级到 `/viewer/` fallback；`GET /api/version` 和 `GET /viewer/` 均返回 200，临时进程已停止。
- [修复] 已从 Nginx 官方下载 stable Windows 包 `https://nginx.org/download/nginx-1.30.2.zip`，提取 `nginx.exe` 到 `tools/nginx/windows/nginx.exe`；SHA256 为 `f2ffb8462dc348a333f40557a9e0d9cd554a2d4501eb3a265da2c429ec99a527`。同一二进制已复制到 debug smoke 包的 `bin/nginx/nginx.exe`，便于后续 Nginx 场景验证。
- [验证] 已构建 release 包布局 smoke：`runtime/codex-validation/nginx-release-with-bundled-20260605-170329/Plant3D-AIOS-win-x64/release`。`BUILD_INFO.json` 标记 `nginxBundled=true`、`nginxExe=bin/nginx/nginx.exe`、`nginxStaticRoot=viewer-root`；包内 Nginx SHA256 与 `tools/nginx/windows/nginx.exe` 一致。
- [验证] bundled Nginx 启动检测 smoke：运行 `start-plant3d.ps1 -Port 3298 -EnableNginx on -RequireNginx -ViewerHost 127.0.0.1 -ViewerPort 3297`，脚本识别包内 Nginx、设置 `AIOS_VIEWER_STATIC_ROOT=viewer-root`、后端 `/api/version` 返回 200；`/api/admin/app-config` 未登录返回 401，符合管理 API 鉴权预期。
- [修复] 真实 release 包只有 `viewer-root/` 静态产物，没有 `plant3d-web` 源码项目；原 `spawn_viewer_process` 在找不到 `plant3d-web` 时会提前跳过 Viewer/Nginx 配置。已改为：若存在 `viewer-root/index.html` 且 Nginx 配置成功，则直接用 release 静态根启动/刷新 Nginx，并返回客户 Viewer URL，不再要求 npm/vite 项目目录。
- [验证] 已构建包含该修复的 debug smoke 包：`runtime/codex-validation/nginx-static-root-fix-20260605-171011/Plant3D-AIOS-win-x64/debug`。通过 admin API 创建最小站点并标记 Parsed 后启动，站点进入 Running，Nginx 生成 `runtime/nginx/conf/conf.d/plant3d-web-nginx-static-smoke-1780650910-3396.conf`；HTTP 验证 `GET /`、`GET /assets/<bundle>.js`、`GET /api/status`、`GET /admin/` 均返回 200，客户 URL 为 `http://127.0.0.1:3397/?output_project=NginxStaticSmoke`，无 `backend=` 参数。`/files/` location 已在生成配置中覆盖，尚缺真实文件样本做 HTTP 命中验证。
- [验证] 已构建包含静态 root 修复的正式 release 包：`runtime/codex-validation/nginx-release-static-root-fix-20260605-172033/Plant3D-AIOS-win-x64/release`。`BUILD_INFO.json` 标记 `nginxBundled=true`、`nginxStaticRoot=viewer-root`，`bin/aios-database.exe` SHA256 与 `aiosDatabaseSha256` 一致，包内 `bin/nginx/nginx.exe` SHA256 与官方下载缓存一致。运行 `start-plant3d.ps1 -Port 3498 -EnableNginx on -RequireNginx -ViewerHost 127.0.0.1 -ViewerPort 3497` 后脚本识别 Nginx/`viewer-root`，`GET /api/version` 返回 200，临时主服务已停止。
- [验证] 已补充 `/files/` Nginx 代理 HTTP 命中验证：在站点 `nginx-static-smoke-1780650910-3396` 的 `output_root` 下创建 `nginx-file-smoke.txt`，启动站点后请求 `GET http://127.0.0.1:3397/files/output/nginx-file-smoke.txt` 返回 200，内容命中 `nginx file proxy smoke`。临时主服务、站点和 Nginx 均已停止。

## Archived Previous Findings

# AMS BRAN quick deploy 解析放大开发发现

## 2026-06-03 Discovery

- [任务] MCP 会话要求“结合 grill-me skill，使用 planning-with-files skill 制定开发计划”；本轮将当前任务切换为 AMS BRAN quick deploy 解析放大修复计划，并把 AABB SQLite index 计划归档保留。
- [目标] 用户要求使用 `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams7999_0001` 作为 dbfile 自动部署，并验证 `BRAN 24383/73930` 显示。
- [代码审查] `src/web_server/models.rs::QuickDeployTestRequest.wait` 当前默认 `true`；同步路径会等待 parse/generate/start 完成。AMS 这类大库解析会超过 HTTP 客户端超时，导致请求方误判为部署卡死，但后台进程仍可能继续运行。
- [代码审查] `src/web_server/admin_handlers.rs::quick_deploy_site()` 直接调用 `managed_project_sites::quick_deploy_test()`；admin 部署入口当前复用测试部署语义。
- [风险] `quick_deploy_test()` 内部持久化固定数据库凭据 `quicktest / QuickTest@2026`；这适合作 smoke，但不适合作正式 admin 归档站点的默认凭据语义。
- [根因] `src/web_server/managed_project_sites.rs::should_run_db_index_prescan()` 当前条件是 `site.auto_parse_related_dbnums && site.manual_db_nums.is_empty()`；quick deploy 单库部署会设置 `manual_db_nums=[dbnum]`，因此不会跑 db_index 精确依赖预扫。
- [根因] `resolve_included_db_files_detailed()` 在 `auto_parse_related_dbnums=true` 时，若启用 sqlite-index 且 `manual_db_nums` 非空，会将 `allow_type_fallback` 设为 true；精确依赖为空/失败后会 fallback 到 `RELATED_DEPENDENCY_DB_TYPES`，这会把 AMS 单 BRAN 解析放大为当前工程 CATA 粗粒度解析。
- [结论] 旧 AMS 站点 `avevamarinesample-bran-24383-73930-20260603-215256-8081` 更像是“解析范围失控 + 同步请求超时”，不是已经完成意义上的部署失败；在修复依赖范围前继续等待不利于验证 `BRAN 24383/73930`。
- [grill-me 决策] admin quick deploy 默认应后台化，返回 `site_id/task_id`；同步等待只保留给显式 smoke/debug。
- [grill-me 决策] 单库 quick deploy 只要开启 `auto_parse_related_dbnums`，也应执行 db_index prescan，用精确依赖闭包补库。
- [grill-me 决策] 精确依赖为空/失败时，默认不回退全 CATA；粗粒度 fallback 必须成为显式选项，并在响应/日志中标明风险。
- [grill-me 决策] admin quick deploy 与 quick deploy test 应拆分凭据和语义，避免正式归档站点使用固定测试用户。
- [修复] `QuickDeployTestResponse` 新增 `task_id`；后台 quick deploy 成功创建持久化 admin task 时会结构化返回任务 ID，不再只藏在 message/warnings 中。
- [修复] `admin_handlers::quick_deploy_site()` 改为调用 `managed_project_sites::quick_deploy_admin()`，鉴权版 quick deploy 始终后台执行并返回 `202 Accepted`。
- [修复] `managed_project_sites` 增加 `QuickDeployProfile`：免鉴权 test 继续使用历史 `quicktest / QuickTest@2026`，admin 入口使用 per-dbnum 默认凭据（如 `siteadmin7999` / `AdminQuickDeploy@7999`），避免正式归档站点写入固定测试用户。
- [修复] `resolve_included_db_files_detailed()` 默认禁用 CATA type fallback；精确依赖为空/失败时仅记录 warn，继续解析目标库与必要系统/字典库。
- [修复] `should_run_db_index_prescan()` 改为只要 `auto_parse_related_dbnums=true` 就预扫，单库 quick deploy 不再因 `manual_db_nums` 非空跳过 prescan。
- [验证] `rustfmt --edition 2024 src/web_server/models.rs src/web_server/admin_handlers.rs src/web_server/managed_project_sites.rs` 通过。
- [验证] `cargo check --bin web_server --features web_server` 通过；仅有上游 `pdms_io` / `parse_pdms_db` 既有 warning。
- [验证] `ReadLints` 对 `models.rs`、`admin_handlers.rs`、`managed_project_sites.rs` 无新增诊断。

## Archived Previous Findings

# AABB Parquet 导入 SQLite Index 开发发现

## 2026-06-03 Discovery

- [任务] MCP 会话要求“结合 grill-me skill，使用 planning-with-files skill 制定开发计划”；本轮将 AABB Parquet 导入 SQLite index 计划前置写入 `task_plan.md`，并保留 quick deploy / ptset 历史计划作为归档上下文。
- [grill-me 规则] 对能通过代码确认的问题先探索代码，不直接反问用户；本轮已核对 `src/sqlite_index.rs`、`src/cli_modes.rs`、`src/main.rs`、`src/fast_model/gen_model/orchestrator.rs`、`src/fast_model/room_model.rs`。
- [现状] `src/sqlite_index.rs::refresh_dbnum_from_parquet_dir(dbnum, parquet_dir)` 已实现核心导入：读取 `aabb.parquet`、`instances.parquet`、`tubings.parquet`，校验 `dbnum`、`refno`、`aabb_hash`，写入 SQLite `items` 与 `aabb_index`。
- [现状] `read_parquet_aabb_table()` 会按 `aabb_hash` 建表并校验 bounds 有限、min/max 合法；`read_parquet_instances()` 要求 `instances.parquet` 的 `dbnum/refno_str/aabb_hash/spec_value` 合法；`read_parquet_tubings()` 会写 TUBI row 并参与 owner 聚合。
- [现状] BRAN/HANG/EQUI owner 聚合已在导入侧实现：children/tubings 的 bounds 可合并到 owner；BRAN/HANG 默认写聚合 row，EQUI 在自身没有 row 时写 owner aggregate。
- [现状] `src/cli_modes.rs::export_dbnum_instances_parquet_mode()` 在 `export_dbnum_instances_parquet()` 成功后，feature=`sqlite-index` 时会打开 `SqliteSpatialIndex::default_path()` 并调用 `refresh_dbnum_from_parquet_dir()`。
- [现状] `src/fast_model/room_model.rs` 明确说明生产主路径已改为 Parquet 导出成功后刷新 SQLite RTree，legacy `inst_relate_aabb` 刷新仅保留给显式 legacy/debug 重建入口。
- [现状] `src/fast_model/gen_model/orchestrator.rs` 在 `export_parquet_after_gen` 开启时会跳过生成后 SurrealDB 刷新，并提示 SQLite spatial index 将在 Parquet 导出成功后刷新。
- [已确认] `src/main.rs` 已有独立 `--import-spatial-index-parquet <PARQUET_DIR> --dbnum <N> [--spatial-index-output <SQLITE_PATH>]` 入口；旧 `--import-spatial-index <JSON_PATH>` 仍保留给 instances.json。
- [已确认] quick deploy 可作为真实 Parquet 产出验收入口；本轮已用 admin 归档站点 `quicktest-250160-8080` 跑通解析、模型生成和 Parquet 导出。
- [已确认] 房间计算前置的 SQLite RTree 消费 smoke 已通过：默认 `output/spatial_index.sqlite` 查询 `2013286704/431` 成功，返回 496 个候选并包含自身。
- [决策建议] 独立 Parquet 导入 CLI 应纳入本轮：索引丢失/损坏时可以从已有 Parquet 包重建，不必重跑模型生成。
- [决策建议] `tubings.parquet` 缺失默认 fatal：新导出包应始终包含空表或实表，旧包兼容应另设显式 flag。
- [决策建议] 房间计算默认不应 legacy fallback：缺失预建 SQLite index 应提示先导出/导入索引，避免重新引入 SurrealDB 依赖。
- [验证策略] 不运行 Rust test；按仓库规则使用 CLI、quick deploy、Parquet/SQLite 检查和房间计算 smoke。
- [环境] 项目内 `.cursor/skills/planning-with-files` 缺少 `scripts/session-catchup.py`；本轮 catchup 命令失败但不阻塞计划制定。
- [Phase 2 决策] 独立 Parquet 导入 CLI 已纳入本轮；它是 `spatial_index.sqlite` 丢失/损坏后的低成本重建路径，避免把索引修复绑定到模型重新导出。
- [Phase 2 决策] 默认导入路径要求新导出包包含 `tubings.parquet`（可为空表），缺失应 fatal；旧包兼容不进入默认路径，后续若需要再加显式兼容 flag。
- [Phase 2 决策] 多 dbnum 共用 SQLite 文件时按 `items.dbnum` 局部替换旧 rows，不删除其他 dbnum；同时保留旧错误 id range 清理，用于清掉曾经按 export dbnum 伪造的 RTree id。
- [Phase 2 决策] 房间计算默认消费预建 SQLite index，不自动 legacy fallback；缺失 index 应提示先跑 Parquet 导出/导入，避免重新引入 SurrealDB 依赖。
- [Phase 3 现状] `main.rs` 已提供独立入口：`--import-spatial-index-parquet <PARQUET_DIR> --dbnum <N> [--spatial-index-output <SQLITE_PATH>]`；旧 `--import-spatial-index <JSON_PATH>` 仍保留，兼容原 instances.json 脚本。
- [Phase 3 现状] `cli_modes.rs::import_spatial_index_parquet_mode()` 已打开/初始化 SQLite，调用 `refresh_dbnum_from_parquet_dir()`，并输出 EQUI/children/tubings/total/unique 统计与 SQLite 路径；未启用 `sqlite-index + parquet-export` 时返回明确错误。
- [验证] `cargo check --bin aios-database --no-default-features --features "review,parquet-export" --target-dir target-aabb-sqlite-cli-check` 通过（EXIT=0，只有上游 `pdms_io` 既有 warning）。
- [Phase 4 验证] `cargo build --bin aios-database` 通过；随后使用 `runtime/quick-deploy-last-payload.json` 复跑 `POST /api/admin/quick-deploy-test`，响应 `success=true`、`data.success=true`、`generated=true`、`parse_status=Parsed`、`warnings=[]`。
- [Phase 4 验证] quick deploy 归档站点已改用 `8022`，不再复用冲突的 `8020`；`generate.log` 显示 Parquet 写出成功：`instances=808`、`geo_instances=808`、`ptsets=0`、`transforms=547`、`aabb=517`、`manifest.json` 写入。
- [Phase 4 验证] `ptset_export` 统计为 `cata_hashes=0`、`empty_ptset_hashes=0`、`missing_cata_hash_refnos=861`；缺失 cata_hash 只进入诊断，不再导致 quick deploy fatal。
- [修复] `src/sqlite_index.rs` 的 Parquet 导入路径保留真实 `refno_u64` 作为 SQLite RTree id；`items` 新增 `dbnum` 列用于按导出 dbnum 局部替换。旧 schema 缺少 `refno_u64` 时才回退字符串解析。
- [修复] `replace_dbnum_aabbs_with_items_and_spec_values()` 先删除 `items.dbnum = <dbnum>` 对应 rows，再额外清理旧版错误导入产生的 `(dbnum << 32)..((dbnum+1)<<32)` id range，避免历史污染残留。
- [验证] 独立导入命令 `target/debug/aios-database.exe --import-spatial-index-parquet output/AvevaPlantSample/parquet/250160 --dbnum 250160 --spatial-index-output runtime/aabb-spatial-index-verify.sqlite` 成功，输出 `EQUI=3`、`Children=808`、`Tubings=0`、`total_inserted=811`、`unique=811`。
- [验证] clean SQLite 查询结果：`aabb_index=811`、`items=811`、`items where dbnum=250160` 为 `811`；错误 id `250160/431` 不存在，真实 id `2013286704/431` 为 `EQUI` 且 `dbnum=250160`。
- [验证] 默认 `output/spatial_index.sqlite` 经 quick deploy 复跑后：总行 `831`，`dbnum_items=811`，错误 id `250160/431` 不存在，真实 id `2013286704/431` 存在；说明本轮局部替换已清理历史错误 id，并保留其它 dbnum 既有行。
- [验证] 默认索引 spatial smoke：`target/debug/aios-database.exe spatial query-refno 2013286704/431 --distance-mm 1000 --include-self` 返回 `success=true`、`result_count=496`，结果包含 `2013286704/431`。
- [数据契约] 历史 Parquet 包可能存在 `instances.parquet.aabb_hash=''` 的行；这些行没有可索引 AABB，导入端应跳过并计数，不能阻塞整个 dbnum 或全量 clean rebuild。
- [修复] `src/sqlite_index.rs` 已在 `read_parquet_instances()` 与 `read_parquet_tubings()` 中跳过空 `aabb_hash`，并记录 `ImportStats::skipped_empty_aabb_count`；非空 `aabb_hash` 找不到对应 `aabb.parquet` 记录时仍 fatal，避免掩盖真实引用损坏。
- [验证] `output/AvevaPlantSample/parquet/250164` clean rebuild 通过，`skipped_empty_aabb_count=243`、`EQUI=2`、`Children=460`、`unique=462`；此前 fatal 的空 hash 不再阻塞导入。
- [验证] 全仓可发现 Parquet 包 clean rebuild 通过：扫描 `output/` 与 `runtime/` 下父目录为数字且具备 `aabb.parquet/instances.parquet/tubings.parquet` 的目录，导入 13 个目录到 `runtime/spatial-index-clean-rebuild-all-aabbskip-20260603-211958.sqlite`，最终 `aabb_index=41270`、`items=41270`、`orphan_aabb=0`、`duplicate_item_ids=0`。
- [注意] SQLite RTree `id` 当前保留真实 PDMS `refno_u64`，因此 `id >> 32` 是 `ref0` 而不是导出 `dbnum`；dbnum 归属应以 `items.dbnum` 为准。
- [验证] 已扫描 `output/` 与 `runtime/` 下 13 个可发现 `tubings.parquet`，全部为 0 行；当前没有现成样本覆盖非空 TUBI 写入与 BRAN/HANG owner 聚合。
- [修复] `src/cli_modes.rs::spatial_query_refno_mode()` 已增加空索引前置检查：`SqliteSpatialIndex::with_default_path()` 打开后若 `get_stats().total_elements == 0`，直接提示先完成 Parquet 导出自动刷新或运行 `--import-spatial-index-parquet` 重建。
- [验证] 空索引 smoke 使用 `AIOS_SPATIAL_INDEX_SQLITE=runtime/spatial-index-missing-smoke.sqlite`，现在返回 `SQLite spatial index 为空... --import-spatial-index-parquet ...`；默认 `output/spatial_index.sqlite` 正常查询仍 `success=true`、`result_count=496`。
- [残留] TUBI/BRAN-HANG 聚合需要另找或生成带非空 `tubings.parquet` 的样本；现有输出集无法覆盖。

## Archived Previous Findings

# ptset parquet measurement snapping 开发发现

## 2026-06-03 Discovery

- [任务] MCP 会话要求“使用 planning-with-files 来制定下一步开发计划”；本轮将 ptset 计划前置写入 `task_plan.md`，并保留 DuckLake 历史计划作为归档。
- [当前阶段] ptset 功能不是从零设计阶段，而是“核心代码已落地，等待真实数据导出和浏览器验收”阶段。
- [后端契约] `plant-model-gen/src/fast_model/export_model/export_dbnum_instances_parquet.rs` 已包含 `instances.cata_hash`、`ptsets.parquet` 写出、`manifest.tables.ptsets`、`ptset_unit` 和 `ptset_export` 诊断字段。
- [前端 loader] `plant3d-web/src/composables/useDbnoInstancesParquetLoader.ts` 已支持 `manifest.tables.ptsets`、懒注册 `ptsets.parquet`、按 `refno -> instances.cata_hash -> ptsets` 返回 `PtsetResponse`。
- [测量行为] `plant3d-web/src/composables/useXeokitMeasurementTools.ts` 已改为严格 ptset 语义：普通表面命中只用于确定 refno 和触发加载，最终测量点必须是 `ptset:<refno>#<point_number>`。
- [UI 文案] `plant3d-web/src/components/tools/MeasurementPanel.vue` 已说明“测量点必须捕捉 ptset，关闭捕捉不会回退表面测量”。
- [事实源] `cata_hash` 应被视为 PE/refno 对应实例侧数据，权威来源是 `EleGeosInfo.cata_hash` / instance info / cache，而不是从 `ptsets.parquet` 反推。
- [风险] 当前后端导出查询从 `pe -> inst_relate -> inst_info(out)` 读取 `out[0].cata_hash`。如果真实库里该字段不稳定或为空，需要改为更直接的 PE/instance-info 来源，或者增加明确兜底；不能让 ptset 表决定 cata_hash。
- [修正] `query_ptset_export_data()` 已改为优先读取 `out[0].cata_hash`，同时读取 `record::id(out[0])` 作为保守兜底；两者都必须通过 `is_valid_cata_hash()`，避免把 `EleGeosInfo::id_str()` 的 `refno_sesno` 退化 ID 当作 cata hash。
- [验证] `rustfmt --edition 2024 --check src/fast_model/export_model/export_dbnum_instances_parquet.rs` 通过；`cargo check --features parquet-export --lib` 在加入 NASM PATH 后通过。
- [阻塞] 真实 scoped Parquet 导出尚未完成。历史失败原因包括 `D:/backup-dbs/ams-8020.db LOCK` 和 `127.0.0.1:8020` 拒绝连接；下一步应先恢复数据库环境，不删除锁、不强杀未知进程。
- [Phase 2 尝试] `127.0.0.1:8020` 当前监听的是 `runtime/admin_sites/quicktest-250160-8080/data/surreal.db`，不是目标 `D:/backup-dbs/ams-8020.db`；使用 ws 配置导出时认证失败，使用 file-mode 直连目标 RocksDB 时被 `LOCK` 阻塞。
- [部署约束] AvevaPlantSample 的部署/导出验证默认不能直接用 `CLI + DbOption.toml` 拼配置；应使用 quick deploy test 快速生成可行配置，并在 admin 站点部署配置中归档。
- [部署约束] quick deploy test 生成配置前必须做前置校验：`project_name` 不重名、DB 端口不冲突、站点端口不冲突等；端口校验通过后才能执行配置。
- [Phase 2 尝试] 临时手动启动 `18020` 连接 `D:/backup-dbs/ams-8020.db` 后，`7011/AvevaPlantSample` 可连接但缺少 `inst_relate`/`pe` 表，说明该数据路径不是可用 APS 模型库；临时 `18020` SurrealDB 已停止。
- [验证策略] 不运行 Rust/前端 test 作为默认路径；按仓库规则使用 `cargo check`、CLI 导出、Parquet/DuckDB 检查和浏览器联调。
- [quick deploy 复跑] 已启动本地 `target/debug/web_server.exe` 并设置 `AIOS_ENABLE_QUICK_DEPLOY_TEST=1`，随后调用 `POST /api/admin/quick-deploy-test`：`project_path=AvevaPlantSample`、`db_file=aps250160_0001`、`pipeline_db_mode=ws`、`wait=true`、`force_recreate=false`、`start_site=false`。
- [quick deploy 当前失败] 本轮复跑立即返回 `409 Conflict`，未进入解析/生成；根因是复用已有同名站点 `quicktest-250160-8080` 时保留旧 `db_port=8020`，而当前 `web_server` 自启动的主 SurrealDB 已占用 `8020`。
- [quick deploy 证据] Admin runtime 显示 `db_port_conflict=true`、`db_conflict_pids=[59684]`；进程命令行为 `surreal start --bind 0.0.0.0:8020 --user root --pass root rocksdb://D:/backup-dbs/ams-8020.db`；归档配置 `runtime/admin_sites/quicktest-250160-8080/DbOption.toml` 中 `surreal_bind="127.0.0.1:8020"`。
- [quick deploy 代码边界] `quick_deploy_test()` 命中同名站点后走 `update_site()`；当前更新请求没有重新分配 `db_port`，`update_site()` 在 `assert_port_available_with_conn()` 阶段检测到旧端口被外部进程占用并返回冲突。
- [历史部署失败] `quicktest-250160-8080/logs/generate.log` 显示模型生成主体已完成，失败发生在生成后 `export_parquet_after_gen`：`query_ptset_export_data()` 的 SQL 使用 `record::id(out[0]) as inst_info_id`，查询失败导致 `Parquet 导出 dbnum=250160 失败` 和子进程退出码 1。
- [grill-me 决策点] quick deploy 复用 failed/stopped 同名站点时，若端口来自自动 quicktest 分配，建议允许重新分配不可用端口并重写归档配置；若端口是用户显式指定，则继续严格冲突失败。
- [Phase 2 修复确认] 当前 `quick_deploy_test()` 复用同名站点时先调用 `resolve_quicktest_reuse_ports_with_conn()`：旧自动 `db_port/web_port` 仍可用则复用，不可用则从自动端口段重新分配；显式 `web_port` 仍走 `reserve_explicit_port()`，冲突时失败，不会悄悄改用户指定端口。
- [Phase 2 落盘路径] 重分配后的端口通过 `UpdateManagedSiteRequest { db_port, web_port }` 进入 `update_site()`；`update_site()` 在事务中调用 `assert_port_available_with_conn()` 和 `persist_site_with_conn()`，随后 `write_site_files()` 重写 `runtime/admin_sites/<site_id>/DbOption*.toml`，满足归档配置可复现要求。
- [Phase 3 修复] `query_ptset_export_data()` 已删除 `record::id(out[0]) as inst_info_id` 和 Rust 层 `inst_info_id` fallback；主 SQL 仅读取 `in as refno`、`out[0].cata_hash`、`out[0].ptset`。缺失/非法 `cata_hash` 只计入 `missing_cata_hash_refnos`，不再让 quick deploy 的 Parquet 导出主查询因 record id fallback fatal。
- [验证] `rustfmt --edition 2024 --check src/fast_model/export_model/export_dbnum_instances_parquet.rs` 通过；`cargo check --features parquet-export --lib` 通过（EXIT=0，只有既有依赖 warning）。

## Archived Previous Findings

# DuckLake ModelWriter 下一步开发发现

## 2026-05-17 Discovery

- [需求] MCP 会话要求“提出下一步的详细方案，使用 planning skills 用中文制定开发文件”；本轮按 `planning-with-files` 在根目录更新 `task_plan.md`、`findings.md`、`progress.md`。
- [上下文] 最近打开文件集中在 `plant-model-ducklake` 与 `plant-model-gen` DuckLake 相关文件，因此下一步方案聚焦 DuckLake ModelWriter / storage adapter 收敛。
- [现状] `plant-model-ducklake` 是独立 DuckLake storage adapter crate，当前 scope 包含 storage config、canonical raw batch DTO、planned write contract、DuckDB backend、schema manifest、JSON/core smoke examples。
- [现状] `plant-model-gen/goals/ducklake-model-writer/brief.md` 明确 `DuckLakeModelWriterBackend` 是 `ModelWriterBackend` 的 opt-in 第三后端，目标是直接通过 Rust `duckdb` crate 写 `ducklake-canonical` raw 表。
- [边界] `pe_transform_store.rs::register_ducklake` 是历史 pe_transform stub，goal 明确不复用也不删除；本轮下一步不应把 pe_transform DuckLake 与 ModelWriter DuckLake 混合。
- [边界] 本期 DuckLake writer 只覆盖 trait 已暴露的 9 张 Phase 1 raw 表；tubi/transforms/refno_assoc 相关 6 项作为 Known Gap 显式报告。
- [发现] `Cargo.toml` 已有 optional `duckdb` 依赖，注释说明由 `model-writer-ducklake` feature 使用；`options.rs` 已有 `ModelWriterMode::DuckLake` 与 `as_str() == "ducklake"`。
- [风险] `model_writer_ducklake.rs::create_table_ddl()` 使用了较简化的 in-repo DDL（部分 payload_json / mesh fields），而 `plant-model-ducklake/src/schema.rs` 定义了更完整的 raw schema；下一步必须先做 schema diff，避免两套 canonical 长期分叉。
- [验证] 仓库规则禁止 `cargo test`；下一步验证应使用 `cargo check`、`model_writer_verify --mode ducklake --json`、web_server POST 与 DuckDB SQL 对账。
- [环境] `planning-with-files` 的用户级 `session-catchup.py` 路径不存在；已作为非阻塞问题记录。
- [审计] `DuckLakeModelWriterBackend` 已实现 trait 的 8 个方法：`init` 打开 DuckDB/DuckLake 并建 9 表，`cleanup` skipped，`write_base_batch` 写 6 张基础表，`persist_mesh_results` 写 mesh AABB/vec3 并更新 `raw_inst_geo`，`persist_inst_relate_aabb` 写 AABB 关系，`reconcile_missing_neg_relations` 写 sentinel 负关系，`run_boolean_bridge` 按 Non-Goal skipped，`finalize` checkpoint 并追加 Known Gap reports。
- [风险] `reconcile_missing_neg_relations` 当前把缺失 carrier 写为 `target_refno="__reconcile_pending__"` 的 sentinel 行，不等价于 Surreal backend 的真实 carrier→target 解析；SQL parity 需要显式 EXCEPT，后续若要求完全 parity 必须实现 raw 表 JOIN 或 provider 查询。
- [风险] `write_base_batch` 返回 `ModelWriteBatchReport::default()`，不会把 DuckLake 侧发现的 missing neg carriers 反馈给后续 reconcile；当前依赖上游/Surreal 逻辑时可能不足，需在 CLI verify 或真实生成里确认调用链期望。
- [风险] `raw_inst_relate` in-repo DDL 是 `(refno, inst_id, payload_json)`，独立 crate canonical schema 是 `(parent_refno, refno, snapshot_id, run_id, written_at, is_deleted)`；语义和主键完全不同。
- [风险] `raw_inst_info` in-repo DDL 使用 `inst_id` 而独立 crate 使用 `refno`，且缺少 `snapshot_id/run_id/written_at/is_deleted`；如果 downstream 以 schema manifest 为准，会无法直接对账。
- [风险] `raw_aabb` / `raw_vec3` in-repo DDL 使用 `aabb_id`、`vec3_id/payload`，独立 crate 使用 `aabb_hash`、`vec3_hash/x/y/z`；mesh payload 的 JSON 化会影响 SQL parity 和 projection 复用。
- [风险] `raw_inst_relate_aabb` in-repo DDL 使用 `(refno, aabb_id, source)`，独立 crate 使用 `(refno, aabb_hash, snapshot_id, run_id, written_at, is_deleted)`；字段名和审计列不一致。
- [清理] `model_writer_ducklake.rs` 文件头仍描述 Slice 2-4 “intentionally NOT IMPLEMENTED / bail”，但实际下方已有 Slice 2/3/4 写入路径；这是陈旧注释，应在下一次代码修改中修正。
- [验证] `src/bin/model_writer_verify.rs` 的默认 `--mode ducklake --json` 只调用 `model_writer_contract_evidence(mode)`，不会打开 DuckDB/DuckLake；运行时 smoke 必须使用 `--exec --mode ducklake --json` 且启用 `model-writer-ducklake` feature。
- [验证] `src/web_server/model_writer_verify.rs` 当前 POST endpoint 也只返回 `model_writer_contract_evidence(mode)`；它是非破坏静态 evidence，不会执行 `DuckLakeModelWriterBackend::init()`，因此不能替代 CLI `--exec` smoke。
- [环境] 明确 Rust 工具链路径可用：`D:\Rust\.cargo\bin\cargo.exe --version` 返回 `cargo 1.97.0-nightly (4f9b52075 2026-05-01)`。
- [阻塞] `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` 失败，退出码 101；根因在 `libduckdb-sys v1.10502.0` custom build script exit code 1，不是当前 Rust 业务代码类型错误。
- [阻塞] `libduckdb-sys` 输出包含大量 MSVC warning（如 C4530 / C4267）和 `VCINSTALLDIR=None` / `LIB=None` / `INCLUDE=None` 环境记录；需要复查是否必须在 VS Developer PowerShell / vcvars 环境下编译，或改用已验证的 DuckDB 构建方式。
- [发现] `duckdb-1.10502.0` 的 `default = []`，因此 `plant-model-gen` 的 `default-features = false, features = ["bundled"]` 与 `plant-model-ducklake` 的 feature 组合在 DuckDB 默认特性层面没有本质差异。
- [阻塞] 使用同一 `target-ducklake-verify` 并加 `-j 1` 重跑后，`libduckdb-sys` 失败点收敛为 `LINK : fatal error LNK1114: 无法覆盖原始文件 ... libduckdb.a；错误代码 112`。
- [环境] `Get-CimInstance Win32_LogicalDisk` 复查显示 `D:` 已降到约 `0.03GB` 可用空间，Windows 错误码 112 对应磁盘空间不足；当前 Phase 3 阻塞优先归因为 DuckDB bundled C++ 静态库归档需要更多 D 盘空间，而不是 ModelWriter Rust 代码错误。
- [下一步] 先释放 `D:` 空间（优先清理本轮生成的 `target-ducklake-verify` 或其它可重建 target/cache）或把 DuckDB 验证 `--target-dir` 指向剩余空间更充足的磁盘；再重跑 DuckLake feature `cargo check` 和 `model_writer_verify --exec`。

## 2026-05-17 续 · Phase 3 闭环 Findings

- [解阻] 磁盘阻塞已自动解除：复检 `D:` 128.42GB / `C:` 16.03GB / `E:` 102.64GB，`target-ducklake-verify` 已被前次失败链路清理；无需手动迁移 target-dir。说明上次的"0.03GB"是 link 阶段写部分对象时把盘撑爆，后续临时文件被释放即恢复。
- [陷阱] PowerShell `Tee-Object` 在长时间 cargo 管道里有缓冲卡顿，导致 exit_code unknown。**改用 `Out-File`** 即可稳定捕获完整日志和退出码；后续长任务推荐这种写法。
- [验证] `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` 二次重跑：EXIT=0，1m 22s（增量），0 error / 110 warning（均为依赖库 dead_code）。
- [证据] `model_writer_verify --mode ducklake --json` 静态路径输出 8 stages：init / cleanup / base_batch / mesh_persist / inst_relate_aabb / missing_neg_reconcile / finalize 均 `implemented`；boolean_bridge `skipped`（phase2 Non-Goal）。
- [证据] `model_writer_verify --mode ducklake --exec --json` 执行路径在 **599ms** 内完成：`init: executed item_count=9` 证明 bundled DuckDB 成功 `INSTALL/LOAD ducklake` + `ATTACH metadata.ducklake` + 建 9 张 raw 表；`cleanup: skipped`（reason: ducklake 不清理 SurrealDB）；6 个 `known_gap:*` stages 全部 skipped 且 reason 指向 `cata_model.rs / refno_assoc_index.rs` 写入面（Q1=C scope）。
- [证据] 磁盘落地：`output/model_writer_storage/ducklake/metadata.ducklake` 3,084 KB；`data/ducklake-canonical/` 下 9 个 raw 表目录与 init 计数完全吻合（raw_aabb / raw_geo_relate / raw_inst_geo / raw_inst_info / raw_inst_relate / raw_inst_relate_aabb / raw_neg_relate / raw_ngmr_relate / raw_vec3）。
- [结论] in-repo `DuckLakeModelWriterBackend` 在本机 Windows + bundled DuckDB 下可启动 DuckLake runtime，DuckDB extension `INSTALL ducklake; LOAD ducklake; ATTACH` 路径稳定可用，**无需 DuckDB CLI 外部工具**（这回答了 task_plan.md Key Question #4）。
- [边界] `--exec` 当前只覆盖 `init → cleanup → finalize` 生命周期，没有真实 batch 写入；要回答 Key Question #1（Slice 2-4 真实写入正确性），需要在 Phase 4 用真实 dbnum 跑 `write_base_batch` / `persist_mesh_results` / `persist_inst_relate_aabb` / `reconcile_missing_neg_relations` 全链路。
- [后续风险] `reconcile_missing_neg_relations` 的 sentinel 行 `target_refno="__reconcile_pending__"` 在 Phase 4 真实数据 smoke 时会出现在 `raw_neg_relate` 表里，SQL 对账需显式 EXCEPT 或加 `is_sentinel` 列，否则会与 Surreal backend 真实 carrier→target 解析结果不一致。

## 2026-05-17 续2 · Phase 4 样本探查 Findings

- [build] `cargo build --bin aios-database --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` **20.83s** 完成 (EXIT=0)；lib 部分被 model_writer_verify build 复用。`target-ducklake-verify/debug/aios-database.exe` 已就绪。
- [安全] 已确认 `cli_modes::run_regen_model` 第 1683 行守卫 `if db_option_override.model_writer_mode.writes_to_surreal()`：DuckLake/DrainOnly 模式 **跳过 `pre_cleanup_for_regen`**，不会删除 SurrealDB 现有模型数据，满足 goal brief.md 「Ask Before / 不删除 SurrealDB cleanup」。
- [阻塞] **本机 dbnum=1112 不可用**：虽 `dbnum_info_table` 有 dbnum=1112 (count=2, file=ams1112_0001) 注册，但 `INST` 表里 `WHERE dbnum=1112` 返回 0 条；可能数据未完成 sync 或被清理。无 INST root 则 `--regen-model` 收集不到 target_refnos，gen 流程会 no-op。
- [现状] 本机 INST 表合计 111 条，按 dbnum 分布：24383(35) / 7999(31) / 23399(24) / 24381(10) / 7997(6) / 23584(3) / 17496(1) / 25688(1)。其中 7997 在 pe_transform 工作里曾扩展成 176K transform 节点，规模偏大；17496 / 25688 (n=1) 与 dbnum=1112 (count=2) 体量最接近，是最佳替代 first-smoke 样本。
- [候选决策] 建议 first-smoke 用 `dbnum=17496` 或 `dbnum=25688`：能在最小风险下验证 ducklake writer 真实写入 9 张 raw 表是否非空、reconcile sentinel 是否生成、finalize Known Gap 表是否正确列出；SQL parity 因数据量极小可手工对账，不需重型 DuckDB CLI。
- [决策点边界] 不擅自跑生成；等待用户在「按推荐方案继续」语义下选定具体 dbnum，再触发：`aios-database.exe -c db_options/DbOption-cli.toml --regen-model --dbnum <N> --model-writer ducklake`。

## Archived Previous Findings

# RUS-248 批注后驳回流转发现

## 2026-05-14 Discovery

- [关键] 外部流程校验已经存在：`/api/review/workflow/verify` 对 `return` 使用 `ReturnReject` 批注门禁，要求当前节点是 `jd/sh/pz` 且至少有 `open` 或 `rejected` 批注。
- [关键] `/api/review/workflow/sync` 的 `return` mutation 会更新 `review_tasks.current_node/status/return_reason`，并同步 `review_forms`，是 PMS 外部驱动应使用的落库路径。
- [风险] `plant3d-web/src/composables/useReviewStore.ts::applyExternalWorkflowChange()` 当前仍调用内部 `reviewTaskReturn()` / `reviewTaskApprove()`，最终落到 `/api/review/tasks/{id}/return|approve`。
- [风险] 内部 API 会校验 JWT `user_id` 是否等于当前节点 owner；PMS 外部流程中 owner 由 `next_step.assignee_id` 声明，二者命名空间不一致时会导致“verify 通过但 changed 落库失败”。
- [现状] 被驳回到 `sj` 后，任务会被前端识别为 `returnedInitiatedTasks`；设计端面板展示退回意见、批注列表，并允许保存 confirmed record。
- [现状] 设计端保存处理结果后不直接推进工作流；再次流转依赖 PMS 后续触发 `active` 或内部提交。
- [方案] PMS iframe/postMessage 外部路径应统一改为 `/api/review/workflow/sync`，并传入 `actor`、`next_step`、`comments`。
- [实现] `plant3d-web` 的 external `workflow_changed` 已改走 `/api/review/workflow/sync`，内部按钮仍保留原 `/api/review/tasks/{id}/submit|return|approve` 路径。
- [兼容] `nextStep` 优先使用 PMS 显式传入；旧 PMS/simulator 未传时，前端按 action/currentNode/targetNode 推导下一节点和负责人。
- [实现] 后端 `review_workflow_history` schema 原已支持 `form_id/target_node/source/actor_*`，本轮补齐 `workflow/sync` 写入字段，便于后续排查外部流转。
- [验证] `npm run type-check` 和 `cargo check --bin web_server --features web_server` 均通过；真实 return/active 数据闭环尚未执行。
- [验证] 真实 HTTP payload 闭环已通过：`SJ active -> JH return -> SJ fixed -> SJ active`，最终任务 `jd/submitted` 且 `returnReason=null`。
- [验证] SurrealDB 直查确认三条 `review_workflow_history` 均写入 `form_id/target_node/source/actor_id/actor_role`，source 分别为 `rus248-cli-verify-active`、`rus248-cli-verify-return`、`rus248-cli-verify-reactive`。
- [环境] 默认 target 启动当前 web_server 失败是因为旧 `:3100` 服务锁定 `target/debug/web_server.exe`；独立 `target-rus248` + `WEB_SERVER_PORT=3199` 可绕开。首次全量编译需要 `C:\Program Files\NASM` 在 PATH。

## Archived Previous Findings

# pe_transform 后端重构发现

## 2026-05-08 Discovery

- `Cargo.toml` 已有 `parquet-export` feature，负责引入 `parquet`、`arrow-array`、`arrow-schema`、`polars`；新增 transform Parquet 能力应考虑复用或拆出更轻的 `transform-store-parquet`。
- `options.rs::validate_model_writer_features` 已有清晰的 feature 校验模式，可复用于 transform backend，例如未启用 `transform-store-ducklake` 时禁止 `--transform-read-backend ducklake`。
- `pe_transform_refresh.rs` 当前直接调用 `save_pe_transform_entries(&entries)` 批量写 SurrealDB，是插入 `PeTransformSink` / dual-write 的主要入口。
- `transform_cache.rs` 和 `transform_rkyv_cache.rs` 当前读取链路是 rkyv/内存优先，miss 后从 SurrealDB `pe_transform` 查询；新增 source 应保持最终统一 prime 到内存 cache。
- `fast_model/export_model/export_dbnum_instances_parquet.rs` 已有 `transforms.parquet`，但它表达的是唯一 transform hash 到矩阵，不是 `refno -> local/world transform` 的 PE 映射，不能直接替代 `pe_transform` 表。
- DuckLake 支持 `ATTACH 'ducklake:metadata.ducklake' AS lake (DATA_PATH 'data/')` 后建表写入，也支持先写外部 Parquet 再 `CALL ducklake_add_data_files(...)` 注册。
- DuckLake partitioning 支持 `ALTER TABLE ... SET PARTITIONED BY (...)`，首版建议按 `project_name, dbnum` 分区，避免按 refno 产生过多小文件和目录。
- 首轮测试样本已由用户指定为 `dbnum=7997`。
- 对比前必须清理历史 `pe_transform`，否则 SurrealDB 旧数据可能和新刷新的 Parquet/DuckLake 数据混在一起，导致矩阵一致性和加载耗时结论失真。
- 当前实现中 `dual` 写入表示 SurrealDB + Parquet 双写；DuckLake 首版通过 `transform-store-ducklake` 生成注册 SQL 脚本，不直接引入 Rust DuckDB/DuckLake 运行时。
- `transform_read_backend=ducklake` 当前先复用 Parquet source 读取文件内容；DuckLake 原生 time-travel 查询需要后续接入 DuckDB/DuckLake CLI 或 Rust binding。
- 当前环境 `cargo` 不在 PATH，无法做 Rust 编译校验；后续必须在 Rust 工具链可用环境补跑 `cargo check`，再跑真实 `--refresh-transform 7997` 流程。
- 本轮无法产出真实耗时 profile：缺少 Rust 工具链、DuckDB/Surreal CLI，且 8020 端口未检测到数据库监听；表格只能记录待测项和当前阻塞状态。

## 2026-05-08 Next-Step Findings

- 下一步不应继续扩大功能面；优先把当前 worktree 主体实现编译收敛，再做 `7997` 的 SurrealDB/Parquet 对比。
- 首轮 profile 表必须区分“计算 transform”和“存储/读取 backend”两类耗时，否则无法判断 Parquet/DuckLake 是否真正改善预热阶段。
- `dual` 写入的验收对象是 SurrealDB baseline 与 Parquet 文件一致性；DuckLake 首轮只验证注册脚本和 metadata 管理，不承诺原生读取性能。
- 对比表的核心列应固定为：`Backend | Write Time | Read Time | Loaded | Missing | Mismatched | Max Delta | Notes`。
- 如果 Parquet 出现 missing，优先排查分区路径和递归扫描；如果出现 mismatched，优先按 refno 抽样比较 local/world 矩阵展开列。
- 指定 `D:/Rust/.cargo/bin` 后 Rust 工具链可用；当前真正阻塞不再是 cargo 缺失，而是 `rs-core` 的 `rust-ploop-processor` git 依赖无法在线更新且本机没有本地副本。
- 为了使后续 `cargo check` 可继续，需要二选一：提供 `D:/work/plant-code/rust-ploop-processor/ploop-rs` 本地仓库并加 patch，或恢复访问 `https://github.com/happyrust/rust-ploop-processor`。

## 2026-05-11 Phase 10 Findings

- [性能] **Parquet 读取速度约 9.5 倍于 SurrealDB**：Parquet 1,711ms vs SurrealDB ~16,250ms。这证实了 Parquet 作为 transform 预热数据源的可行性。
- [精度] Parquet 序列化/反序列化引入 max_delta=0.000854 的 float 精度差异，影响 58,930/143,222 条记录（41%），但绝对误差极小（<0.001mm），在工程精度内可接受。
- [数据完整性] Parquet missing=32,115 不是 bug：SurrealDB 包含 175,337 条历史记录（可能涵盖多个 dbnum），而 Parquet 仅写入本次刷新的 143,222 条。差值 32,115 = 非本次 dbnum 的历史数据。
- [清理] `--clear-transform-before-refresh` 报告 refnos=0，说明按 `dbnum=7997` 查询历史 pe_transform 的查询未找到对应记录。可能原因：pe_transform 表以 refno 为主键、不含独立 dbnum 字段，或 dbnum 筛选逻辑有误。需复查 `clear_pe_transforms_for_dbnums` 的 SurQL 查询。
- [对比输出] 出现两行 SurrealDB 对比结果（第一行 missing=1053/mismatched=0，第二行 missing=0/mismatched=75575），需要检查 `compare_backends` 函数是否对同一后端做了两次不同维度的对比（如分别对比 local 和 world transform），或是代码误输出。
- [写入确认] Dual 写入成功：SurrealDB 和 Parquet 均有数据写入，Parquet 文件 4.5 MB。
- [编译] `cargo build` 通过（29s），`cargo run` 运行完整流程 724s（~12 分钟），其中大部分时间花在 176,390 节点的 transform 计算和 SurrealDB 批量写入。

## 2026-05-11 Phase 11 Profile Findings

- [瓶颈] **Parquet 写入是最大耗时瓶颈**（245,339ms = 39.5%），超过 SurrealDB 写入（145,763ms = 23.4%）。原因：`save_entries_to_parquet` 每批（500条）调用时执行 read-merge-dedup-write 全文件操作，随文件增大为 O(n²)。
- [瓶颈] 计算 local/world transform 占 37.1%（230,888ms），主要由 BFS 遍历 + 逐节点 SurrealDB 查询 `get_local_mat4` 和 `get_children_refnos` 贡献。
- [性能] SurrealDB 批量写入（23.4%）在三个阶段中效率最高，因为使用了原生批量 INSERT。
- [性能] transform_cache prime = 0ms，说明 `prime_global_transform_cache_from_pe_entries` 未实际执行缓存操作（可能全局缓存未初始化）。
- [优化方向] Parquet 写入优化建议：(1) 每批写独立文件 `batch_NNN.parquet`，最后一次合并去重；(2) 或在内存中累积所有 entries，最终一次写入；预期可将 Parquet 写入从 245s 降到 <5s。
- [读取对比] Parquet 读取 1,698ms vs SurrealDB 读取 ~14,900ms，Parquet 读取约 8.8x 快。这说明 Parquet 写入慢只是当前实现问题，读取端已经验证了 Parquet 格式的优势。

## 2026-05-11 Parquet 优化 & Compare 修复 Findings

- [优化] Parquet 写入从 O(n²) 优化为 O(n)：每批写独立 batch 文件 → 最终 merge+dedup。写入 245,339ms → 2,250ms（**73x 快**），finalize 1,113ms。
- [优化] 总刷新耗时从 621,990ms 降到 380,072ms（**39% 减少**），瓶颈已从 Parquet 写入转移到 BFS 计算（59.7%）和 SurrealDB 写入（39.7%）。
- [修复] Compare 冗余 SurrealDB 加载：当 `surreal` 在 `transform_compare_backends` 中时跳过重复加载，输出从 3 行变为 2 行（baseline + parquet）。
- [确认] 优化后 Parquet compare 结果不变：loaded=143222, missing=32115, mismatched=58930, max_delta=0.000854, elapsed=1743ms，证明 batch 写入+合并与旧的增量合并在数据正确性上一致。

## 2026-06-05 Viewer 独立站点 URL Findings

- [现状] 原管理端 Viewer URL 会拼 `/viewer/?backend=...&output_project=...&show_dbnum=...`，这是内部调试/跨端口包装形态，不适合客户访问。
- [用户目标] 客户访问形态应为独立 `plant3d-web` 根站点，例如 `http://123.57.182.243/?output_project=AvevaMarineSample&show_dbnum=7997`。
- [诊断] `http://127.0.0.1:33351/viewer/` 页面和静态资源可返回 200，但 `AvevaMarineSample/parquet/manifest_7998.json`、`instances.parquet`、`geo_instances.parquet` 返回 404；模型不显示的直接原因是输出数据不存在或未导出。
- [契约] 独立 `plant3d-web` 不应依赖 URL 中的 `backend` / `backendPort`。后端访问应由同源 Nginx 代理 `/api/`、`/files/`、`/ws/` 提供。
- [配置] Viewer Base 应是完整 URL（scheme + host + optional port/domain），优先级为 `AIOS_VIEWER_BASE_URL` → 站点 `public_base_url/public_entry_url` → 自动探测本机 IPv4 `http://<local-ip>` → 本机 `viewer_port` fallback。
- [多站点边界] 一个 Viewer Base URL 应绑定一个 web_server 后端；多站点并行对外服务时使用不同域名、端口或 Nginx vhost 隔离，不靠 `output_project/show_dbnum` 选择后端。
- [实现] 已新增共享 `web_server::get_local_ip_via_udp()`，并让 `/api/admin/app-config` 与受管 `viewer_url` 都可默认返回 `http://<local-ip>`。
- [部署] 已新增 `shells/deploy/nginx-plant3d-web.conf.example`，约定 `plant3d-web` 服务在 `/`，同源反代 `/api/`、`/files/`、`/ws/` 到目标 web_server。
- [新增需求] Viewer 独立站点方案需要覆盖 Nginx 自动配置和自动启动/reload，不能只提供静态示例文件。
- [自动化边界] Nginx 自动化应在 Linux/远端部署路径执行；Windows 本机开发保留 `vite preview` fallback，不强制安装 Nginx。
- [安全边界] 自动写 `/etc/nginx/conf.d/*.conf` 和 `systemctl reload nginx` 需要 root/sudo。无权限时应输出配置文件和命令作为降级结果，而不是静默失败。
- [验收边界] 自动化必须先 `nginx -t`，成功后再 reload/start；部署验收应检查 `/`、`/api/health`、模型 `/files/output/...` 可达性。
- [OS 差异] Linux 远端默认走系统 Nginx：写 `/etc/nginx/conf.d/plant3d-web-<site_id>.conf`，执行 `nginx -t`，再 `systemctl reload nginx` / `nginx -s reload`。
- [OS 差异] Windows 默认不要求 Nginx，继续用受管 `vite preview` 作为本机 fallback；只有显式配置 `AIOS_NGINX_BIN` / `AIOS_NGINX_ROOT` 时才启用 Windows Nginx 自动化。
- [OS 差异] Windows Nginx 自动化需要处理 `nginx.exe -t -p <nginx_root>`、`nginx.exe -s reload`、路径分隔符和是否已有 nginx 进程运行；失败时回退到受管 Viewer。
- [实现] Windows 本机已实现可选 Nginx 路径：检测 `AIOS_NGINX_BIN` 或常见 `C:\nginx\nginx.exe` / `D:\nginx\nginx.exe`；未检测到时记录日志并继续受管 `vite preview`，不阻断自动部署。
- [实现] Windows Nginx 检测成功时使用独立 prefix：默认 `runtime/admin_sites/<site_id>/nginx`，生成 `conf/nginx.conf` 与 `conf/conf.d/plant3d-web-<site_id>.conf`，避免依赖用户原始 `nginx.conf` 是否 include `conf.d`。
- [实现] Windows Nginx 配置校验和启动顺序为：`nginx.exe -p <prefix> -t` → `nginx.exe -p <prefix> -s reload` → reload 失败时启动 `nginx.exe -p <prefix>`。
- [运行态验证] 临时启动 `web_server` 到 `WEB_SERVER_PORT=3198`，设置临时 `ADMIN_USER=admin` / `ADMIN_PASS=admin-pass` 后登录成功；`GET /api/admin/app-config` 返回 `viewer_base_url="http://192.168.31.60"`，证明未配置 `AIOS_VIEWER_BASE_URL` 时会默认使用本机 IPv4。
- [兼容修复] 历史站点数据库中已有旧格式 `viewer_url`（包含 `backend=` / `data_source=parquet`）会被前端优先使用。已在 read-side API 返回前归一化旧 URL，不直接改历史 DB 行。
- [运行态验证] 重建并重启临时 `web_server` 后，`GET /api/admin/sites` 返回的历史站点 `viewer_url` 已归一化，例如 `http://192.168.31.60/?output_project=AvevaPlantSample&show_dbnum=250164`，不再包含 `backend=` / `data_source=parquet`。
- [端口修复] Windows 本机无 Nginx 时，默认 Viewer URL 必须带受管 Viewer 端口；否则 `http://<local-ip>/` 会落到 80 端口且不可达。已改为无 `AIOS_VIEWER_BASE_URL` / `public_base_url` 时生成 `http://<local-ip>:<viewer_port>`。
- [绑定修复] 受管 Viewer 原先只绑定 `127.0.0.1`，导致 `http://<local-ip>:<viewer_port>` 被拒绝连接。已新增 `AIOS_VIEWER_BIND_HOST`，默认 `0.0.0.0`，保证本机 IP URL 可访问。
- [运行态验证] 启动 `avevaplantsamplegoalfast-8084` 后，站点进入 `Running`，`viewer_port=3105`，`viewer_url=http://192.168.31.60:3105/?output_project=AvevaPlantSampleGoalFast&show_dbnum=250164`；直接请求该 URL 返回 HTTP 200，内容包含 plant3d/Vite 标识。
- [前端硬化] `/api/admin/app-config` 现在返回 `viewer_base_url_source`。当来源是默认 `local_ip` 且站点存在 `viewer_port` 时，管理端会生成 `http://<local-ip>:<viewer_port>/...`；当来源是显式 `AIOS_VIEWER_BASE_URL` 或 `VITE_VIEWER_BASE` 时，仍按配置的 Nginx/public 入口使用。
- [Linux Nginx 自动化] 非 Windows 受管 Viewer 启动时会尝试 `AIOS_NGINX_CONF_DIR`（默认 `/etc/nginx/conf.d`）写入 `plant3d-web-<site_id>.conf`，执行 `nginx -t`，再尝试 `systemctl reload nginx` / `systemctl enable --now nginx` / `nginx -s reload`。无 Nginx、无权限、校验失败、reload 失败都会写入 viewer 日志；校验失败不会 reload，且会继续受管 `vite preview` fallback。
- [验证限制] 当前本机只安装 `x86_64-pc-windows-msvc` Rust target，Linux `cfg(not(windows))` 路径未能在本机做 Linux target 编译；已完成 Windows 主目标 `cargo check` 和静态 diff 检查。
- [验证] 静态验证已完成：修改文件 `ReadLints` 无诊断，`git diff --check` 无空白错误；真实 Nginx + web_server runtime smoke 尚未执行。
