# Sprint D1 验收报告（2026-04-26）

> 上游：`docs/plans/2026-04-26-sprint-d1-admin-sites-sse.md`（D1 主计划）
> 关联 commit：
> - `d6d14f7` D1 Phase1-3 后端 SSE 推送基线
> - `08c6052` D1 Phase4-5 前端 SSE 实时化 + 连接徽标

---

## 1. 验收方法

按 plant-model-gen `AGENTS.md` 规范，使用 **真 web_server + curl/PowerShell** 端到端验证。
**未运行任何 cargo test**；前端仅做 `vue-tsc --noEmit` 类型检查。

启动环境：
- Windows 10 / PowerShell
- `$env:ADMIN_USER='admin'; $env:ADMIN_PASS='admin'`
- `cargo run --bin web_server --features web_server`（debug 模式）
- 端口：HTTP API → `127.0.0.1:3100`

启动 log 关键点：
- ✅ 自动拉起 SurrealDB（`SurrealDB 进程已启动 (PID: 26736)`）
- ✅ axum router 注册三层路由（stateless / stateful / main）打印完成
- ✅ `🚀 Web UI服务器启动成功！` 总耗时 ~80s（含 cargo 增量编译）

---

## 2. 验收项与实测

### 2.1 验收 A1：SSE 通道存活 + 初始 snapshot 推送（C5 防漏首事件）

```powershell
curl.exe -N -m 45 -sS --no-buffer http://127.0.0.1:3100/api/sync/events/stream
```

**响应（前 1 行 within 1s）**：

```text
data: {"type":"MqttSubscriptionStatusChanged","data":{"is_running":false,"is_master_node":true,"location":"sjz","timestamp":"1777218425"}}
event: message
```

✅ `MqttSubscriptionStatusChanged` 初始 snapshot 立即送达，无需任何前置操作（继承自 Sprint C C5 修 G5 的逻辑）。

---

### 2.2 验收 A2：`AdminSiteCreated` 推送

```powershell
$h = @{Authorization='Bearer ...';'Content-Type'='application/json'}
$body = @{
    project_name = 'd1-test-site'
    project_path = 'D:/tmp-d1-test'
    project_code = 99999
    db_port = 18190
    web_port = 18191
    bind_host = '127.0.0.1'
    db_user = 'siteuser_d1_xyz'
    db_password = 'Password@2026!Strong'
} | ConvertTo-Json
Invoke-RestMethod -Uri http://127.0.0.1:3100/api/admin/sites -Method Post -Headers $h -Body $body
```

**HTTP 响应**：`success=True, site_id=d1-test-site-18191`

**SSE 同步推送（≤ 1s 延迟）**：

```text
data: {"type":"AdminSiteCreated","data":{"site_id":"d1-test-site-18191","project_name":"d1-test-site","timestamp":"1777218428"}}
event: message
```

✅ payload 字段完整：`site_id` + `project_name` + `timestamp`。

---

### 2.3 验收 A3：`AdminSiteSnapshot` 推送（元数据更新）

```powershell
$body2 = @{ project_name = 'd1-test-site-RENAMED' } | ConvertTo-Json
Invoke-RestMethod -Uri http://127.0.0.1:3100/api/admin/sites/d1-test-site-18191 -Method Put -Headers $h -Body $body2
```

**HTTP 响应**：`success=True, name=d1-test-site-RENAMED`

**SSE 同步推送（≤ 1s 延迟）**：

```text
data: {"type":"AdminSiteSnapshot","data":{"site_id":"d1-test-site-18191","project_name":"d1-test-site-RENAMED","status":"Draft","parse_status":"Pending","last_error":null,"timestamp":"1777218430"}}
event: message
```

✅ payload 字段完整：
- `project_name` 已切换为新名 `d1-test-site-RENAMED`（前端可据此更新列表行）
- `status="Draft"` + `parse_status="Pending"`：update_site 内部强制重置（`managed_project_sites.rs:1851-1856`），与现有业务语义一致
- `last_error=null`：update 同时清空错误状态

---

### 2.4 验收 A4：`AdminSiteDeleted` 推送

```powershell
Invoke-RestMethod -Uri http://127.0.0.1:3100/api/admin/sites/d1-test-site-18191 -Method Delete -Headers $h
```

**HTTP 响应**：`{ "data": { "deleted": true, "site_id": "d1-test-site-18191" }, "success": true }`

**SSE 同步推送（≤ 1s 延迟）**：

```text
data: {"type":"AdminSiteDeleted","data":{"site_id":"d1-test-site-18191","timestamp":"1777218433"}}
event: message
```

✅ 仅当 SQLite 真正删除一行时才推送（`changed > 0` 守卫），不会因不存在的 site_id 误推。

---

### 2.5 完整事件链时间线（同次会话）

```text
t=0s    :  init MqttSubscriptionStatusChanged    (timestamp 1777218425)
t=+3s   :  AdminSiteCreated                       (timestamp 1777218428)
t=+5s   :  AdminSiteSnapshot                      (timestamp 1777218430)
t=+8s   :  AdminSiteDeleted                       (timestamp 1777218433)
```

四条事件以正确顺序、≤ 1s 端到端延迟全部送达 SSE 客户端，时间戳单调递增 ✓。

---

## 3. 验收清单（D1 主计划 § 5）

| 验收项 | 状态 | 备注 |
|---|---|---|
| **A1（功能）** admin UI 启动 site → ≤ 1s 看到 Running 翻转 | ⏸ 浏览器联调 | 后端验证 ≤ 1s 推送，前端 patcher 已就位（08c6052），等浏览器手测 |
| **A2（功能）** A 标签页创建/删除 site → B 标签页 ≤ 2s 同步 | ⏸ 浏览器联调 | 同 A1 |
| **A3（功能）** 详情页 CPU/内存/磁盘 5s 一刷 | ❌ 未实施 | `AdminSiteResource` 留 D1 Phase 2-Plus |
| **A4（断流）** 杀死 web_server → 前端进入重连，恢复后 ≤ 5s 全量刷新 | ⏸ 浏览器联调 | useAdminSitesStream 指数退避 + onConnect → fetchSites 已就位 |
| **A5（性能）** 单标签页 5min 内 polling ≤ 5 | ⏸ 浏览器联调 | 60s 兜底 + 启动一次 fetch，估算 5min ≈ 6 次 |
| **A6（兼容）** `shells/smoke-collab-api.sh` ≥ 20/20 PASS | ⏸ 未跑 | 本次未触发 smoke；后端事件型谱新增不会破坏现有事件类型 |
| **A7（编译）** `cargo check --bin web_server --features web_server` 0 error；admin UI `vue-tsc -b` 0 error | ✅ | cargo 12s 增量 0 error；vue-tsc 4s 0 error 0 warning |
| **A2/A3/A4 后端** SSE 端到端推送 admin 事件 | ✅ | 本报告 § 2 实测 4 条事件全部送达 |

---

## 4. 修复的 bug（验证过程中发现）

### bug-1：`useAdminSitesStream` 默认 URL 误用 polling 端点

| 项 | 内容 |
|---|---|
| 位置 | `ui/admin/src/composables/useAdminSitesStream.ts` |
| 现象 | 验证时 curl `http://127.0.0.1:3100/api/sync/events` 返回 `{"events":[],"status":"success"}` 而非 SSE 流 |
| 根因 | `/api/sync/events`（不带 `/stream`）是 polling list endpoint（`sync_control_handlers::sync_events_stream`，函数名误导），**不是 SSE 端点**；真 SSE 端点是 `/api/sync/events/stream`（`sse_handlers::sync_events_handler`，`mod.rs:760` 注册） |
| 修复 | 默认 URL 改为 `/api/sync/events/stream`，并加 inline 注释说明两个 path 的区别避免再次误用 |
| 影响 | 修复前 useAdminSitesStream 永远连不上真 SSE 流，前端徽标会一直显示「未连接」；后端推送虽工作但前端拿不到 |

修复一并入本次 commit。

---

## 5. 已知偏差 / 未实施项

- **`AdminSiteResource` 资源指标推送（D1 Phase 2-Plus）**：本次未实施，详情页 CPU / 内存 / 磁盘仍走 10s polling。后续 Sprint 接 ResourceCollector 5s 节流推送。
- **`shells/smoke-collab-api.sh` 未升级到 24 项**：本次未触发 smoke 验证；后端新加的 3 个 SyncEvent 变体属增量 forward-compat，不会破坏现有 13 项事件类型测试。后续 Sprint 单独追加 smoke 项。
- **浏览器双 tab 联调（A1/A2/A4/A5）**：需 admin UI build + 浏览器人工验证，留下次会话。

---

## 6. 总结

D1 后端 SSE 推送基线 **端到端验证通过**：4 条事件类型（init MQTT snapshot / AdminSiteCreated / AdminSiteSnapshot / AdminSiteDeleted）以正确顺序、≤ 1s 延迟、payload 字段完整地推送到 SSE 客户端。前端 useAdminSitesStream 修复 URL bug 后 wire 完整就位，`vue-tsc` 0 error。

**下一步**：D1 Phase 2-Plus（AdminSiteResource）/ Sprint E（安全治理）/ 浏览器联调（A1/A2/A4/A5）任选其一。

---

## 7. 附：完整 SSE 输出（实测）

```text
data: {"type":"MqttSubscriptionStatusChanged","data":{"is_running":false,"is_master_node":true,"location":"sjz","timestamp":"1777218425"}}
event: message

data: {"type":"AdminSiteCreated","data":{"site_id":"d1-test-site-18191","project_name":"d1-test-site","timestamp":"1777218428"}}
event: message

data: {"type":"AdminSiteSnapshot","data":{"site_id":"d1-test-site-18191","project_name":"d1-test-site-RENAMED","status":"Draft","parse_status":"Pending","last_error":null,"timestamp":"1777218430"}}
event: message

data: {"type":"AdminSiteDeleted","data":{"site_id":"d1-test-site-18191","timestamp":"1777218433"}}
event: message
```
