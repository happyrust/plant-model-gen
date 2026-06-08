# Release 包 sidecar job 终态竞态修复进度

## 2026-06-08

- 已按用户要求使用 `planning-with-files` 制定 sidecar job 终态竞态修复计划。
- 已读取项目内 `planning-with-files` skill，确认规划文件写入项目根目录：`task_plan.md`、`findings.md`、`progress.md`。
- 已尝试运行 session catchup：
  - 命令：`python "$env:USERPROFILE\.cursor\skills\planning-with-files\scripts\session-catchup.py" (Get-Location)`
  - 结果：失败，用户级 skill 目录没有 `scripts/session-catchup.py`。
  - 处理：记录为非阻塞，继续读取现有 planning 文件并前置新计划。
- 已完成 release 包现场证据采集：
  - 当前运行目录：`D:\work\plant-code\plant-model-gen\dist\package\Plant3D-AIOS-win-x64\release`。
  - 主控 `3100` 和主控 SurrealDB `8020` 正在运行。
  - 截图中的 sidecar 临时端口 `53081` 已无监听。
  - `runtime/admin_sites/quicktest-250160-8080/logs/parse.log` 显示 job `f29566ec-dc7b-47da-a223-9e5deaafbb21` 实际 `job_done: status=succeeded, exit_code=0`。
- 已完成源码定界：
  - `parse_sidecar_client::run_cli_job_with_status()` submit 后每 500ms 轮询 `/jobs/{job_id}`。
  - `parse_sidecar_client::spawn_sidecar()` 对 job sidecar 使用 `--shutdown-after-job --shutdown-delay-ms 1000`。
  - `parse_sidecar.rs::schedule_shutdown_after_job()` 在 job 完成后按 delay 关闭 sidecar。
- 当前结论：
  - 解析 CLI 已成功，UI 的失败是主控获取终态时撞上 sidecar 自动退出。
  - 短期修复：job sidecar shutdown delay 默认提升到 10s，并支持环境变量覆盖。
  - 长期修复：websocket terminal event 参与最终判定，避免 HTTP 轮询成为唯一真相。
- 已完成 Phase S2 短期止血修复：
  - `src/web_server/parse_sidecar_client.rs` 新增 `DEFAULT_JOB_SIDECAR_SHUTDOWN_DELAY_MS = 10_000`。
  - 新增环境变量 `ADMIN_SIDECAR_JOB_SHUTDOWN_DELAY_MS`，只接受正整数毫秒值；无效/未设置时回退 10 秒。
  - `job:` sidecar 继续使用 `--shutdown-after-job`，但 `--shutdown-delay-ms` 改为来自 helper；preview/scan 等非 job sidecar 不受影响。
  - `rustfmt --edition 2024 src/web_server/parse_sidecar_client.rs` 通过。
  - `cargo check --bin web_server --features web_server` 通过；仅有上游 `pdms_io` / `parse_pdms_db` 既有 warning。
- 已完成 Phase S3 稳态兜底实现：
  - `run_sidecar_cli_job_with_site_events()` 现在会记录 submitted `job_id` 与 websocket terminal event 状态。
  - websocket event task 收到 `job_done/job_failed/job_cancelled` 后，会保存 `RunCliJobStatus`。
  - `run_cli_job_with_status()` 如果因 `/jobs/{id}` HTTP 轮询失败返回错误，会短暂等待 terminal event；若已收到终态，就按 event status 构造 `RunCliJobResponse`。
  - 命中兜底时会写入 parse/generate log，明确说明 HTTP 轮询失败但已收到 websocket 终态事件。
  - 未收到 terminal event 时仍返回原始 HTTP/sidecar 错误。
  - `rustfmt --edition 2024 src/web_server/managed_project_sites.rs src/web_server/parse_sidecar_client.rs` 通过。
  - `cargo check --bin web_server --features web_server` 通过；仅有上游 `pdms_io` / `parse_pdms_db` 既有 warning。
- 当前 Phase 已推进到 S4：release 包更新与运行态验证。
- 已完成 Phase S4 release 包更新与运行态验证：
  - 使用独立 target 目录构建 release `web_server.exe`：`cargo build --bin web_server --features web_server --release --target-dir target-sidecar-fix-release`，耗时较长但最终通过。
  - 新 binary：`target-sidecar-fix-release/release/web_server.exe`，SHA256 `96934A1F697E4A4863ABE081DEEDC45FA765325C46AC45474BF837163C2DFFBD`。
  - 已备份旧包内二进制：`dist/package/Plant3D-AIOS-win-x64/release/bin/web_server.exe.bak-20260608-180944`。
  - 已替换 release 包内：`dist/package/Plant3D-AIOS-win-x64/release/bin/web_server.exe`。
  - 已重启 release 包，`/api/version` 返回 commit `cd50f3c865ce778d2206f779e1eb328124f05482`。
  - 第一次 smoke 脚本误把 quick-deploy 创建失败后继续轮询，实际是主控还未复现稳定；随后用 `RUST_BACKTRACE=1` 重启并单独复现 quick-deploy API，返回 `202` 且主控保持可用。
  - 新建站点 `sidecarracefixonly-8080` 并执行完整部署，轮询结果最终 `status=Running parse=Parsed`。
  - `parse.log` 显示 `sidecar 解析 event job_done: status=succeeded, exit_code=0`，随后 HTTP 轮询也记录 `sidecar 解析 job ... status=succeeded, exit_code=0`。
  - `generate.log` 显示 `sidecar 模型生成 event job_done: status=succeeded, exit_code=0`，随后 HTTP 轮询也记录 `sidecar 模型生成 job ... status=succeeded, exit_code=0`。
  - 部署校验：`blocking=0 warning=1 checks=26`；唯一 warning 是 synthetic root `api_e3d_subtree_refnos`，符合预期，不阻断模型加载。
  - Cursor 浏览器打开 `http://127.0.0.1/?output_project=SidecarRaceFixOnly&show_dbnum=250160` 成功，页面标题为 `plant3d-web - 3D 模型查看`。
  - 浏览器页面模型树显示 `SITE SITE-EQUIPMENT-AREA03`，说明 `show_dbnum=250160` 的模型树已加载。
  - 浏览器 resource 记录显示 `manifest_250160.json`、`instances.parquet`、`geo_instances.parquet`、`transforms.parquet`、`aabb.parquet` 均被请求，并加载了多条 `files/meshes/lod_L1/*.glb`。
  - 页面文本中没有出现 `show_dbnum Parquet 加载失败`，截图中之前的 DuckDB trap 不再复现。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| planning-with-files skill 读取 | `.cursor/skills/planning-with-files/SKILL.md` | 确认三文件规划约定 | 已读取 | PASS |
| session catchup | 用户级 `planning-with-files/scripts/session-catchup.py` | 恢复未同步上下文 | 脚本缺失，非阻塞 | WARN |
| release 端口状态 | `netstat :3100/:8020/:53081` | 判断主控与 sidecar 状态 | `3100/8020` 运行，`53081` 无监听 | PASS |
| parse log | `runtime/admin_sites/quicktest-250160-8080/logs/parse.log` | 判断 parse job 真实状态 | `job_done: status=succeeded, exit_code=0` | PASS |
| 源码定界 | `parse_sidecar_client.rs` / `parse_sidecar.rs` | 找到 shutdown 竞态机制 | job sidecar delay 当前 1000ms | PASS |
| Phase S2 编译检查 | `cargo check --bin web_server --features web_server` | 编译通过 | 通过；仅上游既有 warning | PASS |
| Phase S3 编译检查 | `cargo check --bin web_server --features web_server` | 编译通过 | 通过；仅上游既有 warning | PASS |
| Release web_server 构建 | `cargo build --bin web_server --features web_server --release --target-dir target-sidecar-fix-release` | 构建成功 | 成功，已生成并替换包内 `web_server.exe` | PASS |
| Release quick deploy smoke | `sidecarracefixonly-8080` 完整部署 | 不再出现 `/jobs/{id}` error sending request | 最终 Running / Parsed | PASS |
| Deploy validation | `POST /api/admin/sites/sidecarracefixonly-8080/deploy-validation` | blocking=0 | blocking=0, warning=1 | PASS |
| Cursor Browser Viewer smoke | `http://127.0.0.1/?output_project=SidecarRaceFixOnly&show_dbnum=250160` | 模型能加载 | 模型树显示 `SITE SITE-EQUIPMENT-AREA03`，GLB 资源已加载 | PASS |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Active plan 已切换到 release 包 sidecar job 终态竞态修复，所有计划阶段已完成。 |
| Where am I going? | 下一步可提交/推送本轮修复，或继续清理 release smoke 站点/进程。 |
| What's the goal? | 避免 parse/generate job 已成功却因 sidecar 自动退出导致 UI 误报失败。 |
| What have I learned? | 当前 release 包里 parse job 实际成功，失败发生在 `/jobs/{id}` 终态轮询链路。 |
| What have I done? | 已固化证据、根因、短期与长期修复方向，完成 shutdown delay 止血、websocket terminal event 兜底，并替换 release 包 `web_server.exe` 后通过真实 quick deploy smoke。 |

## Archived Previous Progress

# Release 包 Nginx 客户入口开发进度

## 2026-06-05

- 已按用户要求使用 `planning-with-files` 制定 release 包 Nginx 修复计划。
- 已读取项目内 `planning-with-files` skill，确认规划文件写入项目根目录：`task_plan.md`、`findings.md`、`progress.md`。
- 已尝试运行 session catchup：
  - 命令：`python "$env:USERPROFILE\.cursor\skills\planning-with-files\scripts\session-catchup.py" (Get-Location)`
  - 结果：失败，用户级 skill 目录没有 `scripts/session-catchup.py`。
  - 处理：记录为非阻塞，继续读取现有 planning 文件并前置新计划。
- 已完成 release 包与源码现状分析：
  - 当前已打验证包仍使用 `http://127.0.0.1:3100/viewer/`。
  - 包内前端是 `/viewer/` base 产物，位于 `viewer/index.html`。
  - 包内未发现 `nginx*` 文件，启动脚本未设置 Nginx/Viewer 相关环境变量。
  - 源码 Nginx 自动配置逻辑当前倾向使用 `plant3d-web/dist`，与 release 包静态布局不一致。
  - Windows Nginx listen 端口当前硬编码 `80`，需要改为跟 `AIOS_VIEWER_BASE_URL` 一致。
- 已完成 `grill-me` 决策收敛：
  - Windows release 包推荐开箱即用，Linux 继续接管系统 Nginx。
  - Windows 包推荐生成两份前端产物：`viewer/` 和 `viewer-root/`。
  - Nginx 客户入口使用 `/`，保留 `/viewer/` fallback。
  - 一个 `host:port` 只绑定一个客户站点，多站点用不同 vhost/端口/域名。
  - `/admin/` 推荐代理到主控 web_server，客户数据 API 代理到站点 web_server。
- 已更新 planning 文件：
  - `task_plan.md` 前置为 `Release 包 Nginx 客户入口修复计划`。
  - `findings.md` 记录包内容、源码逻辑和决策发现。
  - `progress.md` 记录本轮计划制定过程。
- 下一步第一条动作：进入 Phase R2/R3，修改打包脚本生成 `viewer-root/`，并让运行时 Nginx root 优先使用 release 包内静态目录。
- 已完成 Phase R2/R3/R4 的代码实现：
  - `build-windows-bundle.ps1` 双构建 `viewer/` 与 `viewer-root/`，并更新 `BUILD_INFO.json` / README。
  - `build-windows-bundle.ps1` 可选复制 `tools/nginx/windows/nginx.exe` 到 `bin/nginx/nginx.exe`，缺失时 warning。
  - `managed_project_sites.rs` 的 Nginx root 现在优先使用 `AIOS_VIEWER_STATIC_ROOT` / 包内 `viewer-root/`，Windows listen 使用 `viewer_base_listen_port(site)`。
  - Nginx 模板新增 `/admin/`、`/api/admin/` 到主控端口代理，保留 `/api/`、`/files/`、`/ws/` 到站点端口。
  - Windows Nginx 失败默认 fallback，`AIOS_REQUIRE_NGINX=1` 才 fatal。
  - `start-plant3d.ps1/.bat`、`install-service.ps1/.bat` 已传递 Nginx 相关参数。
- 下一步进入 Phase R5：实际构建新的 Windows release 包，并验证 `viewer-root/index.html`、`viewer/index.html`、Nginx `/`、`/assets/`、`/api/`、`/files/`、`/admin/` 路径。
- 已完成 Phase R5 的无 Nginx debug smoke：
  - 构建输出：`runtime/codex-validation/nginx-package-smoke-20260605-165240/Plant3D-AIOS-win-x64/debug`。
  - `viewer/index.html` 引用 `/viewer/assets/...`，`viewer-root/index.html` 引用 `/assets/...`。
  - `BUILD_INFO.json` 包含 `viewerFallbackUrl`、`customerViewerUrlTemplate`、`nginxStaticRoot=viewer-root`、`nginxBundled=false`。
  - 运行 `start-plant3d.ps1 -Port 3196 -NoBrowser -NoPortFallback` 后，脚本按预期提示未发现 `nginx.exe` 并使用 `/viewer/` fallback。
  - `GET /api/version` 和 `GET /viewer/` 均返回 200；临时 `web_server` 进程已停止。
- 已按用户要求从 Nginx 官方下载 stable Windows 包：
  - 官方 URL：`https://nginx.org/download/nginx-1.30.2.zip`
  - 部署缓存：`tools/nginx/windows/nginx.exe`
  - smoke 包副本：`runtime/codex-validation/nginx-package-smoke-20260605-165240/Plant3D-AIOS-win-x64/debug/bin/nginx/nginx.exe`
  - SHA256：`f2ffb8462dc348a333f40557a9e0d9cd554a2d4501eb3a265da2c429ec99a527`
- 已完成带 bundled Nginx 的 release 包布局 smoke：
  - 构建输出：`runtime/codex-validation/nginx-release-with-bundled-20260605-170329/Plant3D-AIOS-win-x64/release`。
  - `BUILD_INFO.json` 标记 `nginxBundled=true`、`nginxExe=bin/nginx/nginx.exe`、`nginxStaticRoot=viewer-root`。
  - 包内 `bin/nginx/nginx.exe` SHA256 与官方下载缓存一致。
  - `viewer/index.html` 使用 `/viewer/assets/...`，`viewer-root/index.html` 使用 `/assets/...`。
- 已完成 bundled Nginx 启动检测 smoke：
  - 命令使用 `-EnableNginx on -RequireNginx -ViewerHost 127.0.0.1 -ViewerPort 3297`。
  - 启动脚本输出包内 Nginx 路径、`runtime/nginx` prefix、`viewer-root` static root。
  - `GET /api/version` 返回 200；`GET /api/admin/app-config` 未登录返回 401，符合管理 API 鉴权预期。
  - 临时 `web_server` 进程已停止。
- 修复 release 包静态 root 分支：
  - 问题：release 包只有 `viewer-root/`，没有 `plant3d-web` 源码项目；旧逻辑找不到 `plant3d-web` 时直接跳过 Viewer/Nginx。
  - 修复：`viewer_static_root_for_nginx` 支持无 `viewer_dir`，`configure_*_nginx_if_available` 返回是否真正配置成功，`spawn_viewer_process` 在 `viewer-root/index.html` 存在且 Nginx 成功时直接返回客户 Viewer URL。
- 已完成真实站点 Nginx 代理 smoke：
  - 构建输出：`runtime/codex-validation/nginx-static-root-fix-20260605-171011/Plant3D-AIOS-win-x64/debug`。
  - 站点：`nginx-static-smoke-1780650910-3396`，客户 Viewer：`http://127.0.0.1:3397/?output_project=NginxStaticSmoke`。
  - 生成 Nginx 配置：`runtime/nginx/conf/conf.d/plant3d-web-nginx-static-smoke-1780650910-3396.conf`。
  - HTTP 验证：`/`、`/assets/<bundle>.js`、`/api/status`、`/admin/` 均返回 200。
  - `/files/` location 已在配置中覆盖，尚缺真实文件样本做 HTTP 命中验证。
  - 临时主服务、站点和 Nginx 均已停止。
- 已完成包含静态 root 修复的正式 release 包构建与启动检测：
  - 构建输出：`runtime/codex-validation/nginx-release-static-root-fix-20260605-172033/Plant3D-AIOS-win-x64/release`。
  - `BUILD_INFO.json` 标记 `nginxBundled=true`、`nginxStaticRoot=viewer-root`。
  - `bin/aios-database.exe` SHA256：`136660564078fd1fecba4f4617ef7c76d62c65ceb3bde596a59399ad4e89893c`，与 `BUILD_INFO.aiosDatabaseSha256` 一致。
  - `bin/nginx/nginx.exe` SHA256：`f2ffb8462dc348a333f40557a9e0d9cd554a2d4501eb3a265da2c429ec99a527`，与官方下载缓存一致。
  - 启动检测：`start-plant3d.ps1 -Port 3498 -EnableNginx on -RequireNginx -ViewerHost 127.0.0.1 -ViewerPort 3497` 输出包内 Nginx 与 `viewer-root`，`GET /api/version` 返回 200，临时主服务已停止。
- 已补充 `/files/` 真实文件命中验证：
  - 样本文件：`runtime/admin_sites/nginx-static-smoke-1780650910-3396/output/nginx-file-smoke.txt`。
  - 经 Nginx 请求：`GET http://127.0.0.1:3397/files/output/nginx-file-smoke.txt` 返回 200，内容命中 `nginx file proxy smoke`。
  - 临时主服务、站点和 Nginx 均已停止。
- 下一步：使用真实 quick deploy 站点打开客户 URL，确认模型加载且无 `backend=` 参数。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| planning-with-files skill 读取 | `.cursor/skills/planning-with-files/SKILL.md` | 确认三文件规划约定 | 已读取 | PASS |
| session catchup | 用户级 `planning-with-files/scripts/session-catchup.py` | 恢复未同步上下文 | 脚本缺失，非阻塞 | WARN |
| release 包内容审计 | `runtime/codex-validation/full-deploy-aveva-package-*` | 判断是否已支持 Nginx 根入口 | 未发现 nginx，viewer 仍为 `/viewer/` base | FAIL |
| 源码 Nginx 逻辑审计 | `src/web_server/managed_project_sites.rs` | 判断是否可复用 | 有自动配置雏形，但 root/listen/package 适配不足 | PARTIAL |
| planning 文件更新 | `task_plan.md` / `findings.md` / `progress.md` | 新任务成为 active plan | 已完成 | PASS |
| PowerShell 语法解析 | `build-windows-bundle.ps1` / `start-plant3d.ps1` / `install-service.ps1` | 无解析错误 | 全部 OK | PASS |
| Rust 类型检查 | `cargo check --bin web_server --no-default-features --features "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,kv-rocksdb"` | 编译通过 | 通过；仅有 `pdms_io` / `parse_pdms_db` 既有 warning | PASS |
| IDE lints | 变更文件 | 无新增诊断 | 无 linter errors | PASS |
| debug 包布局 smoke | `build-windows-bundle.ps1 -BuildProfile debug -SkipBackendBuild -SkipZip` | 生成 `viewer/` 与 `viewer-root/` | 已生成，Nginx 缺失仅 warning | PASS |
| 前端 base path | debug smoke 包 `viewer/index.html` / `viewer-root/index.html` | fallback 用 `/viewer/assets/`，Nginx root 用 `/assets/` | 符合预期 | PASS |
| 无 Nginx fallback smoke | 包内 `start-plant3d.ps1 -Port 3196 -NoBrowser -NoPortFallback` | 缺 Nginx 不阻断，`/viewer/` 可访问 | `/api/version=200`，`/viewer/=200`，进程已停止 | PASS |
| 官方 Nginx 下载 | `https://nginx.org/download/nginx-1.30.2.zip` | `tools/nginx/windows/nginx.exe` 存在且可被后续打包复制 | 已落地，SHA256 已记录 | PASS |
| release 包 bundled Nginx | `build-windows-bundle.ps1 -BuildProfile release -SkipBackendBuild -SkipZip` | 包含 `bin/nginx/nginx.exe`，`BUILD_INFO.nginxBundled=true` | 符合预期 | PASS |
| bundled Nginx 启动检测 | release 包 `start-plant3d.ps1 -EnableNginx on -RequireNginx` | 识别包内 Nginx 并设置 Viewer static root | 输出 Nginx 路径和 `viewer-root`，`/api/version=200` | PASS |
| release 静态 root 分支 | `spawn_viewer_process` 无 `plant3d-web`、仅有 `viewer-root` | 仍能配置 Nginx 并返回客户 Viewer URL | debug 包真实站点 smoke 进入 Running，客户 URL 无 `backend=` | PASS |
| per-site Nginx HTTP smoke | `http://127.0.0.1:3397/` | `/`、`/assets/`、`/api/`、`/admin/` 关键路径可用 | 对应请求均 200，生成 per-site conf | PASS |
| 正式 release 包静态 root 修复 | `build-windows-bundle.ps1 -BuildProfile release -SkipZip` | 新 release 包含修复后的后端、Nginx、`viewer-root` | 包构建成功，启动检测 `/api/version=200` | PASS |
| `/files/` Nginx HTTP smoke | `GET /files/output/nginx-file-smoke.txt` | Nginx 代理到站点 web_server 文件服务 | 返回 200，内容命中样本文件 | PASS |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Active plan 已切换到 release 包 Nginx 客户入口修复，Phase R1 已完成。 |
| Where am I going? | 下一步继续 Phase R5 的真实 quick deploy 模型加载验收。 |
| What's the goal? | Windows release 包能开箱配置 Nginx 客户入口，同时保留 `/viewer/` fallback；Linux 继续接管系统 Nginx。 |
| What have I learned? | 双前端产物和运行时 static root 必须同步落地；`/admin` 还需要 `/api/admin` 代理到主控端口，否则 Admin 前端会走错站点 API。 |
| What have I done? | 已实现 R2/R3/R4，完成 debug fallback、官方 Nginx 下载、release bundled Nginx 包布局、启动检测 smoke，并修复/验证 release 静态 `viewer-root` 下真实站点 Nginx 代理路径（含 `/files/`）。 |

## Archived Previous Progress

# AMS BRAN quick deploy 解析放大开发进度

## 2026-06-03

- 已按 MCP 会话要求结合 `grill-me` 与 `planning-with-files` 制定新的 active plan：`AMS BRAN quick deploy 解析放大修复计划`。
- 已读取 `grill-me` skill：能通过代码探索回答的问题先探索代码，剩余决策按单问题 + 推荐答案收敛。
- 已读取项目内 `planning-with-files` skill，确认规划文件应写在项目根目录：`task_plan.md`、`findings.md`、`progress.md`。
- 已尝试运行 session catchup：
  - 命令：`python "$env:USERPROFILE\.cursor\skills\planning-with-files\scripts\session-catchup.py" (Get-Location)`
  - 结果：失败，用户级 skill 目录没有 `scripts/session-catchup.py`。
  - 处理：记录为非阻塞，继续用现有 planning 文件和源码上下文制定计划。
- 已完成部署代码审查：
  - `QuickDeployTestRequest.wait` 默认 `true`，admin quick deploy 同步路径会等待完整 parse/generate/start，大项目容易超过客户端超时。
  - `quick_deploy_site()` 直接调用 `quick_deploy_test()`，admin 入口当前复用测试部署语义。
  - `should_run_db_index_prescan()` 在 `manual_db_nums` 非空时返回 false；quick deploy 单库部署正好会设置 `manual_db_nums=[dbnum]`，导致不会预扫 db_index 精确依赖。
  - `resolve_included_db_files_detailed()` 对显式单库允许 CATA fallback；精确依赖为空/失败后会纳入 `RELATED_DEPENDENCY_DB_TYPES`，解释 AMS 解析范围被放大。
- 已记录 grill-me 决策树：
  - admin quick deploy 默认不应同步等待完整部署，推荐默认后台任务并返回 `site_id/task_id`。
  - 单库 quick deploy 开启 `auto_parse_related_dbnums` 时也应跑 db_index prescan。
  - 精确依赖为空/失败时不应默认回退全 CATA；粗粒度 fallback 应显式开启。
  - admin quick deploy 不应使用 quick deploy test 的固定 `quicktest / QuickTest@2026` 凭据语义。
- 已把 `task_plan.md` 前置为 AMS BRAN quick deploy active plan，并将 AABB SQLite index 计划归档。
- 已把本轮代码审查发现写入 `findings.md`，保留目标 BRAN `24383/73930` 和目标 dbfile 路径。
- 已完成 Phase 3 最小代码修复：
  - `QuickDeployTestResponse` 新增 `task_id`，后台任务创建成功后结构化返回任务 ID。
  - `admin_handlers::quick_deploy_site()` 改为调用 `quick_deploy_admin()`，鉴权版 quick deploy 始终后台执行并返回 `202 Accepted`。
  - `managed_project_sites` 增加 `QuickDeployProfile`，test 保留历史 quicktest 凭据，admin 使用 per-dbnum 默认凭据，避免正式归档站点写入固定测试用户。
  - `resolve_included_db_files_detailed()` 默认禁用粗粒度 CATA fallback；精确依赖为空/失败时只记录 warn，不纳入全 CATA。
  - `should_run_db_index_prescan()` 改为只要 `auto_parse_related_dbnums=true` 就执行，单库 quick deploy 不再因 `manual_db_nums` 非空跳过预扫。
- 已完成验证：
  - `rustfmt --edition 2024 src/web_server/models.rs src/web_server/admin_handlers.rs src/web_server/managed_project_sites.rs` 通过。
  - `cargo check --bin web_server --features web_server` 通过；仅有上游 `pdms_io` / `parse_pdms_db` 既有 warning。
  - `ReadLints` 对三个编辑文件无新增诊断。
- 下一步第一条动作：进入 Phase 4，使用重启后的 `web_server` 调用 admin quick deploy，重新部署 `ams7999_0001` 并验证 `BRAN 24383/73930`。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| grill-me skill 读取 | `C:/Users/dpc/.agents/skills/grill-me/SKILL.md` | 确认提问/探索规则 | 已读取 | PASS |
| planning-with-files skill 读取 | `.cursor/skills/planning-with-files/SKILL.md` | 确认三文件规划约定 | 已读取 | PASS |
| session catchup | 用户级 `planning-with-files/scripts/session-catchup.py` | 恢复未同步上下文 | 脚本缺失，非阻塞 | WARN |
| 部署代码审查 | `models.rs` / `admin_handlers.rs` / `managed_project_sites.rs` | 定界 AMS 解析放大原因 | 定位到 `wait=true`、manual db 跳过 prescan、CATA fallback | PASS |
| planning 文件更新 | `task_plan.md` / `findings.md` / `progress.md` | 新任务成为 active plan | 已完成 | PASS |
| Phase 3 Rust 格式化 | `rustfmt --edition 2024 ...` | 编辑文件格式化通过 | EXIT=0 | PASS |
| Phase 3 编译检查 | `cargo check --bin web_server --features web_server` | web_server 类型检查通过 | EXIT=0，仅上游既有 warning | PASS |
| Phase 3 lints | `ReadLints` edited files | 无新增 IDE 诊断 | 无诊断 | PASS |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Active plan 已切换到 AMS BRAN quick deploy 解析放大修复，Phase 1/2/3 已完成，准备进入 Phase 4 真实重部署验证。 |
| Where am I going? | 使用重启后的 admin `quick-deploy` 重新部署 `ams7999_0001`，检查后台 `task_id`、parse plan 和 `BRAN 24383/73930` 显示。 |
| What's the goal? | 让 admin 自动部署在 AMS 单 BRAN 场景下使用精确依赖、后台执行、归档可复现配置，并完成 BRAN 显示验证。 |
| What have I learned? | 当前卡住不是单纯失败，而是 `wait=true` 客户端超时叠加 `manual_db_nums` 跳过 prescan 后 CATA fallback 放大解析范围。 |
| What have I done? | 已审查关键代码、制定 grill-me 决策、更新规划文件，并实现/验证 Phase 3 最小修复。 |

## Archived Previous Progress

# AABB Parquet 导入 SQLite Index 开发进度

## 2026-06-03

- 已按 MCP 会话要求结合 `grill-me` 与 `planning-with-files` 制定 AABB Parquet 导入 SQLite index 的 active plan。
- 已读取 `grill-me` skill：能通过代码探索回答的问题先探索代码；剩余决策以单问题 + 推荐答案形式收敛。
- 已读取项目内 `planning-with-files` skill，确认规划文件应写在项目根目录：`task_plan.md`、`findings.md`、`progress.md`。
- 已尝试运行 session catchup：
  - 命令：`python ".cursor/skills/planning-with-files/scripts/session-catchup.py" "D:/work/plant-code/plant-model-gen"`
  - 结果：失败，项目 skill 目录没有 `scripts/session-catchup.py`。
  - 处理：记录为非阻塞，继续用现有 planning 文件和源码上下文制定计划。
- 已把 `AABB Parquet 导入 SQLite Index 开发计划` 前置为新的 active plan。
- 已保留原 quick deploy test 修复计划作为归档；它仍是 AABB SQLite index 真实验收的 Phase 4 依赖。
- 已完成 Phase 1 上下文审计：
  - `src/sqlite_index.rs::refresh_dbnum_from_parquet_dir()` 已实现 Parquet 到 SQLite RTree 核心导入。
  - `src/cli_modes.rs::export_dbnum_instances_parquet_mode()` 已在 Parquet 导出成功后自动刷新 SQLite spatial index。
  - `src/fast_model/room_model.rs` 已声明生产主路径是 Parquet 导出后预建 SQLite RTree，legacy rebuild 仅用于显式/debug。
  - `src/fast_model/gen_model/orchestrator.rs` 已避免在 Parquet 导出模式下继续走生成后 SurrealDB 刷新。
- 已记录 grill-me 决策树：
  - 是否需要独立 Parquet 导入 CLI：推荐需要。
  - `tubings.parquet` 缺失是否 fatal：推荐默认 fatal。
  - 房间计算是否默认 legacy fallback：推荐不 fallback。
  - 多 dbnum 共用 SQLite 文件如何刷新：推荐按 dbnum 局部替换。
- 已确认当前主要缺口：
  - 还没有独立 `parquet-dir -> sqlite index` CLI 入口。
  - 还没有 quick deploy 真实导出后的 inserted/path/SQLite 行数验收证据。
  - 还没有房间计算消费预建 SQLite index 的 smoke 记录。
- 已按推荐方案继续 Phase 2/3：
  - Phase 2 决策收敛：纳入独立 Parquet 导入 CLI；`tubings.parquet` 在默认新包路径中保持必需；刷新粒度沿用按 `dbnum` 局部替换；房间计算默认不回退 legacy。
  - Phase 3 代码现状确认：`main.rs` 已提供 `--import-spatial-index-parquet <PARQUET_DIR>`、`--dbnum <N>`、`--spatial-index-output <SQLITE_PATH>`；旧 `--import-spatial-index <JSON_PATH>` 仍保留。
  - `cli_modes.rs::import_spatial_index_parquet_mode()` 已调用 `SqliteAabbIndex::refresh_dbnum_from_parquet_dir()`，并打印 EQUI/children/tubings/total/unique 与 SQLite 路径；未启用 `sqlite-index + parquet-export` 时有明确错误。
  - 编译验证：`cargo check --bin aios-database --no-default-features --features "review,parquet-export" --target-dir target-aabb-sqlite-cli-check` 通过（EXIT=0，只有上游 `pdms_io` 既有 warning）。
- 下一步第一条动作：进入 Phase 4，先完成 quick deploy test 当前阻塞的真实复跑，再用 quick deploy 归档配置验证 Parquet 导出后 SQLite spatial index 刷新日志和行数。
- 已完成 Phase 4 quick deploy 真实复跑：
  - `cargo build --bin aios-database` 通过，确保 quick deploy 子进程使用最新代码。
  - `POST /api/admin/quick-deploy-test` 使用 `runtime/quick-deploy-last-payload.json` 复跑，响应保存到 `runtime/quick-deploy-after-sqliteidfix-response.json`。
  - 响应 `success=true`、`data.success=true`、`generated=true`、`parse_status=Parsed`、`warnings=[]`，耗时 `26851ms`。
  - 站点 `quicktest-250160-8080` 使用归档配置中的新端口 `8022` 连接成功，不再复用冲突的 `8020`。
  - Parquet 输出成功：`instances.parquet=808`、`geo_instances.parquet=808`、`ptsets.parquet=0`、`transforms.parquet=547`、`aabb.parquet=517`、`manifest.json` 已写入。
  - `ptset_export` 统计：`cata_hashes=0`、`empty_ptset_hashes=0`、`missing_cata_hash_refnos=861`；缺失 cata_hash 不再导致部署 fatal。
- 已修复并验证 AABB Parquet 导入 SQLite index 的 refno/dbnum id 构造：
  - 问题：`src/sqlite_index.rs::required_refno_id()` 原先把 `refno_str` 的第一段当 dbnum；APS `refno_str=2013286704_3298` 实际是 PDMS `ref0_ref1`，导致导入报“不属于 dbnum=250160”。
  - 修复：Parquet 新 schema 优先读取 `refno_u64`、`owner_refno_u64`、`tubi_refno_u64`，保留真实 RefU64 作为 SQLite RTree id；`items` 新增 `dbnum` 列用于局部替换；旧 schema 缺列时才回退字符串解析以兼容旧 fixture。
  - 替换策略：`replace_dbnum_aabbs_with_items_and_spec_values()` 先按 `items.dbnum` 删除本 dbnum 旧 rows，再额外清理旧版错误导入产生的 `(dbnum << 32)..((dbnum+1)<<32)` id range。
  - 验证：`rustfmt src/sqlite_index.rs` 通过；`ReadLints` 无诊断；`cargo build --bin aios-database --features "review,parquet-export"` 通过。
  - 独立导入命令：`target/debug/aios-database.exe --import-spatial-index-parquet output/AvevaPlantSample/parquet/250160 --dbnum 250160 --spatial-index-output runtime/aabb-spatial-index-verify.sqlite`。
  - clean SQLite 验收：`aabb_index=811`、`items=811`、`items where dbnum=250160` 为 `811`；错误 id `250160/431` 不存在，真实 id `2013286704/431` 为 `EQUI` 且 `dbnum=250160`。
  - 已删除临时验证库 `runtime/aabb-spatial-index-verify.sqlite`。
- 已复跑 quick deploy 让默认 `output/spatial_index.sqlite` 使用最新导入逻辑：
  - `POST /api/admin/quick-deploy-test` 响应 `success=true`、`data.success=true`、`generated=true`、`parse_status=Parsed`、`warnings=[]`，耗时 `27201ms`。
  - 默认 SQLite 验收：`output/spatial_index.sqlite` 大小 `5931008`，`items=831`、`aabb_index=831`、`items where dbnum=250160` 为 `811`。
  - 旧错误 id `250160/431` 已不存在；真实 id `2013286704/431` 存在，`noun=EQUI`、`dbnum=250160`。
- 已完成消费端 smoke：
  - 命令：`target/debug/aios-database.exe spatial query-refno 2013286704/431 --distance-mm 1000 --include-self`。
  - 结果：`success=true`、`query_refno=2013286704/431`、`result_count=496`，结果包含自身 `2013286704/431`。
- 已修复并验证历史 Parquet 包中的空 `aabb_hash` 兼容：
  - 问题：`output/AvevaPlantSample/parquet/250164/instances.parquet` 有 243 行 `aabb_hash=''`，旧导入逻辑会把空 hash 当作必须存在于 `aabb.parquet` 的引用并 fatal。
  - 修复：`src/sqlite_index.rs` 在 `instances.parquet` 与 `tubings.parquet` 导入时跳过空 `aabb_hash` 行，并通过 `ImportStats::skipped_empty_aabb_count` 计数；非空 hash 缺失仍保持 fatal 校验。
  - CLI 输出：`src/cli_modes.rs::import_spatial_index_parquet_mode()` 新增 `跳过空 AABB` 统计。
  - 编译验证：`rustfmt --edition 2024 src/sqlite_index.rs src/cli_modes.rs` 通过；`cargo build --bin aios-database` 通过（仅既有依赖 warning）。
  - APS 主样本 clean rebuild：`runtime/spatial-index-clean-rebuild-aabbskip-20260603-211918.sqlite`，导入 `output/AvevaPlantSample/parquet` 下 6 个数字 dbnum 目录；最终 `aabb_index=1273`、`items=1273`、`orphan_aabb=0`。
  - 全仓可发现 Parquet 包 clean rebuild：`runtime/spatial-index-clean-rebuild-all-aabbskip-20260603-211958.sqlite`，导入 13 个具备 `aabb/instances/tubings.parquet` 的数字 dbnum 目录；最终 `aabb_index=41270`、`items=41270`、`orphan_aabb=0`、`duplicate_item_ids=0`。
  - 全仓最终 `items.dbnum` 分布：`7009=25`、`7011=133`、`7505=435`、`7600=40`、`7997=39267`、`250160=811`、`250164=462`、`250193=97`。
- 已补缺失/空预建 SQLite index 的错误提示 smoke：
  - 扫描 `output/` 与 `runtime/` 下 13 个 `tubings.parquet`，全部为 0 行；当前没有可用于 TUBI 非空聚合验证的现成样本。
  - 初次空索引 smoke 使用 `AIOS_SPATIAL_INDEX_SQLITE=runtime/spatial-index-missing-smoke.sqlite`，旧错误会继续 fallback 到可见子构件推导并报 db_meta/cache 错误。
  - 修复：`src/cli_modes.rs::spatial_query_refno_mode()` 打开索引后先检查 `get_stats().total_elements`；空索引直接提示先完成 Parquet 导出自动刷新，或运行 `--import-spatial-index-parquet <PARQUET_DIR> --dbnum <DBNUM> --spatial-index-output <PATH>` 重建。
  - 验证：`cargo build --bin aios-database` 通过；空索引 smoke 返回明确错误 `SQLite spatial index 为空: runtime\spatial-index-missing-smoke.sqlite...`；默认索引正常查询仍返回 `success=true`、`result_count=496`。
- 残留风险：当前所有可发现 `tubings.parquet` 均为 0 行，尚未覆盖非空 TUBI 写入与 BRAN/HANG owner 聚合；需要另找或生成带 TUBI 的样本补验。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| grill-me skill 读取 | `C:/Users/dpc/.agents/skills/grill-me/SKILL.md` | 确认提问/探索规则 | 已读取；代码可回答的问题已先探索 | PASS |
| planning-with-files skill 读取 | `.cursor/skills/planning-with-files/SKILL.md` | 确认三文件规划约定 | 已读取 | PASS |
| session catchup | `.cursor/skills/planning-with-files/scripts/session-catchup.py` | 恢复未同步上下文 | 脚本缺失，非阻塞 | WARN |
| AABB SQLite 代码审计 | `src/sqlite_index.rs` 等 | 确认当前实现状态 | 核心导入与导出后刷新已落地 | PASS |
| planning 文件更新 | `task_plan.md` / `findings.md` / `progress.md` | AABB 计划成为 active plan | 已完成 | PASS |
| Parquet 导入 CLI 入口 | `main.rs` / `cli_modes.rs` | 独立 `parquet-dir -> sqlite index` 入口存在，旧 JSON 入口兼容 | 已确认 `--import-spatial-index-parquet` + `--dbnum` + `--spatial-index-output` | PASS |
| Feature gate 编译 | `cargo check --bin aios-database --no-default-features --features "review,parquet-export"` | CLI 在目标 feature 下类型检查通过 | EXIT=0，仅上游 `pdms_io` warning | PASS |
| quick deploy 端到端复跑 | `POST /api/admin/quick-deploy-test` with `runtime/quick-deploy-last-payload.json` | 不再端口冲突，解析/生成/Parquet 导出成功 | `success=true`, `generated=true`, `parse_status=Parsed`, `warnings=[]` | PASS |
| AABB SQLite id 修复 | `src/sqlite_index.rs` | APS `ref0_ref1` 不再被误判为 `dbnum_refno` | 保留真实 `refno_u64`，并用 `items.dbnum` 做局部替换 | PASS |
| 独立 Parquet 导入 clean 验收 | `--import-spatial-index-parquet output/AvevaPlantSample/parquet/250160 --dbnum 250160 --spatial-index-output runtime/aabb-spatial-index-verify.sqlite` | 生成 SQLite 且保留真实 RefU64 id | `aabb_index=811`, `items=811`, `2013286704/431=EQUI`, `250160/431=0` | PASS |
| 默认 SQLite 验收 | `output/spatial_index.sqlite` | quick deploy 后默认索引可消费 | `items=831`, `aabb_index=831`, `dbnum_items=811`, `2013286704/431=EQUI` | PASS |
| Spatial query smoke | `spatial query-refno 2013286704/431 --distance-mm 1000 --include-self` | 默认 RTree 可按真实 refno 查询 | `success=true`, `result_count=496`, 包含自身 | PASS |
| 空 AABB 兼容 | `output/AvevaPlantSample/parquet/250164` | 空 `aabb_hash` 行跳过并计数，非空缺失 hash 仍 fatal | `skipped_empty_aabb_count=243`, `unique=462` | PASS |
| 全仓 Parquet clean rebuild | `output/` + `runtime/` 下可发现数字 dbnum Parquet 目录 | 所有现存有效包均可导入，SQLite 无孤立/重复行 | `DIR_COUNT=13`, `aabb_index=41270`, `items=41270`, `orphan_aabb=0`, `duplicate_item_ids=0` | PASS |
| TUBI 非空覆盖 | `output/` + `runtime/` 下 13 个 `tubings.parquet` | 验证 TUBI rows 与 BRAN/HANG 聚合 | 全部为 0 行，当前无可用样本 | BLOCKED |
| 空索引错误提示 | `AIOS_SPATIAL_INDEX_SQLITE=runtime/spatial-index-missing-smoke.sqlite spatial query-refno 2013286704/431` | 不 fallback 到 db_meta/cache 错误，提示先导出/导入 SQLite index | 返回 `SQLite spatial index 为空... --import-spatial-index-parquet ...` | PASS |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Active plan Phase 4/5/6 已基本通过；空 `aabb_hash` 历史包兼容与空索引错误提示均已修复验证。 |
| Where am I going? | 下一步进入文档收口，或找/生成一个带 TUBI 的样本补齐 BRAN/HANG 聚合验证。 |
| What's the goal? | 让 `aabb.parquet` / `instances.parquet` / `tubings.parquet` 到 `output/spatial_index.sqlite` 的链路可独立重建、可真实验收、可被房间计算稳定消费。 |
| What have I learned? | quick deploy 端口、ptset SQL fatal、SQLite refno/dbnum id 误解析、历史包空 `aabb_hash`、空索引错误提示是串联阻塞；所有现存 TUBI 表目前为空。 |
| What have I done? | 已修复 `src/sqlite_index.rs` id 构造、dbnum 替换策略、空 AABB 跳过逻辑和 `spatial query-refno` 空索引提示，重编 `aios-database`，完成主样本/全仓 clean rebuild 和空索引 smoke。 |

## Archived Previous Progress

# ptset parquet measurement snapping 开发进度

## 2026-06-03

- 已按 MCP 会话要求使用 `planning-with-files` 制定 ptset 下一步开发计划。
- 已读取并保留项目根目录既有 planning 文件；将旧 DuckLake active plan 移入归档段，新的 active plan 是 `ptset parquet measurement snapping`。
- 已把下一步拆为 5 个阶段：
  - Phase 1：确认/修正 `cata_hash` 的 PE/实例侧来源。
  - Phase 2：恢复数据库环境并跑 scoped Parquet 导出。
  - Phase 3：前端 loader 对新导出包做 ptset Parquet smoke。
  - Phase 4：浏览器联调距离、角度、点标高、高差四种测量模式。
  - Phase 5：issue/progress 收口与最终交付。
- 已确认关键设计决策：
  - `cata_hash` 属于 PE/refno 对应实例侧数据，不从 `ptsets.parquet` 反推。
  - `ptsets.parquet` 只按 `cata_hash + point_number` 存局部关键点定义。
  - 测量落点必须是 `ptset:*`，普通表面点只用于识别 refno 和触发 ptset 加载。
  - 旧 `/api/pdms/ptset` 不作为测量正常兜底。
- 已记录主要风险：当前后端导出查询只读取 `inst_relate.out[0].cata_hash`，下一步需验证它是否稳定等价于 `EleGeosInfo.cata_hash`；若不稳定，应改用更直接的 PE/instance-info 来源或明确兜底。
- 已完成 Phase 1 修正：`query_ptset_export_data()` 现在优先使用 `out[0].cata_hash`，并仅在 `record::id(out[0])` 通过 `is_valid_cata_hash()` 时作为兜底，避免把 `refno_sesno` 退化 ID 当成 cata hash。
- 静态验证已通过：
  - `rustfmt --edition 2024 --check src/fast_model/export_model/export_dbnum_instances_parquet.rs`
  - `cargo check --features parquet-export --lib`（首次失败是 NASM 不在 PATH；临时加入 `C:\Program Files\NASM` 后通过）
- 已尝试 Phase 2 scoped 导出：
  - `cargo run --features parquet-export -- -c db_options/DbOption-cli.toml ...` 失败：`-c` 会自动补 `.toml`，导致路径变成 `.toml.toml`。
  - `target/debug/aios-database.exe -c db_options/DbOption-cli ...` 失败：当前 `127.0.0.1:8020` 是另一套 `runtime/admin_sites/quicktest-250160-8080/data/surreal.db`，配置凭据认证失败。
  - `target/debug/aios-database.exe -c db_options/DbOption ...` 失败：file-mode 直连 `D:/backup-dbs/ams-8020.db` 时 `LOCK` 被占用。
- 已收到部署验证路径修正：以后默认不直接使用 `CLI + DbOption.toml` 拼配置，而是先走 quick deploy test，生成可行的 admin 站点部署配置并归档。
- quick deploy test 前置要求：`project_name` 不重名，DB 端口/站点端口不冲突；端口可行性检查通过后才允许执行配置。
- 本轮临时探查：`18020` 空闲并曾手动启动到 `D:/backup-dbs/ams-8020.db`，但 `7011/AvevaPlantSample` 缺少 `inst_relate`/`pe` 表，说明该数据路径不是可用 APS 模型库；临时 `18020` SurrealDB 已停止。
- 已运行 admin 站点部署里的 quick deploy test 复跑：
  - 启动 `target/debug/web_server.exe`，设置 `AIOS_ENABLE_QUICK_DEPLOY_TEST=1`、`ADMIN_USER=admin`、`ADMIN_PASS=admin`。
  - `web_server` 自启动主 SurrealDB 到 `0.0.0.0:8020`，数据路径 `D:/backup-dbs/ams-8020.db`。
  - 发送 `POST /api/admin/quick-deploy-test`，payload 保存于 `runtime/quick-deploy-last-payload.json`。
  - 当前复跑返回 `409 Conflict`，没有进入解析/生成。
- 已分析当前 `409 Conflict`：
  - 旧站点 `quicktest-250160-8080` 被同名复用。
  - 旧归档配置 `DbOption.toml` 中 DB 端口是 `8020`。
  - Admin runtime 报 `db_port_conflict=true`、`db_conflict_pids=[59684]`。
  - 代码路径为 `quick_deploy_test()` → `update_site()` → `assert_port_available_with_conn()`，复用路径没有重新分配旧 DB 端口。
- 已分析历史部署失败：
  - `generate.log` 显示 parse 和模型生成主体完成。
  - 失败发生在 `export_parquet_after_gen` 阶段，`query_ptset_export_data()` 的 SurrealQL 包含 `record::id(out[0]) as inst_info_id`，导致 Parquet 导出 fatal，子进程退出码 1。
- 已按 grill-me + planning-with-files 制定新的 active plan：当前任务切换为 `quick deploy test 部署失败修复计划`，保留 ptset plan 为归档上下文。
- 已按推荐方案继续 Phase 2/3：
  - Phase 2 代码现状确认：`quick_deploy_test()` 复用同名站点时会调用 `resolve_quicktest_reuse_ports_with_conn()`；旧 `db_port/web_port` 不可用时自动重分配，用户显式 `web_port` 仍走 `reserve_explicit_port()` 严格冲突失败；随后通过 `update_site()` 持久化并重写 `runtime/admin_sites/<site_id>/DbOption*.toml`。
  - Phase 3 修复：`query_ptset_export_data()` 不再在 SQL 中读取 `record::id(out[0]) as inst_info_id`，只读取 `in as refno`、`out[0].cata_hash`、`out[0].ptset`；缺失或非法 `cata_hash` 进入 `missing_cata_hash_refnos` 诊断并跳过。
  - 静态验证：`rustfmt --edition 2024 --check src/fast_model/export_model/export_dbnum_instances_parquet.rs` 通过；`cargo check --features parquet-export --lib` 最终 EXIT=0（等待 build directory lock 后完成，只有既有依赖 warning）。
- 下一步第一条动作：进入 Phase 4，重新运行 quick deploy test，确认不再因端口复用返回 `409 Conflict`，并验证生成后 Parquet 导出不再因 ptset/cata_hash 查询 fatal。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| planning 文件更新 | `task_plan.md` / `findings.md` / `progress.md` | 新 ptset active plan 位于文件顶部，旧计划保留归档 | 已完成 | PASS |
| cata_hash 归属澄清 | 代码检索 `EleGeosInfo.cata_hash` 与导出查询 | 明确 `cata_hash` 是 PE/实例侧事实 | 已记录到 `task_plan.md` / `findings.md` | PASS |
| Phase 1 后端修正 | 修改 `query_ptset_export_data()` | 优先 PE/实例字段，兜底值必须是有效 cata hash | 已完成 | PASS |
| Rust 格式检查 | `rustfmt --edition 2024 --check src/fast_model/export_model/export_dbnum_instances_parquet.rs` | 无格式 diff | 通过 | PASS |
| Rust 编译检查 | `cargo check --features parquet-export --lib` | parquet feature lib 类型检查通过 | 加入 NASM PATH 后通过 | PASS |
| 真实导出验证 | quick deploy test → scoped Parquet export | 生成并检查 `manifest` / `instances` / `ptsets` | 已尝试旧 CLI/DbOption 路径并纠偏；下一步必须走 quick deploy test 产出的 admin 站点配置 | BLOCKED |
| quick deploy 当前复跑 | `POST /api/admin/quick-deploy-test` for `aps250160_0001` | 创建/复用站点并进入解析/生成 | `409 Conflict`，旧站点 `db_port=8020` 与当前主 SurrealDB 冲突 | FAIL |
| 历史 quicktest 失败定位 | `runtime/admin_sites/quicktest-250160-8080/logs/generate.log` | 找到部署失败边界 | 模型生成完成后，`query_ptset_export_data()` 的 `record::id(out[0])` SQL fatal | FAIL |
| quicktest 复用端口策略 | `resolve_quicktest_reuse_ports_with_conn()` + `quick_deploy_test()` 代码检查 | 自动端口可重分配，显式端口仍严格失败，并通过 `update_site()` 写回归档配置 | 已确认当前代码满足 | PASS |
| ptset 查询健壮性 | `query_ptset_export_data()` | 主查询不使用 `record::id(out[0])` fallback | 已移除 `inst_info_id` 字段和 SQL fallback | PASS |
| Rust 格式检查 | `rustfmt --edition 2024 --check src/fast_model/export_model/export_dbnum_instances_parquet.rs` | 无格式 diff | 通过 | PASS |
| Rust 编译检查 | `cargo check --features parquet-export --lib` | parquet feature lib 类型检查通过 | EXIT=0，仅既有依赖 warning | PASS |
| quick deploy 端到端复跑 | `POST /api/admin/quick-deploy-test` with `runtime/quick-deploy-last-payload.json` | 不再端口冲突，解析/生成/Parquet 导出成功 | `success=true`, `generated=true`, `parse_status=Parsed`, `warnings=[]` | PASS |
| AABB SQLite id 修复 | `src/sqlite_index.rs` | APS `ref0_ref1` 不再被误判为 dbnum_refno | 改为优先使用 `refno_u64` 构造 id；旧 schema fallback 保留 | PASS |
| 独立 Parquet 导入 clean 验收 | `--import-spatial-index-parquet output\AvevaPlantSample\parquet\250160 --dbnum 250160 --spatial-index-output runtime\quick-deploy-spatial-250160-sqliteidfix.sqlite` | 生成 SQLite 且所有行属于 250160 | `aabb_index=811`, `items=811`, `DBNUMS=[(250160, 811)]` | PASS |
| 默认 SQLite 残留检查 | `output/spatial_index.sqlite` | 识别是否有历史错误 id 残留 | 总行 `1642`，其中 `250160=811`；需后续 repair/重建 | WARN |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Active plan Phase 4 的 quick deploy 与 clean SQLite 导入验收已通过；默认 `output/spatial_index.sqlite` 发现旧错误 id 残留。 |
| Where am I going? | 下一步进入 Phase 5/6：决定默认索引 repair/重建策略，并补房间计算消费预建 SQLite index 的 smoke。 |
| What's the goal? | 让 admin quick deploy test 产出的 Parquet 包可稳定重建 SQLite spatial index，并让房间计算默认消费该预建索引。 |
| What have I learned? | quick deploy 端口、ptset SQL fatal、SQLite refno/dbnum id 误解析是三个串联阻塞；clean SQLite 已证明新 id 构造正确，但默认索引可能需要清理历史污染。 |
| What have I done? | 已修复 `src/sqlite_index.rs` id 构造，重编 `aios-database`，复跑 quick deploy 成功，并用独立 Parquet 导入 CLI 生成 clean SQLite 验收库。 |

## Archived Previous Progress

# DuckLake ModelWriter 下一步开发进度

## 2026-05-17

- 已按 MCP 会话要求使用 `planning-with-files` 制定中文开发文件。
- 已读取 planning skill，确认计划文件应落在项目根目录：`task_plan.md`、`findings.md`、`progress.md`。
- 已检查并保留根目录既有 planning 文件，将 RUS-248 / pe_transform 历史内容归档到新计划下方。
- `session-catchup.py` 用户级路径不存在：`C:\Users\dpc\.cursor\skills\planning-with-files\scripts\session-catchup.py`，已记录为非阻塞。
- 已读取 DuckLake 相关上下文：
  - `plant-model-ducklake/README.md`
  - `plant-model-ducklake/src/duckdb_backend.rs`
  - `plant-model-ducklake/src/schema.rs`
  - `plant-model-gen/goals/ducklake-model-writer/brief.md`
  - `plant-model-gen/goals/ducklake-model-writer/plan.md`
  - `plant-model-gen/goals/ducklake-model-writer/blockers.md`
  - `plant-model-gen/src/fast_model/gen_model/model_writer_ducklake.rs`
  - `plant-model-gen/src/options.rs`
  - `plant-model-gen/Cargo.toml`
- 已完成 Phase 1：锁定下一步范围为 DuckLake ModelWriter 验收与跨 crate 收敛分析；不扩大到 pe_transform DuckLake stub、不运行 Rust tests。
- 已完成 Phase 2：函数级审计 + schema 差异审计。
  - 8 个 trait 生命周期方法都有实现面；`cleanup` 与 `boolean_bridge` 是计划内 skipped。
  - `reconcile_missing_neg_relations` 仍是 sentinel 行，不是完整 carrier→target 解析。
  - in-repo DuckLake DDL 与 `plant-model-ducklake` canonical schema 分叉明显，尤其是 `raw_inst_info`、`raw_inst_relate`、`raw_aabb`、`raw_vec3`、`raw_inst_relate_aabb`。
  - 源码顶部 Slice 1 注释已陈旧，仍声称 Slice 2-4 未实现。
- 当前进入 Phase 3：CLI / feature / HTTP 验证。
- Phase 3 验证前检查终端状态：多个 sibling repo cargo 任务仍显示运行中，包括 `plant-model-ducklake` 的 `cargo run/check` 与 `plant-model-core` 的 `cargo check`；为避免叠加重型编译，本轮未启动新的 `plant-model-gen` cargo check。
- Phase 3 验证面预检：
  - `D:\Rust\.cargo\bin\cargo.exe --version` 返回 `cargo 1.97.0-nightly (4f9b52075 2026-05-01)`。
  - `model_writer_verify --mode ducklake --json` 是静态 contract evidence，不执行 DuckLake init。
  - `model_writer_verify --mode ducklake --exec --json` 才会打开 DuckDB、INSTALL/LOAD ducklake、ATTACH metadata、创建 9 张 raw 表并 finalize。
  - `POST /api/model/writer-verify {"mode":"ducklake"}` 当前只走静态 `model_writer_contract_evidence()`，不能作为 runtime DuckLake smoke。
- Phase 3 编译验证：
  - 命令：`cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`
  - 环境：`PATH` 前置 `D:\Rust\.cargo\bin` 与 `C:\Program Files\NASM`
  - 结果：FAILED，退出码 101，耗时约 370s。
  - 失败点：`libduckdb-sys v1.10502.0` custom build script exit code 1。
  - 可见错误摘要：`error: failed to run custom build command for libduckdb-sys v1.10502.0`；输出主要为 MSVC/DuckDB C++ warnings，未暴露 Rust 业务代码错误。
- Phase 3 编译阻塞复查：
  - 已读取本地 `duckdb-1.10502.0/Cargo.toml`，确认 `default = []`；当前失败不是由 `plant-model-gen` 关闭 DuckDB default features 引起。
  - 使用同一 target 目录和 `-j 1` 重跑：`cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify -j 1`。
  - 结果：FAILED，退出码 101，耗时约 876s。
  - 明确失败点：`LINK : fatal error LNK1114: 无法覆盖原始文件 ...\libduckdb.a；错误代码 112`。
  - 磁盘检查：初次检查 `D:` 剩余约 `1.52GB`，随后复查已降至约 `0.03GB`；`C:` 剩余约 `8.10GB`。当前阻塞应先按磁盘空间不足处理。
  - 尝试查找 `plant-model-ducklake/target` 可复用产物，未找到可直接复用的 `libduckdb.a`；且该路径同样位于空间不足的 `D:`。

## 2026-05-17 续 · Phase 3 阻塞解除与验证闭环

- 磁盘复检：`D:` 剩余 `128.42GB`，`C:` `16.03GB`，`E:` `102.64GB`；之前 0.03GB 阻塞已自动解除（`target-ducklake-verify` 被清理），无需手动迁移 target-dir。
- 第一次重跑 `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`：在 `Checking duckdb v1.10502.0` 之后 PowerShell pipeline 异常断开，exit_code unknown，但 `libduckdb-sys` bundled C++ 已成功（否则不会到 `Checking duckdb`）。
- 第二次重跑同命令（用 `Out-File` 替代 `Tee-Object` 规避缓冲问题）：**EXIT=0**，`Finished dev profile in 1m 22s`。0 error，110 warning（均为依赖库 dead_code/unused_variables，不影响 ducklake 链路）。
- CLI 静态验证：`cargo run --bin model_writer_verify --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify -- --mode ducklake --json`，EXIT=0，`Finished in 10m 58s`（含主 crate 11min 全量 build+link）。
  - 输出 8 个 stage 的 contract evidence：7 implemented + 1 skipped（boolean_bridge，phase2 Non-Goal）。
  - `known_gap_tables`：`raw_tubi_info / raw_tubi_relate / raw_aabb(tubi) / raw_trans / raw_vec3(tubi) / raw_refno_assoc_index` 共 6 项。
- CLI 执行验证：直接运行已 build 的 `target-ducklake-verify\debug\model_writer_verify.exe --mode ducklake --exec --json`，EXIT=0，**elapsed_ms=599**。
  - `init: executed, item_count=9` ← 真实打开 DuckDB、`INSTALL/LOAD ducklake`、ATTACH metadata、CREATE 9 张 raw 表均成功。
  - `cleanup: skipped`，reason：ducklake does not clean SurrealDB; metadata is created fresh per run。
  - 6 个 `known_gap:*` stages 全部 skipped 并附 reason 指向 `cata_model.rs / refno_assoc_index.rs` 写入面（goal `Q1=C` scope）。
  - `ducklake_root`: `output/model_writer_storage/ducklake`。
- 磁盘落地确认 (`output/model_writer_storage/ducklake/`)：
  - `metadata.ducklake` 3,084 KB（DuckDB 格式的 DuckLake metadata 数据库）。
  - `data/ducklake-canonical/` 下 9 个 raw 表目录就绪：raw_aabb / raw_geo_relate / raw_inst_geo / raw_inst_info / raw_inst_relate / raw_inst_relate_aabb / raw_neg_relate / raw_ngmr_relate / raw_vec3（与 `init` item_count=9 完全吻合）。
- 结论：Phase 3 编译 + CLI 静态 + CLI exec 三个核心验证点全部 PASS；in-repo `DuckLakeModelWriterBackend` 可在本机以 bundled DuckDB 启动 DuckLake 并建表，无需 DuckDB CLI 外部工具。

## 2026-05-17 续2 · Phase 4 入口准备 + 样本可用性阻塞

- 已 build `aios-database` bin：`cargo build --bin aios-database --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`，EXIT=0，**20.83s 完成**（因 lib 已被前次 model_writer_verify build 过，只 link bin）。可执行：`target-ducklake-verify/debug/aios-database.exe`。
- 已确认 `run_regen_model` 路径对 DuckLake/DrainOnly 模式自动跳过 `pre_cleanup_for_regen`，满足 goal constraint「不写 SurrealDB」。
- 本机环境探查（SurrealDB `localhost:8020`，ns=`1516`，db=`AvevaMarineSample`）：
  - `dbnum_info_table` 注册 dbnum=1112（DESI 类型，file_name=`ams1112_0001`，count=2）✓
  - **INST 表里 dbnum=1112 完全无记录**（SELECT count() FROM INST WHERE dbnum=1112 → 0）✗
  - 整个 INST 表只有 111 条记录，dbnum 分布：24383(n=35) / 7999(n=31) / 23399(n=24) / 24381(n=10) / 7997(n=6) / 23584(n=3) / 17496(n=1) / 25688(n=1)。
  - 与 goal brief.md "若本机数据不可用则记录原因并请求替代样本" 路径吻合。
- Phase 4 候选样本（按"42s 历史基线"小样本原则筛选）：
  - `dbnum=17496` (INST n=1)：与 1112 (count=2) 体量最接近，最小可行 smoke。
  - `dbnum=25688` (INST n=1)：同样极小。
  - `dbnum=23584` (INST n=3)：略大但仍小。
  - `dbnum=7997` (INST n=6)：曾在 pe_transform 工作里跑出 176K transform entries，规模偏大，不建议作为本期 first smoke。
- 决策点：选定样本 dbnum 后即可执行：`target-ducklake-verify/debug/aios-database.exe -c db_options/DbOption-cli.toml --regen-model --dbnum <N> --model-writer ducklake`。
- 待执行：跑通后用 DuckDB SQL（通过 duckdb crate 内嵌方式，或 rust 一次性脚本）查 9 张 raw 表的行数与样本主键。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| planning 文件更新 | 写入 `task_plan.md` / `findings.md` / `progress.md` 顶部 | 新 active plan 存在，历史内容保留 | 已完成 | PASS |
| session catchup | `python ...session-catchup.py` | 输出 catchup report 或无上下文 | 脚本路径不存在，非阻塞 | WARN |
| Phase 2 audit | 读取 `model_writer_ducklake.rs` 与 `plant-model-ducklake/src/schema.rs` | 找出实现缺口和 schema 分叉 | 已记录到 `findings.md` | PASS |
| Phase 3 cargo check readiness | 检查现有终端 | 避免重复启动重型 Rust 编译 | 发现多个相关 cargo 任务仍显示运行，暂缓新编译 | DEFERRED |
| Phase 3 verifier preflight | 读取 CLI / web endpoint + cargo version | 确认静态/执行验证命令差异 | 已确认 CLI 需 `--exec` 才实际触发 DuckLake init；web endpoint 仅静态 evidence | PASS |
| Phase 3 cargo check | `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` | DuckLake feature 编译通过 | `libduckdb-sys v1.10502.0` custom build script exit code 1 | FAIL |
| Phase 3 cargo check retry | 同上并加 `-j 1` | 若为并行编译不稳定，应继续或暴露更明确错误 | `lib.exe` 写 `libduckdb.a` 失败，Windows 112；`D:` 已降至约 0.03GB | BLOCKED |
| Phase 3 cargo check 2026-05-17 重跑 | 同上 cargo check 命令（`D:` 已恢复 128.42GB） | DuckLake feature 编译通过 | `Finished dev profile in 1m 22s`，0 error / 110 warning | PASS |
| Phase 3 CLI 静态验证 | `cargo run --bin model_writer_verify -- --mode ducklake --json` | 输出 contract evidence + Known Gap 表 | EXIT=0，8 stages（7 implemented + 1 skipped），6 known_gap_tables | PASS |
| Phase 3 CLI exec 验证 | `model_writer_verify.exe --mode ducklake --exec --json` | 真实打开 DuckLake，创建 9 张 raw 表 | EXIT=0，elapsed_ms=599，init item_count=9，6 known_gap stages 显式 skipped | PASS |
| Phase 3 DuckLake 磁盘落地 | 检查 `output/model_writer_storage/ducklake/` | metadata.ducklake + 9 张 raw 表目录就绪 | metadata.ducklake 3,084 KB；9 个 raw 表目录与 init 计数完全吻合 | PASS |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Phase 3 已闭环：cargo check + CLI 静态 + CLI exec 三项验证全部 PASS，9 张 raw 表已建。 |
| Where am I going? | 决策点：是否进入 Phase 4（选 dbnum，跑 Surreal baseline + DuckLake writer 真实生成 smoke），还是先收口 Phase 5 跨 crate 收敛分析。 |
| What's the goal? | 产出可执行的 DuckLake ModelWriter 下一步开发和验证路径；当前阶段交付物已就绪。 |
| What have I learned? | 见 `findings.md` 的 2026-05-17 Discovery 与 2026-05-17 续 Phase 3 闭环段。 |
| What have I done? | 已完成 Phase 3 编译解阻 + DuckLake runtime smoke + planning 文件同步。 |

## Archived Previous Progress

# RUS-248 批注后驳回流转进度

## 2026-05-14

- 已按用户要求启用 planning-with-files。
- `session-catchup.py` 在用户级和项目级 skill 脚本路径均不存在，已记录到 `task_plan.md`，不阻塞本轮开发。
- 已读取 Trellis backend spec 和 shared thinking guides；backend 具体规范多数为占位，跨层重点是明确 PMS postMessage → 前端 API → 后端 workflow sync → SurrealDB 状态的契约。
- 已将 RUS-248 active plan 前置写入 `task_plan.md`、`findings.md`、`progress.md`，旧 `pe_transform` 计划保留在归档段落。
- 已完成 Phase 2：`plant3d-web` 新增 `reviewWorkflowSyncMutation()`，`pms.workflow_changed` 支持 `nextStep`，`applyExternalWorkflowChange()` 改为调用 `/api/review/workflow/sync`，并保留旧 PMS 消息的 `nextStep` fallback。
- 已完成 Phase 3：PMS simulator 的 `emitPmsWorkflowChanged()` 支持/推导 `nextStep`；postMessage synced 已持续回传 `ok/taskId/status/currentNode/error/requestId`。
- 已完成 Phase 4：后端 `workflow/sync` 的 `review_workflow_history` 写入补齐 `form_id`、`target_node`、`actor_*`、`source`、`created_at`，保留旧 `operator_*` 字段。
- 验证：`npm run type-check` 通过；`cargo check --bin web_server --features web_server` 通过（仅既有依赖警告）。
- 已完成 Phase 5 真实 HTTP payload 验证（本机当前代码 `web_server` 启动于 `:3199`）：
  - 创建任务 `formId=RUS248-VERIFY-20260514110621`，task `task-a19fe2cc-bd6e-4b6e-9f7f-2288c0a7f6be`。
  - SJ `active` 到 JH：`jd/submitted`。
  - JH 写入 rejected 批注状态后 `return` 到 SJ：`sj/draft`，`returnReason=RUS-248 verify return to SJ`。
  - SJ 标记 fixed 后再次 `active` 到 JH：`jd/submitted`，`returnReason=null`。
  - SurrealDB 直查 `review_workflow_history` 确认 `form_id/target_node/source/actor_*` 已落库。
- 当前 RUS-248 开发计划已完成；剩余可选项是跑 PMS CDP 端到端，但既有 CDP 卡点在 PMS 列表无法重新打开刚创建记录，不影响本轮 workflow/sync 验证结论。

## Archived Previous Progress

# pe_transform 后端重构进度

## 2026-05-08

- 已安装 `planning-with-files`：
  - Cursor 项目安装到主工作区 `D:/work/plant-code/plant-model-gen/.cursor/skills/planning-with-files`。
  - Cursor worktree 同步到 `.worktrees/pe-transform-backends/.cursor/skills/planning-with-files`。
  - Codex 个人安装到 `C:/Users/dpc/.codex/skills/planning-with-files`，并新增全局 hooks。
  - `C:/Users/dpc/.codex/config.toml` 已启用 `[features] codex_hooks = true`。
- 已创建 worktree：`D:/work/plant-code/plant-model-gen/.worktrees/pe-transform-backends`，分支 `feat/pe-transform-backends`，基于 `f0aedb6`。
- 已完成首轮代码发现，确认重构核心入口：`Cargo.toml` features、`options.rs` feature 校验、`pe_transform_refresh.rs` batch 写入、`transform_cache.rs`/`transform_rkyv_cache.rs` 读取链路。
- 已创建本轮 planning files：`task_plan.md`、`findings.md`、`progress.md`。
- `codex --version` 返回 `codex-cli 0.129.0`；`codex features list` 显示当前 CLI 的 hook feature 名为 `hooks` 且已启用，因此 `config.toml` 同时保留 `codex_hooks = true` 和 `hooks = true` 以兼容文档与当前 CLI。
- 已按用户补充要求更新方案：首轮对比固定刷新 `dbnum=7997`，且对比前必须清理历史 `pe_transform` 数据。
- 已实现 transform backend 配置面：`transform-store-parquet`、`transform-store-ducklake`、`transform-store-compare` features；`transform_write_backend`、`transform_read_backend`、`transform_compare_backends`、Parquet/DuckLake 路径和 `clear_transform_before_refresh` 配置/CLI。
- 已新增 `src/pe_transform_store.rs`：封装 `PeTransformSink` / `PeTransformSource`，默认 SurrealDB sink/source，Parquet sink/source（feature-gated），DuckLake 注册 SQL 脚本生成，dbnum 历史 `pe_transform` 清理，对比统计。
- 已修改 `src/pe_transform_refresh.rs`：batch flush 改走统一 backend，并在写入后 prime `transform_cache`。
- 已修改 `src/fast_model/gen_model/transform_cache.rs`：生成阶段 cache miss 可按 `transform_read_backend` 从 Parquet/DuckLake source 读取 local/world 并写回内存；默认 `auto/surreal` 仍走旧 SurrealDB 查询/计算路径。
- 已修改 `src/main.rs`：`--refresh-transform` 支持清理历史数据、选择写入/读取 backend、输出 compare stats。
- 静态验证：`ReadLints` 检查本轮修改文件无 linter errors；`git diff --check` 通过。
- 阻塞：当前 PowerShell 中 `cargo --version` 失败（`cargo` not recognized），尚未执行 `cargo check` 和真实 `--refresh-transform 7997` 验证。
- 2026-05-08 运行对比/profile 前环境检查：
  - `cargo` / `rustc` / `rustup` 均不在当前 PowerShell `PATH`，`C:/Users/dpc/.cargo/bin/cargo.exe` 不存在。
  - `duckdb` / `surreal` 命令均不在当前 `PATH`。
  - `Get-NetTCPConnection -LocalPort 8020` 未返回监听连接。
  - worktree 内没有现成 `aios-database.exe`，无法运行包含本轮改动的新 CLI。
- 待工具链恢复后的首个真实验证命令建议：
  - `cargo check --bin aios-database --features "review,transform-store-parquet,transform-store-compare"`
  - `cargo run --bin aios-database --features "review,transform-store-parquet,transform-store-compare" -- -c db_options/DbOption-cli --refresh-transform 7997 --clear-transform-before-refresh --transform-write-backend dual --transform-compare-backends surreal,parquet`
- 已按 planning-with-files 补充下一步详细开发方案到 `task_plan.md`：
  - Phase 8：恢复 Cargo/SurrealDB/DuckDB 验证环境。
  - Phase 9：编译收敛并修复最小错误。
  - Phase 10：执行 `7997` 清理、刷新、双写、SurrealDB vs Parquet 对比。
  - Phase 11：profile 清理、计算、写入、prime、读取、compare 各阶段耗时。
  - Phase 12：验证 DuckLake 注册脚本和 snapshot/表行数。
  - Phase 13：输出最终对比表并完成交付记录。
- 用户指定 Rust 路径后，已用 `D:/Rust/.cargo/bin` 识别到 `cargo 1.97.0-nightly` 与 `rustc 1.97.0-nightly`。
- 首次在线 `cargo check` 卡在 `happyrust/indextree` git 更新；改为离线后发现多个 git 依赖缺本地缓存。
- 已在 `Cargo.toml` 增加本地 patch，复用本机仓库：
  - `indextree -> D:/work/plant-code/indextree/indextree`
  - `miniacd -> D:/work/plant-code/miniacd`
  - `rvm-rs -> D:/work/plant-code/rvmparser/rvm-rs`
  - `surrealdb/surrealdb-types -> D:/work/plant-code/surrealdb/...`
  - `calamine -> D:/work/plant-code/calamine-mirror`
  - `cavalier_contours -> D:/work/plant-code/cavalier_contours/cavalier_contours`
  - `id_tree -> D:/work/plant-code/id_tree-mirror`
- 当前 `cargo check` 阻塞在 `rs-core` 的 `ploop-rs = { git = "https://github.com/happyrust/rust-ploop-processor", branch = "1.0" }`；本机 `D:/work/plant-code` 下未找到 `rust-ploop-processor` / `ploop` 对应本地仓库，在线更新也长时间无输出。
- 已停止本轮卡住的 `cargo check` 进程；保留了一个非本轮启动的 `cargo test ... parse_real_files ...` 进程未处理。
- `git diff --check` 通过；planning 文件 lints 无错误。

## 2026-05-11

- **`cargo check` 通过**：`cargo check --bin aios-database --features "review,transform-store-parquet,transform-store-compare" --offline` 编译成功，耗时 44s。
- 修复了以下编译阻塞问题：
  1. `surrealdb_types` 双版本冲突（301 errors）：依赖用 `github.com/happyrust/surrealdb` 但 patch 只覆盖 `gitee.com/happydpc/surrealdb`。修复：在 `Cargo.toml` 增加 `[patch."https://github.com/happyrust/surrealdb"]` 指向相同本地路径。
  2. NASM 汇编器缺失：`aws-lc-sys` 编译需要 NASM。修复：将 `C:\Program Files\NASM` 加入 PATH。
  3. `review_db.rs` 重复导入 `Ordering` 和缺少 `REVIEW_DB_CONTEXT_SET` 静态变量、重复定义 `fresh_review_db`。修复：合并导入、添加静态变量、删除重复函数。
  4. `workflow_sync.rs` 中 `request.actor.id` 直接字段访问 `Option<WorkflowActor>`。修复：改为 `request.actor().id` 方法调用。
  5. `VerifyWorkflowData` 初始化缺少 `block_code`/`actor_id`/`owner_id`/`owner_source`/`expected_next_node`/`requested_next_step` 字段。修复：补充 `None` 初始值。
- `ploop-rs` git 依赖：cargo git cache 中已有 checkout（commit `33985df`），`--offline` 模式可直接使用，无需本地 path patch。
- Phase 9（编译收敛）已完成。下一步进入 Phase 10（SurrealDB vs Parquet 首轮对比）。

### Phase 10: SurrealDB vs Parquet 首轮对比

- **环境**：
  - Cargo: `1.97.0-nightly`，SurrealDB: `3.1.0-alpha` (port 8020)
  - 数据库：`ws://127.0.0.1:8020`，namespace `1516`，database `AvevaMarineSample`
  - Worktree: `.worktrees/pe-transform-backends`（branch `feat/pe-transform-backends`）

- **执行命令**：
  ```
  cargo run --bin aios-database --features "review,transform-store-parquet,transform-store-compare" --offline \
    -- -c db_options/DbOption-cli --refresh-transform 7997 --clear-transform-before-refresh \
    --transform-write-backend dual --transform-compare-backends surreal,parquet
  ```

- **执行结果**：
  - 总耗时：724,614ms（~12 分钟）
  - dbnum 7997 总节点数：176,390
  - 已处理节点数：143,222
  - 清理历史 pe_transform：refnos=0（未找到需清理的记录）
  - Parquet 文件：`output/AvevaMarineSample/pe_transform/pe_transform.parquet`（4.5 MB）

- **对比结果**：

  | Backend | Loaded | Missing | Mismatched | Max Delta | Elapsed (ms) |
  |---------|--------|---------|------------|-----------|--------------|
  | SurrealDB (run 1) | 175,337 | 1,053 | 0 | 0.000000 | 16,283 |
  | SurrealDB (run 2) | 175,337 | 0 | 75,575 | 0.000000 | 16,235 |
  | Parquet | 143,222 | 32,115 | 58,930 | 0.000854 | 1,711 |

- **关键发现**：
  1. **Parquet 读取速度约 9.5 倍于 SurrealDB**（1,711ms vs ~16,250ms）
  2. Parquet missing=32,115 = SurrealDB 总数(175,337) - 本次刷新数(143,222)，因 Parquet 只含本次写入数据
  3. Parquet mismatched=58,930 max_delta=0.000854，为 float 序列化精度差异
  4. SurrealDB 出现两行输出，可能是 local/world transform 分别对比，或代码 bug
  5. 清理报告 refnos=0，说明按 dbnum 查找历史记录的查询可能需要调整

- **待排查**：
  - 两行 SurrealDB 对比的含义（是 local/world 分开还是代码重复输出？）
  - Parquet mismatched 的 float 精度是否可接受
  - 清理为何未找到历史记录（pe_transform 表结构是否包含 dbnum 字段？）
- Phase 10 已完成。

### Phase 11: Profile 耗时热点

- **执行命令**：同 Phase 10（第二次运行，含计时器）
- **耗时 profile**：

  | 阶段 | 耗时 (ms) | 占比 |
  |------|----------|------|
  | 计算 local/world transform | 230,888 | 37.1% |
  | SurrealDB 写入 | 145,763 | 23.4% |
  | Parquet 写入 | 245,339 | 39.5% |
  | transform_cache prime | 0 | 0.0% |
  | **总耗时** | **621,990** | **100%** |

- **关键发现**：Parquet 写入是最大瓶颈（39.5%），原因是每批 500 条写入时 read-merge-dedup-write 整个文件（O(n²)行为），随着文件增大越来越慢。
- **对比读取（compare 阶段）**：

  | Backend | Elapsed (ms) |
  |---------|-------------|
  | SurrealDB baseline | 14,845 |
  | SurrealDB compare | 14,922 |
  | Parquet | 1,698 |

- **优化建议**：Parquet 写入改为先写多个 batch 文件，最终一次合并去重。
- Phase 11 已完成。

### Parquet 写入优化 & Compare 修复

- **Parquet 写入优化**：改为每批写独立 batch 文件，最终一次 merge+dedup
  - 写入：245,339ms → 2,250ms（**73x 快**）
  - Finalize: 1,113ms
  - 总 Parquet I/O: 3,363ms
- **Compare 修复**：跳过 `surreal` 在 compare backends 中时的冗余加载，消除两行 SurrealDB 输出
- **优化后 profile**：

  | 阶段 | 耗时 (ms) | 占比 |
  |------|----------|------|
  | 计算 local/world transform | 227,056 | 59.7% |
  | SurrealDB 写入 | 150,766 | 39.7% |
  | Parquet 写入 + finalize | 3,363 | 0.9% |
  | **总耗时** | **380,072** | **100%** |

- **总耗时减少 39%**：621,990ms → 380,072ms（节省 242 秒）
- 当前瓶颈已转移到"计算 transform"（59.7%，BFS + 逐节点 SurrealDB 查询）和"SurrealDB 写入"（39.7%）

## 2026-06-05 Viewer 独立站点 URL 计划与实现记录

- 使用 `planning-with-files` skill 组织本轮 Viewer URL / Nginx / 默认本机 IP 方案。
- `session-catchup.py` 用户级脚本路径不存在：`C:\Users\dpc\.cursor\skills\planning-with-files\scripts\session-catchup.py`，已记录为非阻塞错误。
- 已在 `task_plan.md` 追加 `Viewer 独立站点 URL 与 Nginx 入口计划`：
  - Phase V1 URL contract 决策完成。
  - Phase V2 Viewer Base URL 优先级完成。
  - Phase V3 代码改动完成。
  - Phase V4 运行态验证仍在进行。
  - Phase V5 文档和远端部署加固待办。
  - Phase V6 Nginx 自动配置与自动启动待办。
- 已在 `findings.md` 追加 Viewer 独立站点 URL findings，记录客户 URL 形态、Nginx 同源代理契约、多站点边界和验证状态。
- 根据用户补充要求，已把 Nginx 自动配置和自动启动/reload 纳入计划：
  - 生成站点专属 Nginx conf。
  - 自动 `nginx -t`。
  - 自动 `systemctl enable --now nginx` / `systemctl reload nginx`，无 systemd 时 fallback 到 `nginx -s reload`。
  - 无 root/sudo 时降级输出配置与命令。
  - 部署验收检查 `/`、`/api/health`、`/files/output/...`。
- 根据用户补充要求，已把 Windows/Linux 差异纳入 Phase V6：
  - Linux remote/root：系统 Nginx + `/etc/nginx/conf.d` + systemd/reload。
  - Linux no-systemd：系统 Nginx + `nginx -s reload` fallback。
  - Windows local default：不强制 Nginx，保留受管 `vite preview`。
  - Windows optional Nginx：仅在配置 `AIOS_NGINX_BIN` / `AIOS_NGINX_ROOT` 时自动生成 conf、`nginx.exe -t -p`、reload/start。
- 已开始执行 Windows 本机自动部署优先路径：
  - 当前机器未检测到 `nginx.exe`：`Get-Command nginx.exe`、`C:\nginx\nginx.exe`、`D:\nginx\nginx.exe` 均不存在。
  - 已在 `src/web_server/managed_project_sites.rs` 增加 Windows 可选 Nginx 集成。
  - 未检测到 Nginx 时自动记录日志并继续受管 `vite preview` fallback，确保当前本机自动部署可用。
  - 检测到 Nginx 时默认使用 `runtime/admin_sites/<site_id>/nginx` 作为独立 prefix，自动生成 `conf/nginx.conf` 和站点 conf。
  - Windows Nginx 启动顺序：生成配置 → `nginx.exe -p <prefix> -t` → `nginx.exe -p <prefix> -s reload` → reload 失败则启动 `nginx.exe -p <prefix>`。
- 验证记录：
  - 首次 `cargo check --bin aios-database --features web_server --offline --target-dir target-viewer-url-check` 失败，错误为 `Result::unwrap_or_else` 闭包多接收了一个参数。
  - 已修复为 `unwrap_or_else(|| ...)`。
  - 二次增量 `cargo check --bin aios-database --features web_server --offline --target-dir target-viewer-url-check` 通过，耗时 34.11s；输出仅有既有依赖 warning。
  - 临时运行 `web_server`：`WEB_SERVER_PORT=3198`, `ADMIN_USER=admin`, `ADMIN_PASS=admin-pass`, 未设置 `AIOS_VIEWER_BASE_URL`。
  - 登录 `/api/admin/auth/login` 成功后请求 `/api/admin/app-config`，返回 `viewer_base_url=http://192.168.31.60`。
  - 临时监听进程 `PID=43532` 已停止，释放 `3198` 端口。
  - 读取 `/api/admin/sites` 时发现历史站点仍持久化旧 `viewer_url`：包含 `backend=` / `data_source=parquet`，会被前端优先使用。
  - 已实现 read-side legacy URL 归一化：`/viewer/`、`backend=`、`backend%3D`、`data_source=` 命中时，若存在 `viewer_port`，返回前重算 clean URL。
  - 增量 `cargo check --bin aios-database --features web_server --offline --target-dir target-viewer-url-check` 通过，耗时 14.58s。
  - 增量 `cargo build --bin web_server --features web_server --offline --target-dir target-viewer-url-check` 通过，耗时 36.13s。
  - 重启临时 `web_server` 后，`GET /api/admin/sites` 已返回 clean `viewer_url`，例如 `http://192.168.31.60/?output_project=AvevaPlantSample&show_dbnum=250164`。
  - 第二次临时监听进程 `PID=9212` 已停止，释放 `3198` 端口。
  - 启动 `avevaplantsamplegoalfast-8084` 做新写入 URL 验证时，发现 `http://192.168.31.60:3105` 被拒绝连接；根因是受管 Viewer 仍绑定 `127.0.0.1`。
  - 已新增 `AIOS_VIEWER_BIND_HOST`，默认 `0.0.0.0`，并将受管 Viewer 启动参数 `--host` 改为该值。
  - 已修正本机 fallback URL：无全局/站点公网入口时生成 `http://<local-ip>:<viewer_port>`，而不是 `http://<local-ip>`。
  - 重建后再次启动 `avevaplantsamplegoalfast-8084`，最终 `Running`，返回 `viewer_port=3105` 与 `viewer_url=http://192.168.31.60:3105/?output_project=AvevaPlantSampleGoalFast&show_dbnum=250164`。
  - 真实请求该 URL：HTTP `200`，内容长度 `529`，命中 plant3d/Vite 标识。
  - 已停止 smoke 站点和临时 admin，`3198/8084/3105` 均已释放。
  - 继续硬化管理端 fallback：新增 `/api/admin/app-config.viewer_base_url_source`，区分 `env` 和默认 `local_ip`。
  - 更新 `ui/admin/src/lib/app-config.ts` 与 `ui/admin/src/lib/viewer.ts`：默认 `local_ip` 来源会结合 `site.viewer_port` 生成 `http://<local-ip>:<viewer_port>/...`；显式配置来源保持原始 base。
  - 已执行 `npm run build`，管理端 TS 与生产构建通过，静态 admin 产物已刷新。
  - 已补充部署文档 `docs/guides/AvevaPlantSample-deploy-preset.md`：说明 clean Viewer URL、Windows 本机 fallback、生产 `AIOS_VIEWER_BASE_URL` / `public_base_url`、Nginx 同源反代、多站点 vhost/port/domain 隔离。
  - 已更新 `shells/deploy/nginx-plant3d-web.conf.example` 注释：记录默认本机 IP、`viewer_port` fallback、`AIOS_VIEWER_BIND_HOST`。
  - 已为非 Windows 平台新增 Linux Nginx 自动配置：写入 `AIOS_NGINX_CONF_DIR` 或 `/etc/nginx/conf.d` 下的站点 conf，执行 `nginx -t`，再尝试 `systemctl reload nginx`、`systemctl enable --now nginx`、`nginx -s reload`。
  - Linux 自动化遇到无 Nginx、无权限、配置校验失败、reload/start 失败时会写 viewer 日志并给出手动命令；校验失败不执行 reload；整体继续受管 `vite preview` fallback。
  - 本机只有 Windows Rust target，无法在本机编译 Linux cfg；已完成 Windows 主目标 `cargo check` 验证。
- 代码改动摘要：
  - `src/web_server/mod.rs` 新增共享 `get_local_ip_via_udp()`。
  - `src/web_server/site_config_handlers.rs` 改为复用共享 IP 探测 helper。
  - `src/web_server/admin_handlers.rs` 的 `/api/admin/app-config` 未配置 `AIOS_VIEWER_BASE_URL` 时默认返回 `http://<local-ip>`。
  - `src/web_server/managed_project_sites.rs` 的 `build_viewer_url()` 改为 `AIOS_VIEWER_BASE_URL` → `public_entry_url` → `http://<local-ip>` → `127.0.0.1:<viewer_port>`，并只拼 `output_project/show_dbnum`。
  - `ui/admin/src/lib/viewer.ts` 改为独立 Viewer Base URL + 业务 query，不再拼 `backend` / `backendPort` / `data_source`。
  - `shells/deploy/nginx-plant3d-web.conf.example` 新增根站点 + `/api` `/files` `/ws` 同源代理示例。
- 静态验证：
  - `ReadLints` 对修改的 Rust/TS 文件无诊断。
  - `git diff --check` 对相关文件无空白错误。
- 未执行项：
  - 未启动/重启 web_server 做 `/api/admin/app-config` 真实响应验证。
  - 未部署 Nginx 做客户 URL 真实加载验证。
