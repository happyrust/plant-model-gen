# Changelog

## Unreleased

### Added

- `/admin` 站点管理新增"关联工程"字段（`associated_project`），支持持久化到 SQLite 并在新建/编辑站点时设置；打开 Viewer 时优先用该字段，未设置则回退到项目名称。
- `/admin/#/collaboration` 异地协同工作台正式注册路由，AppHeader 导航栏新增「异地协同」入口。

### Changed

- `/admin` 注册表页改造：`DbOption 导入` 改用对话框替代 `window.prompt`，`删除` 改用确认对话框替代 `window.confirm`，`创建任务` 改为跳转到任务向导页面；后端地址列新增复制按钮，编辑按钮改用 Pencil 图标。

### Fixed

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
