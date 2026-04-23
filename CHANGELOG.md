# Changelog

## Unreleased

### Added

- **异地协同前端独立 + API 汇总（2026-04-22 · feat/collab-api-consolidation 分支）**：把 `D:\work\plant-code\web-server` 的异地协同专业监控 UI 剥离为独立仓库 `D:\work\plant-code\plant-collab-monitor`（Vue 3.5 + Vite 5.4 + Naive UI 2.40 + Tailwind 3.4），后端 API 汇入 plant-model-gen 作为单一源。
  - Phase 1 代码层（7 commit）：
    - `site_config_handlers.rs` 完全迁入（7 路由，路径 env 化）
    - `mqtt_monitor_handlers.rs` 完全迁入（5 基础路由）
    - `sync_control_handlers` 追加 MQTT 订阅/主从控制 7 简化 stub 端点（Phase 1.3a）
    - `remote_sync_handlers`（101KB）与 `sync_control_center` 保留 plant 版为主，web-server 独有 handler 通过子 phase 按需补齐
    - `deployment_sites.sqlite` schema 对齐：`src/web_server/collab_migrations.rs::ensure_collab_schema` 幂等追加 `remote_sync_sites.master_mqtt_host/port/location` 3 列 + 新建 `node_config` 表
    - `fix(mbd)`: 给 `mbd_pipe_api::compute_branch_layout_result` 与唯一 caller 加 `#[cfg(feature = "mbd-iso")]` gate，解除 `cargo run --features web_server` 被 rs-core 未完工 `iso_extras/iso_params/SolveBranchInput` 阻塞的问题（未开启时 `layout_result` 降级为 None）
  - Phase 2 新前端仓库 plant-collab-monitor（5 commit）：
    - vite 脚手架 + TypeScript strict + Pinia + vue-router 11 路由 + Naive UI 主题
    - axios 类型化 API 层（`syncApi`/`remoteSyncApi`/`mqttApi`/`siteConfigApi`）
    - 完整移植 web-server/frontend 的 7 核心组件 + 2 charts + 5 composables + 11 视图
    - `vite.config.ts` proxy `/api /ws /files /static → VITE_API_TARGET`（默认 `http://127.0.0.1:3100`）
    - `vue-tsc -b` 0 error；`vite build` 7.61s 成功（408 KB gzipped）
  - Phase 3 端到端冒烟：8 关键 endpoint 6 个直接 200，2 个 `/api/remote-sync/*` 返回 503 属 admin 鉴权保护（deny by design，不是 bug）
  - Phase 4 文档与部署：
    - `docs/architecture/异地协同API汇总清单.md`：81 个端点按 6 大功能域分组，每条标注状态（NEW/merge/stub/plant）
    - `shells/deploy/nginx-plant-collab-monitor.conf.example`：Nginx 反代示例（`/monitor/` SPA + `/api/` proxy + `/ws/` upgrade + SSE 友好）
    - `shells/deploy/deploy_all_with_frontend.sh` 追加 `plant-collab-monitor` 构建 + rsync 步骤（可用 `SKIP_COLLAB_MONITOR=true` 跳过）
    - `plant-collab-monitor/README.md` 完整定位/技术栈/环境变量/结构/部署/限制
    - `web-server/MIGRATION_NOTICE.md` 向旧仓库用户说明迁移路径
  - 4 张规划 diagram（`design/collab-consolidation/`）：架构总览 · API 迁移 swimlane · 同步时序 · Roadmap 甘特
  - 计划/执行清单：`docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md`、`2026-04-22-phase-1-execution-checklist.md`、`2026-04-22-phase-3-phase-4-execution-checklist.md`、`2026-04-22-m1-smoke-test-result.md`

- `design/collaboration-v2/index.html` **Hi-Fi 原型 5 轮 24 项 UI 完善**（1267→1866 行 +47%）：
  - R1: 左栏搜索过滤、拓扑诊断历史/关联任务、洞察图表 tooltip、Pending/Running 区分、空状态 UI、Tab fadeIn 动画、表格行指示条
  - R2: SVG 流向动画粒子（ok/warn 沿路径运动）、实时模拟引擎（3s 进度自增+完成 toast）、配置保存 Toast+删除确认弹窗、响应式 3 级断点+hamburger、日志关键词高亮
  - R3: 站点表格 5 列排序、KPI sparkline 趋势线、日志行展开详情+CSV Blob 导出、拓扑全屏模式+节点 hover tooltip、新增站点 Modal 表单、focus-visible 无障碍
  - R4: Toast 5s 自动消失+进度条+hover 暂停、协同组切换 Hero 数据联动、拓扑节点拖拽重定位+重置、SVG ProgressRing 进度环组件
  - R5: usePersist localStorage 偏好持久化、?键快捷键面板（1-4/D/N）、节点 dimmed 聚焦效果、bad 流向脉冲闪烁动画
  - 5 份开发计划文档（UI_POLISH_PLAN R1–R5.md）

- `/admin/#/collaboration` **异地协同 v2 UI 里程碑 M1** 上线：
  - 4-Tab 壳（拓扑 / 站点 / 洞察 / 日志），URL hash 同步 `#topo` `#sites` `#insight` `#logs`
  - 新增 `CollaborationTopologyPanel.vue`：放射状拓扑可视化，节点内嵌 5 态 chip（Idle/Scanning/ChangesDetected/Syncing/Completed/Error），支持 `siteCode` 角标与 `progress` bar
  - 新增 `CollaborationActiveTasks.vue`：日志 Tab 顶部「进行中任务」条，含进度条与 `Running` / `Pending` chip
  - 新增 `CollaborationFailedTasks.vue`：洞察 Tab 底部「失败任务队列」，含 `retry_count/max_retries` 与重试 / 清理已耗尽操作
  - 新增 `CollaborationConfigDrawer.vue`：右侧参数配置抽屉，4 分组 · 9 项（自动化 / 吞吐并发 / 连接重连 / 通知日志）
  - 指挥条新增 `ONLINE / 轮询模式` 实时连接徽标（由 `useCollaborationStore.realtimeConnected` 驱动；Phase 9 接入 SSE 后生效）
  - `types/collaboration.ts` 追加 `CollaborationDetectionStatus` / `CollaborationActiveTask` / `CollaborationFailedTask` / `CollaborationConfig` / `CollaborationToast` / `CollaborationStreamEvent` 6 个类型；`CollaborationSiteCard` 可选追加 `siteCode` / `detectionStatus` / `progress` / `pendingItems` / `syncedItems`
  - `stores/collaboration.ts` 追加 `activeTasks` / `failedTasks` / `realtimeConnected` / `collabConfig` / `toasts` 等 state，`pushToast` / `dismissToast` / `fetchActiveTasks` / `fetchFailedTasks` / `retryFailedTask` / `cleanupExhaustedFailedTasks` / `fetchCollabConfig` / `saveCollabConfig` 8 个 action（Phase 9 后端就绪后填充实现）
  - `style.css` 追加 `--collab-*` design token 命名空间（含 `[data-theme="dark"]` 暗色覆盖），工业感配色避开紫渐变 AI slop
  - 设计产出归档在 `design/collaboration-v2/`（原型 HTML · README · GAP_ANALYSIS · DEVELOPMENT_PLAN · ROADMAP · 7 张 Playwright 截图）
- 新增 `docs/plans/2026-04-19-admin-站点部署整改计划.md`：对 admin 站点部署功能进行全链路审核（后端编排、前端状态机、安全默认值、Viewer 联动），输出 5 阶段整改计划（P0 安全收口 / P0 状态机加固 / P1 部署解耦 / P1 Viewer URL / P1 可观测性）、9 项回归矩阵及长期演进路线。
- 新增 `scripts/test-admin-deployment.ps1`：Admin 站点端到端部署回归脚本，覆盖登录 / 建站 / 解析 / 启动 / 健康检查 / 停站 / 清理 9 个步骤，默认针对 `AvevaPlantSample + aps7011_0001` 场景，支持 `-SkipCleanup` 与 `ADMIN_USER/ADMIN_PASS` 环境变量。
- 新增 `.memory/.gitkeep` 与 `.memory/2026-04-17.md`：沉淀 E3D 3.1 F&M state 补丁、PML modLoadMethod、jmp-self / spawn+suspend 等逆向调试日志，便于跨会话衔接。

### Changed

- 精简 `.cursor/rules/mcp-messenger.mdc` 与 `.cursor/rules/my-mcp.mdc` 中的重复强约束段落，仅保留"回合结束必须调用 `check_messages`"等核心条款，避免多模型规则冗余。

- 异地协同 remote-sync API 路由从 `mod.rs` 提取到 `remote_sync_handlers::create_remote_sync_routes()`，统一纳入 `admin_api_routes` 认证链路。
- `open_sqlite()` 使用 `std::sync::Once` 守卫，确保 SQLite schema 仅初始化一次。
- remote-sync 所有 `map_err` 增加 `eprintln!` 错误日志输出，便于问题排查。
- 站点诊断批量请求新增并发限制（每批 5 个），避免大量站点时一次性并发过高。
- 新增异地协同架构文档、admin 系统审核总结、本机站点编排架构文档。
- 新增 `scripts/test-remote-sync.sh` 测试脚本。

- `/admin` 站点管理新增"关联工程"字段（`associated_project`），支持持久化到 SQLite 并在新建/编辑站点时设置；打开 Viewer 时优先用该字段，未设置则回退到项目名称。
- `/admin/#/collaboration` 异地协同工作台正式注册路由，AppHeader 导航栏新增「异地协同」入口。
- 新增 `site-status.ts` 集中管理站点状态 label/color/busy/error 判断规则，供列表页和详情页统一使用。
- 站点工具栏新增 quick filter chips（全部 / 运行中 / 处理中 / 异常 / 待解析），一键筛选快速聚焦。
- 总览页统计卡改为 4 张业务卡（总站点 / 运行中 / 处理中 / 异常），替代旧 SiteStatsCards。
- 新增 `SiteWorkbenchHeader` 组件（标题/副标题/刷新按钮/最近刷新时间/当前结果数）。
- 站点列表项目名列新增入口地址链接和错误摘要行，合并 DB/Web 端口为一列，提升信息密度。
- 新增 `SiteDetailHeader` 组件（状态徽标 + 统一按钮禁用 + Viewer 入口）。
- 新增 `SiteRuntimeCards` 组件（当前阶段/数据库/Web 服务/解析 4 张运行态卡片）。
- 新增 `SiteLogSummaryPanel` 组件（每个 stream 展示行数/更新时间/关键日志摘要）。
- 新增 `SiteConfigSections` 组件（项目信息/运行配置/路径信息/时间信息 4 个结构化分区）。

### Changed

- `/admin` 注册表页改造：`DbOption 导入` 改用对话框替代 `window.prompt`，`删除` 改用确认对话框替代 `window.confirm`，`创建任务` 改为跳转到任务向导页面；后端地址列新增复制按钮，编辑按钮改用 Pencil 图标。
- `SiteDrawer` 抽屉表单重组为 4 个 fieldset 分组（项目信息 / 运行配置 / 解析范围 / 数据库凭据），提升表单可读性。
- `SiteDataTable` 状态徽标和按钮禁用规则统一使用 `site-status.ts`，不再各组件内联定义。
- 删除不再使用的 `SiteStatsCards` 组件（已被总览页内联统计卡替代）。

### Changed

- **引入 `web_api::assemble_stateless_web_api_routes()` 聚合装配函数，结构性消除"新增路由忘记 merge"的静默遗漏风险（2026-04-23）**：`src/web_api/mod.rs` 新增 `assemble_stateless_web_api_routes()`，把 13 个无状态 `create_*_routes()`（`room_tree` / `pdms_attr` / `pdms_transform` / `ptset` / `pdms_model_query` / `review_integration` / `platform_api` / `jwt_auth` / `review_api` / `scene_tree` / `mbd_pipe` 加上两条 `nest` 前缀的 `pipeline_annotation` 与 `version`）一次性装配为单个 `Router`；`src/web_server/mod.rs` 端从原来 5 个独立 `let xxx_routes = create_xxx_routes();` 与 12 个分散 `.merge() / .nest()` 调用，统一收拢为 1 个 `let stateless_web_api_routes = assemble_stateless_web_api_routes();` 与 1 个 `.merge(stateless_web_api_routes)`，未来新增无状态路由只需在聚合函数里追加一行；带 state 的 `collision / e3d_tree / noun_hierarchy / spatial_query / search / upload / room_api` 仍由 `web_server/mod.rs` 在拿到对应 state 后单独挂载。M2 落地前后跑过 10/10 PASS 的 HTTP 冒烟矩阵（含 `pdms transform / pdms transform compute / pdms ui-attr / pdms ptset / pdms type-info / pdms children / version / 负面 bogus 路径` 八个维度）和 `scripts/verify_fixing_position.ps1`（世界坐标误差 0.006mm），未引入回归。同期落盘 `docs/plans/2026-04-23-pdms-transform-followup-hardening-plan.md` 的 M1→M5 计划与 `docs/plans/2026-04-23-pdms-transform-m1-m2-execution-checklist.md` 的执行清单。

### Fixed

- **修复 `plant3d-web` 控制台 `q pos` / `q ori` / `q pos wrt owner` / `q ori wrt owner` 四条命令永远返回 `Failed to query transform: Error: HTTP 404 Not Found` 的问题（2026-04-23）**：`src/web_api/pdms_transform_api.rs` 完整定义了 `create_pdms_transform_routes()`（涵盖 `/api/pdms/transform/{refno}` 与 `/api/pdms/transform/compute/{refno}` 两条路由），`src/web_api/mod.rs` 也已 `pub use`，但 `src/web_server/mod.rs` 的装配段漏了 `use` / `let` / `.merge()` 三处，导致 Axum 根本没挂载该路由前缀，所有请求命中默认 fallback 返回 404。本次补齐三处装配；`scripts/verify_fixing_position.ps1` 重新跑通（世界坐标误差 0.006mm，< 1.0mm 容差），线上调用 `GET /api/pdms/transform/24381_145018` 返回 200 + `success=true`。同步落盘问题复盘 `docs/plans/2026-04-23-pdms-transform-route-missing-registration-fix.md`，后续通过本次同期落地的 `assemble_stateless_web_api_routes` 聚合装配函数（见上方 Changed 条目）从机制上彻底防止同类问题重发。
- 修复 `/admin` 注册表页导入/删除操作因函数名不匹配（`handleImport` → `openImportDialog`、`handleDelete` → `openDeleteConfirm`）导致点击无响应的问题。
- 修复 `auth.ts` 中 `session.user` / `user` 可能为 `undefined` 的 TypeScript 严格检查错误。
- 修复 `SiteDataTable.vue` 和 `SiteDetailView.vue` 中 `window.open` / `navigator.clipboard` 在 Vue 模板作用域中不可访问的 TypeScript 错误，改为组件方法调用。
- Admin 前端通过 `vue-tsc` 类型检查，0 错误。

### Previously Added

- `/admin` 新增“中心注册表”页面，并补齐 admin 风格的注册表接口：支持列表/过滤/分页、新建/编辑/删除、`DbOption.toml` 导入、健康检查、配置导出和创建任务。
- 导出模型查询：增加对 `Neg` (负实体) 的导出支持，并放宽图元节点输出条件确保元素正常输出。
- 新增工具脚本：`scripts/export_dbnum_parquet_file.sh` 和 `test_ida_mcp.py` 辅助调测。
- 完善 web_server 站点模型与注册链路：新增站点身份与配置信息链路（`site_id/site_name/region/project_code/frontend_url/backend_url`）以及 `project_code` 作为站点代号字段。
- 新增独立站点注册模块 `src/web_server/site_registry.rs`，并统一 `GET /api/sites` 与 `GET /api/deployment-sites` 的清单事实源。
- 站点配置 CRUD 能力增强：支持从配置文件导入站点、按项目/区域过滤、以及可直接管理站点的 `bind_host/bind_port` 监听信息。
- 新增任务创建向导原型 `ui/task_wizard.pen`，并补充 `docs/plans/2026-04-09-站点管理功能开发计划.md` 作为站点管理后续开发说明。
- 新增 `src/web_server/admin_response.rs`，归纳管理端接口的响应结构以便复用。
- 补充管理员模块教程 `docs/guides/ADMIN_MODULE_TUTORIAL.md`；补充 Room Compute CLI 校验说明 `ROOM_COMPUTE_CLI_VALIDATION.md`，并提供 `scripts/verify-room-compute.ps1` 与 `verification/room_compute_validation.json` 作为校验脚本与参考数据。

### Changed

- `/admin` 后台入口继续收口：本机编排使用 `/admin/#/sites`，中心注册表使用 `/admin/#/registry`，异地协同使用 `/admin/#/collaboration`；`/deployment-sites`、`/console/deployment/sites`、`/remote-sync` 与 `/console/sync/remote` 只保留兼容跳转。
- **平台 API `POST /api/review/embed-url` 请求体**：本单据工作流角色字段由 `role` 更名为 **`workflow_role`**（Rust 侧 `EmbedUrlRequest.workflow_role`）；JSON 仍接受顶层别名 **`role`** 以兼容旧客户端。**不再接受** `user_role`（含 `extra_parameters.user_role`）。JWT claims 内字段名仍为 `role`，未变。详见 `docs/guides/PLATFORM_API_HTTP_EXAMPLES.md`。
- 调整环境默认配置：`db_options/DbOption.toml` 等修改默认数据库连接模式为 `file`。

- 房间树 API 支持展开 `COMP_GROUP`（构件分组）并返回该组下所有构件列表，同时支持查询节点下的有效子节点数量。
- 房间树 API 支持查询单个构件 refno 的祖先路径（正确关联至相应的 `COMP_GROUP` 和 `ROOM` 节点）。
- 提取网格生成状态管理至独立的 `mesh_state.rs`，优化流式模型生成与流形布尔运算的依赖调度。
- 提供 `query_component_insts` 示例以支持构件实例的快捷查询调测。
- 一个 web_server 实例现支持“单站点单项目”注册语义，`site_id/backend_url/bind_host+port` 可参与唯一性约束。
- `site_identity` 与站点列表接口增强，返回站点绑定项目、前后端地址及状态，支持跨进程清单聚合。
- 新增站点页面可配置区域字段：区域、项目、项目代号、前端地址、后端地址、监听地址与运行配置收敛。
- 优化 DB 模式和 File 模式下的 mesh 依赖判断逻辑，确保流式生成与强制生成的正确触发。
- 重构模型生成编排层（`orchestrator`）与相关查询接口，提升查询和并发图组生成的稳定性。
- 任务创建接口在 `manual_refnos` 非空时自动切换到 `RefnoModelGeneration`，`/api/model/generate-by-refno` 同步透传 noun 过滤与调试限制参数。
- SQLite 空间查询接口新增 `spec_values` 过滤，并改为先按距离完整排序后再截断返回结果。
- 房间计算验收基线中的房间编号统一改为 `R540`，与当前数据事实保持一致。
- 增强后台管理模块：完善管理端登录鉴权与任务创建/进度相关 API 及后端处理逻辑（涉及 `admin_auth_handlers`、`admin_task_handlers`、`handlers`、`models`、`room_api`、`mbd_pipe_api`、`main`、`cli_modes` 等）。
- 管理端前端（`ui/admin`）：更新鉴权与任务相关 API 客户端、状态存储及任务向导与进度视图。
- 同步更新内置管理端静态资源。

### Fixed

- 修复 admin 管理接口此前只在前端带 token、后端并未校验的问题；现在 `/api/admin/sites`、`/api/admin/tasks` 与 `/api/admin/registry/*` 都要求有效 Bearer token，失效会话会回到登录页。
- 修复 admin 登录态接口返回不一致的问题：`/api/admin/auth/me` 改为真实读取当前会话，`/api/admin/auth/logout` 返回明确的登出结果，避免前端状态残留。
- **embed-url**：当请求未带工作流角色且单据库中尚无 `role` 时，签发 JWT 仍写入默认 **`sj`**，避免 plant3d 嵌入页因 claims 缺 `role` 报「缺少可信身份声明」。
- 调整 OBJ 导出默认首图为更接近正视的验收视角，减少斜俯视角度导致的方向误判。
- 修复 `--regen-model` 未清理旧 `tubi_relate` 导致 BRAN/HANG 导出时混入历史局部坐标直段的问题。
- 修复三通元件库表达式 `TWICE PARAM 3` 被错误求值为 `0`，导致 `24381_145582` 一类 `TEE` 丢失支管几何的问题。
- 补齐 `--debug-model --export-obj` 的 PNG 预览输出，`CaptureConfig` 不再只是打印“自动启用截图”但没有实际文件产出。
- 修复 `deployment-sites` 页面表单可访问性问题，补齐输入字段 `label/for/id/name` 与按钮可读性文案，降低控制台无障碍告警。
- 修复 `query_deep_visible_inst_refnos` 与 `/api/e3d/visible-insts` 对 BRAN/HANG 根节点的可见范围判断，避免树上可见但实际不加载。
- 修复 web_server 任务配置向 `DbOption` 下发时遗漏 `manual_refnos` 的问题，避免 refno 任务退回全库生成。
