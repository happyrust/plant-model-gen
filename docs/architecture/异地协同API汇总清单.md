# 异地协同 API 汇总清单

> 产出日期：2026-04-22
>
> 对应里程碑：M1（Phase 1.1–1.6 代码就位）+ M3（实际运行冒烟 6/8 通过 + 2 admin-gated 503）
>
> 相关文档：
> - 父计划：`docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md`
> - Phase 1 清单：`docs/plans/2026-04-22-phase-1-execution-checklist.md`
> - Phase 3/4 清单：`docs/plans/2026-04-22-phase-3-phase-4-execution-checklist.md`
> - M1 冒烟结果：`docs/plans/2026-04-22-m1-smoke-test-result.md`

本清单按功能域汇总 plant-model-gen 为异地协同场景提供的全部 HTTP 端点，供 `plant-collab-monitor` 前端对接时参考。

## 使用说明

- **基础地址**：`http://<host>:<port>`，`<port>` 由 `db_options/DbOption.toml.server_release_ip` 决定，默认 `3100`
- **鉴权**：带 `admin-gated` 标识的端点需要先完成 admin 登录拿 JWT，再在请求头加 `Authorization: Bearer <token>`
  - 配置：启动前设置环境变量 `ADMIN_USER` / `ADMIN_PASS`
  - 登录端点：`POST /api/admin/login`（plant 原生，未覆盖在本清单）
- **SSE**：标注 `SSE` 的端点返回 `text/event-stream`，前端用 `EventSource` 订阅
- **状态**：
  - `NEW` — Phase 1 本轮新增（从 web-server 迁入）
  - `merge` — plant 已有，做过 diff/合并
  - `stub` — Phase 1.3a 简化 stub，真实逻辑待后续补齐
  - `plant` — plant-model-gen 原生，未改动

## 1. 站点配置 · `/api/site-config/*` + `/api/site/info`

| Method | Path | Handler | 状态 | 说明 |
|---|---|---|---|---|
| GET | `/api/site-config` | `site_config_handlers::get_site_config` | NEW | 读取当前 DbOption.toml 全部字段返回 `{status, config, config_file_location}` |
| GET | `/api/site/info` | `site_config_handlers::get_site_info` | NEW | 给其他站点查询用的轻量信息 `{file_server_host, mqtt_host/port, location, location_dbs, project_name, project_code}` |
| POST | `/api/site-config/save` | `site_config_handlers::save_site_config` | NEW | 写 SQLite `site_config` 表 + `DbOption.toml`（**stub**：无 `shutdown_tx`，需手动重启生效） |
| POST | `/api/site-config/validate` | `site_config_handlers::validate_site_config` | NEW | 校验项目路径存在性、IP/端口格式、`location_dbs` 非空 |
| POST | `/api/site-config/reload` | `site_config_handlers::reload_site_config` | **stub** | 简化实现，提示"请手动重启"，待接入 `config_reload_manager` |
| POST | `/api/site-config/restart` | `site_config_handlers::restart_server` | **stub** | 同上 |
| GET | `/api/site-config/server-ip` | `site_config_handlers::get_server_ip` | NEW | 通过 UDP 探测本机出口 IPv4 |

## 2. MQTT 监控 · `/api/mqtt/*`

### 2.1 节点与消息（Phase 1.2）

| Method | Path | Handler | 状态 | 说明 |
|---|---|---|---|---|
| GET | `/api/mqtt/nodes` | `mqtt_monitor_handlers::get_mqtt_nodes_status` | NEW | 当前站点 + 可见节点列表 `{current_location, is_master_node, nodes, summary:{online,offline,total}}` |
| GET | `/api/mqtt/nodes/{location}` | 同上 | NEW | 按 location 查单节点状态（当前复用同一 handler） |
| DELETE | `/api/mqtt/nodes/{location}` | `mqtt_monitor_handlers::remove_mqtt_node` | NEW | 主节点：从数据库删站点；从节点：停订阅 + `clear_master_config_internal` |
| POST | `/api/mqtt/nodes/client-unsubscribed` | `mqtt_monitor_handlers::client_unsubscribed` | NEW | 从节点向主节点通报"已取消订阅" |
| GET | `/api/mqtt/messages` | `mqtt_monitor_handlers::get_message_delivery_status` | NEW | 消息投递总表（按 message_id + receivers） |
| GET | `/api/mqtt/messages/{message_id}` | `mqtt_monitor_handlers::get_message_delivery_detail` | NEW | 单条消息投递详情 |

### 2.2 订阅 + 主从控制（Phase 1.3a 简化 stub）

| Method | Path | Handler | 状态 | 说明 |
|---|---|---|---|---|
| GET | `/api/mqtt/broker/logs` | `sync_control_handlers::get_mqtt_broker_logs_api` | **stub** | 返回空 `{logs:[], count:0}`，待接 `sync_control_center::get_mqtt_broker_logs` |
| POST | `/api/mqtt/subscription/start` | `sync_control_handlers::start_mqtt_subscription_api` | **stub** | 简化：直接 `remote_runtime::start_runtime(env_id)` |
| POST | `/api/mqtt/subscription/stop` | `sync_control_handlers::stop_mqtt_subscription_api` | **stub** | 简化：置空 `REMOTE_RUNTIME` |
| POST | `/api/mqtt/subscription/clear-master-config` | `sync_control_handlers::clear_master_config_api` | **stub** | 调 `clear_master_config_internal` |
| GET | `/api/mqtt/subscription/status` | `sync_control_handlers::get_mqtt_subscription_status` | **stub** | 返回 `{is_running, is_server_running:false, location, subscribed_topics:["Sync/E3d"]}` |
| POST | `/api/mqtt/node/set-master` | `sync_control_handlers::set_as_master_node` | **stub** | 仅 log warn，TODO: 真实写入 `DbOption.toml` + SQLite |
| POST | `/api/mqtt/node/set-client` | `sync_control_handlers::set_as_client_node` | **stub** | 同上 |

## 3. 同步服务控制 · `/api/sync/*`（plant 原生）

| Method | Path | Handler | 状态 |
|---|---|---|---|
| POST | `/api/sync/start` | `sync_control_handlers::start_sync_service` | plant |
| POST | `/api/sync/stop` | `sync_control_handlers::stop_sync_service` | plant |
| POST | `/api/sync/restart` | `sync_control_handlers::restart_sync_service` | plant |
| POST | `/api/sync/pause` | `sync_control_handlers::pause_sync_service` | plant |
| POST | `/api/sync/resume` | `sync_control_handlers::resume_sync_service` | plant |
| GET | `/api/sync/status` | `sync_control_handlers::get_sync_status` | plant |
| GET | `/api/sync/events` | `sync_control_handlers::sync_events_stream` | plant |
| GET | `/api/sync/metrics` | `sync_control_handlers::get_sync_metrics` | plant |
| GET | `/api/sync/metrics/history` | `sync_control_handlers::get_sync_metrics_history` | plant |
| GET | `/api/sync/queue` | `sync_control_handlers::get_sync_queue` | plant |
| POST | `/api/sync/queue/clear` | `sync_control_handlers::clear_sync_queue` | plant |
| GET | `/api/sync/config` | `sync_control_handlers::get_sync_config` | plant |
| PUT | `/api/sync/config` | `sync_control_handlers::update_sync_config` | plant |
| POST | `/api/sync/test` | `sync_control_handlers::test_sync_connection` | plant |
| POST | `/api/sync/task` | `sync_control_handlers::add_sync_task` | plant |
| POST | `/api/sync/trigger-download` | `sync_control_handlers::trigger_file_download` | plant |
| POST | `/api/sync/task/{id}/cancel` | `sync_control_handlers::cancel_sync_task` | plant |
| GET | `/api/sync/history` | `sync_control_handlers::get_sync_history` | plant |
| GET (SSE) | `/api/sync/events/stream` | `sse_handlers::sync_events_handler` | plant |
| GET (SSE) | `/api/sync/events/test` | `sse_handlers::test_sse_handler` | plant |
| POST | `/api/sync/mqtt/start` | `sync_control_handlers::start_mqtt_server_api` | plant |
| POST | `/api/sync/mqtt/stop` | `sync_control_handlers::stop_mqtt_server_api` | plant |
| GET | `/api/sync/mqtt/status` | `sync_control_handlers::get_mqtt_server_status` | plant |

## 4. 异地环境与站点 · `/api/remote-sync/*`（admin-gated · plant 原生 · Phase 1.4 零操作）

> ⚠️ 以下全部需要 admin 鉴权，未登录访问返回 503

| Method | Path | Handler | 说明 |
|---|---|---|---|
| GET | `/api/remote-sync/envs` | `remote_sync_handlers::list_envs` | 环境列表 |
| POST | `/api/remote-sync/envs` | `remote_sync_handlers::create_env` | 创建环境 |
| GET | `/api/remote-sync/envs/{id}` | `remote_sync_handlers::get_env` | 单个环境 |
| PUT | `/api/remote-sync/envs/{id}` | `remote_sync_handlers::update_env` | 更新环境 |
| DELETE | `/api/remote-sync/envs/{id}` | `remote_sync_handlers::delete_env` | 删除环境 |
| GET | `/api/remote-sync/envs/{id}/sites` | `remote_sync_handlers::list_sites` | 环境下的站点列表 |
| POST | `/api/remote-sync/envs/{id}/sites` | `remote_sync_handlers::create_site` | 创建站点 |
| GET | `/api/remote-sync/sites/{id}` | `remote_sync_handlers::get_site_by_id` | 单站点详情 |
| PUT | `/api/remote-sync/sites/{id}` | `remote_sync_handlers::update_site` | 更新站点 |
| DELETE | `/api/remote-sync/sites/{id}` | `remote_sync_handlers::delete_site` | 删除站点 |
| POST | `/api/remote-sync/envs/{id}/apply` | `remote_sync_handlers::apply_env` | 应用环境到运行时 |
| POST | `/api/remote-sync/envs/{id}/activate` | `remote_sync_handlers::activate_env` | 激活环境 |
| POST | `/api/remote-sync/runtime/stop` | `remote_sync_handlers::stop_runtime` | 停止 runtime |
| GET | `/api/remote-sync/runtime/status` | `remote_sync_handlers::runtime_status` | runtime 状态 |
| GET | `/api/remote-sync/runtime/config` | `remote_sync_handlers::runtime_config` | runtime 配置 |
| POST | `/api/remote-sync/envs/{id}/test-mqtt` | `remote_sync_handlers::test_mqtt_env` | 测 MQTT 连通 |
| POST | `/api/remote-sync/envs/{id}/test-http` | `remote_sync_handlers::test_http_env` | 测文件服务连通 |
| POST | `/api/remote-sync/sites/{id}/test-http` | `remote_sync_handlers::test_http_site` | 测单站点连通 |
| POST | `/api/remote-sync/envs/import-from-dboption` | `remote_sync_handlers::import_env_from_dboption` | 从 DbOption 导入环境 |
| GET | `/api/remote-sync/logs` | `remote_sync_handlers::list_logs` | 同步日志表 |
| GET | `/api/remote-sync/stats/daily` | `remote_sync_handlers::daily_stats` | 每日统计 |
| GET | `/api/remote-sync/stats/flows` | `remote_sync_handlers::flow_stats` | 流向统计 |
| GET | `/api/remote-sync/sites/{id}/metadata` | `remote_sync_handlers::get_site_metadata` | 站点元数据 |
| GET | `/api/remote-sync/sites/{id}/files` | `remote_sync_handlers::serve_site_files_root` | 站点文件列表根 |
| GET | `/api/remote-sync/sites/{id}/files/{*path}` | `remote_sync_handlers::serve_site_files` | 站点文件代理 |
| GET | `/api/remote-sync/topology` | `remote_sync_handlers::get_topology` | 拓扑图数据 |

## 5. 部署站点管理 · `/api/deployment-sites/*`（plant 原生）

| Method | Path | Handler | 说明 |
|---|---|---|---|
| POST | `/api/deployment-sites/import-dboption` | `handlers::import_deployment_site_from_dboption` | 从 DbOption 导入站点 |
| GET | `/api/deployment-sites` | `handlers::list_deployment_sites` | 站点列表 |
| POST | `/api/deployment-sites` | `handlers::create_deployment_site` | 创建站点 |
| GET | `/api/deployment-sites/{id}` | `handlers::get_deployment_site` | 单站点 |
| PUT | `/api/deployment-sites/{id}` | `handlers::update_deployment_site` | 更新 |
| DELETE | `/api/deployment-sites/{id}` | `handlers::delete_deployment_site` | 删除 |
| GET | `/api/deployment-sites/{id}/tasks` | `handlers::list_deployment_site_tasks` | 任务列表 |
| POST | `/api/deployment-sites/{id}/healthcheck` | `handlers::healthcheck_deployment_site` | 健康检查 |
| GET | `/api/deployment-sites/{id}/export-config` | `handlers::export_deployment_site_config` | 导出配置 |

## 6. 其他辅助

| Method | Path | Handler | 说明 |
|---|---|---|---|
| GET | `/api/sync-status` | `litefs_handlers::sync_status` | LiteFS 同步状态 |
| GET | `/api/site/identity` | `handlers::api_get_site_identity` | 当前站点身份 |
| GET | `/api/sites` | `handlers::api_get_deployment_sites` | 部署站点简列 |

## 已知限制与后续回填计划

| 限制 | 影响 | 回填计划 |
|---|---|---|
| Phase 1.1 的 `save_site_config` / `restart_server` 无 `shutdown_tx` | 保存配置后需手动重启 | AppState 追加 `shutdown_tx` + axum graceful shutdown 接入 |
| Phase 1.1 的 `reload_site_config` 是 stub | 热重载不生效 | 迁入 `config_reload_manager` + `sync_control_center::get_location` |
| Phase 1.3a 的 7 个 MQTT 订阅/主从 handler 简化 | 无法真正切换主从、无 broker logs | 迁入 `check_is_master_node`/`get_available_master_nodes`/`SYNC_EVENT_TX`/`sse_handlers::SyncEvent::MqttSubscriptionStatusChanged` |
| `compute_branch_layout_result` 被 `mbd-iso` feature gate | MBD `layout_result` 字段降级 None | 等 rs-core 补 `aios_core::mbd::iso_extras/iso_params/SolveBranchInput` |
| `/api/remote-sync/*` 全部 admin-gated | 未配 ADMIN_USER/PASS 时 503 | 前端增加 admin login flow（待 Phase 5+）|

## 总计

| 功能域 | 端点数 |
|---|---|
| 站点配置 | 7 |
| MQTT 监控 + 订阅 | 13 |
| 同步服务控制 | 23 |
| 异地环境与站点（admin-gated）| 26 |
| 部署站点管理 | 9 |
| 其他 | 3 |
| **合计** | **81** |
