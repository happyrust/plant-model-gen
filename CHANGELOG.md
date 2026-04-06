# Changelog

## Unreleased

### Added

- 导出模型查询：增加对 `Neg` (负实体) 的导出支持，并放宽图元节点输出条件确保元素正常输出。
- 新增工具脚本：`scripts/export_dbnum_parquet_file.sh` 和 `test_ida_mcp.py` 辅助调测。
- 完善 web_server 站点模型与注册链路：新增站点身份与配置信息链路（`site_id/site_name/region/project_code/frontend_url/backend_url`）以及 `project_code` 作为站点代号字段。
- 新增独立站点注册模块 `src/web_server/site_registry.rs`，并统一 `GET /api/sites` 与 `GET /api/deployment-sites` 的清单事实源。
- 站点配置 CRUD 能力增强：支持从配置文件导入站点、按项目/区域过滤、以及可直接管理站点的 `bind_host/bind_port` 监听信息。

### Changed

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

### Fixed

- **embed-url**：当请求未带工作流角色且单据库中尚无 `role` 时，签发 JWT 仍写入默认 **`sj`**，避免 plant3d 嵌入页因 claims 缺 `role` 报「缺少可信身份声明」。
- 调整 OBJ 导出默认首图为更接近正视的验收视角，减少斜俯视角度导致的方向误判。
- 修复 `--regen-model` 未清理旧 `tubi_relate` 导致 BRAN/HANG 导出时混入历史局部坐标直段的问题。
- 修复三通元件库表达式 `TWICE PARAM 3` 被错误求值为 `0`，导致 `24381_145582` 一类 `TEE` 丢失支管几何的问题。
- 补齐 `--debug-model --export-obj` 的 PNG 预览输出，`CaptureConfig` 不再只是打印“自动启用截图”但没有实际文件产出。
- 修复 `deployment-sites` 页面表单可访问性问题，补齐输入字段 `label/for/id/name` 与按钮可读性文案，降低控制台无障碍告警。
