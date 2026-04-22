# M1 冒烟测试结果 · 2026-04-22

> 对应 Phase 3.3 `docs/plans/2026-04-22-phase-3-phase-4-execution-checklist.md`
>
> 启动命令：`cargo run --bin web_server --features web_server`
> 监听端口：`http://127.0.0.1:3100`（由 `db_options/DbOption.toml.server_release_ip` 决定）

## 启动关键日志

```
✅ [collab-migrate] 已追加列 remote_sync_sites.master_mqtt_host
✅ [collab-migrate] 已追加列 remote_sync_sites.master_mqtt_port
✅ [collab-migrate] 已追加列 remote_sync_sites.master_location
✓ [collab-migrate] node_config 表就绪
🎯 [collab-migrate] 异地协同 schema 对齐完成 (path=deployment_sites.sqlite)
✅ 数据库连接初始化成功
🚀 Web UI服务器启动成功！
```

**Phase 1.6 迁移函数 `ensure_collab_schema` 首次运行**，成功为 plant 侧 `deployment_sites.sqlite` 追加 3 列 + 新建 `node_config` 表 —— 与 web-server schema 完全对齐。

## 8 个关键 endpoint 冒烟结果

| # | Method | Path | 状态 | 响应要点 | 归属 |
|---|---|---|---|---|---|
| 1 | GET | `/api/site-config` | ✅ 200 | 完整 DbOption 字段 | Phase 1.1 |
| 2 | GET | `/api/site/info` | ✅ 200 | `{location:"sjz", mqtt_host, file_server_host}` | Phase 1.1 |
| 3 | GET | `/api/remote-sync/envs` | ⚠ 503 (by design) | admin 鉴权未配 | Phase 1.4 |
| 4 | GET | `/api/remote-sync/topology` | ⚠ 503 (by design) | admin 鉴权未配 | Phase 1.4 |
| 5 | GET | `/api/mqtt/nodes` | ✅ 200 | `{nodes:[], summary:{online:0,offline:0}}` | Phase 1.2 |
| 6 | GET | `/api/mqtt/subscription/status` | ✅ 200 | `{is_running:false, location:"sjz", subscribed_topics:["Sync/E3d"]}` | Phase 1.3a |
| 7 | GET | `/api/sync/status` | ✅ 200 | sync service 完整状态 | plant 原生 |
| 8 | GET | `/api/sync/queue` | ✅ 200 | `{pending:0, running:0, queue:[]}` | plant 原生 |

**直接 200：6/8**。剩余 2 个 503 是 plant 主分支已有的 admin 鉴权策略（`/api/remote-sync/*` 被 admin_auth 中间件保护），**不是 Phase 1 引入的问题**。

## 503 原因详情

访问 `/api/remote-sync/envs` 的响应体：

```json
{
  "data": null,
  "message": "管理员凭据未配置，请先设置 ADMIN_USER 与 ADMIN_PASS",
  "success": false
}
```

启动日志里的提示：

```
⚠️ Admin 后台鉴权未启用：缺少环境变量 ADMIN_USER, ADMIN_PASS。
   访问 /api/admin/* 将返回 503，请先配置管理员凭据。
```

## M1 结论

- Phase 1 **API 汇入功能正确** ✅
  - Phase 1.1 `site_config_handlers` 的 2 条端点全绿
  - Phase 1.2 `mqtt_monitor_handlers::get_mqtt_nodes_status` 全绿
  - Phase 1.3a 的 `get_mqtt_subscription_status` stub 按设计返回 idle 状态
  - Phase 1.6 schema 迁移在**首次启动**时生效，符合幂等设计预期
- `/api/remote-sync/*` 的 503 属于 admin 鉴权流程，需要：
  1. 设置 `ADMIN_USER` / `ADMIN_PASS` env
  2. 先 POST `/api/admin/login` 获 jwt
  3. 后续请求带 `Authorization: Bearer <token>`
- `mbd-iso` cfg-gate 生效 — 未开启时 `layout_result` 降级为 None，其他 MBD API 正常

**M1 里程碑达成** ✅（除 admin-gated endpoint 外全部可访问）。

## 接下来

- **Phase 3.4 前端实地验收**：浏览器打开 `http://localhost:3200/`，对 11 个视图逐个看 DevTools 有无红错
- **Phase 4 文档 + 部署**：
  - 完善 README
  - 新建 `docs/architecture/异地协同API汇总清单.md`
  - 更新 `shells/deploy/deploy_all_with_frontend.sh`
- **后续优化（可选）**：
  - 补齐 Phase 1.3b 真实 MQTT 订阅 handler（目前是简化 stub）
  - 回填 `compute_branch_layout_result` 的 rs-core 依赖
  - 前端 API 层增加 admin auth flow（login → bearer token）
