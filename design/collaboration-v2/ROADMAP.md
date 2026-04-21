# 异地协同 v2 · 端到端路线图

> 范围：**整个协同模块**（前端 + 后端 + DB + 部署 + 测试），非仅前端重构。
>
> 起点：2026-04-21（DEVELOPMENT_PLAN.md 前端 Phase 1–6、8 已完成）
>
> 终点：**`/admin/#/collaboration` 达到 v0.4 原型同等能力，且能在生产部署中承载真实站点流量**。

## 现状盘点

| 层 | 已完成 | 未完成 |
|---|---|---|
| **前端** | token · TopologyPanel · 3 新组件 · 类型+store 扩展 · 暗色主题 | Workbench 4-Tab 重构（P7）· 实时通道（P9）· 文档（P10） |
| **后端 API** | envs/sites/logs CRUD · 诊断 · 运行时 · 元数据 · topology · 统计 | ActiveTasks · FailedTasks · Config · Events SSE · TaskControl |
| **数据库** | remote_sync_envs / sites / logs 3 表 | remote_sync_tasks · remote_sync_failed_tasks · remote_sync_env_config |
| **实时通道** | — | SSE 或 WebSocket |
| **部署** | admin SPA 构建到 static/admin · web_server 提供 API | - |
| **测试** | — | 前端 E2E · API 冒烟 · 断线重连 · 并发 |

## 4 个里程碑

```
M1 前端自成体系           (纯前端, 无后端依赖)
 ├─ Phase 7: Workbench 4-Tab 重构
 └─ Phase 10: CHANGELOG + 架构文档更新
 ▼
M2 后端扩展              (Rust 后端)
 ├─ 后端 B1: 任务队列 API
 ├─ 后端 B2: 失败任务重试 API
 ├─ 后端 B3: 协同组参数配置 API
 └─ 后端 B4: SQLite schema migration (+3 表)
 ▼
M3 实时通道              (Phase 9, 前后端协同)
 ├─ 后端 B5: SSE /events/stream
 └─ 前端 F1: useCollaborationStream composable
 ▼
M4 部署与验证
 ├─ 测试 T1: 3 站点 mock 部署（site-main + 2 peer）
 ├─ 测试 T2: 断线重连冒烟
 ├─ 测试 T3: 失败任务重试链路冒烟
 └─ 发版 R1: CHANGELOG + tag v2.0
```

### M1 · 前端自成体系（本轮立即执行）

**目标**：v2 UI 全量上线，admin 用户看到的 `/collaboration` 页面达到 v0.4 原型效果。

**Phase 7 · Workbench 4-Tab 重构**

- 删除 `CollaborationWorkbenchView.vue` 现有的一页式堆叠
- 引入 4 个 Tab 壳（Topology / Sites / Insights / Logs），URL hash 同步
- 每个 Tab 渲染对应 Panel，ActiveTasks / FailedTasks / ConfigDrawer 挂到对应位置
- Sidebar（GroupListPane）不变
- Header（GroupDetailHeader）不变

**Phase 10 · 文档**

- 更新 `docs/development/admin/异地协同功能架构文档.md` §5 组件架构章节
- `CHANGELOG.md` 写一条 `- admin/#/collaboration v2 重构: 4-Tab 壳 + 活跃任务条 + 失败重试 + 参数配置抽屉 + 暗色主题`

**验收标准**：
- `npm run build` 通过，bundle 合理（gzip < 25 kB）
- 浏览器访问 `/admin/#/collaboration#topo` `#sites` `#insight` `#logs` 均可切换
- `?theme=dark` 下暗色生效

**估时**：Phase 7 · 40 分钟，Phase 10 · 20 分钟

### M2 · 后端扩展

> **✅ 全部落地（2026-04-21）**：`src/web_server/remote_sync_handlers.rs` 追加 ~400 行。
> - 7 个 handler (`list_active_tasks` / `abort_active_task` / `list_failed_tasks` / `retry_failed_task` / `cleanup_failed_tasks` / `get_env_config` / `update_env_config`)
> - 3 新表 + 3 索引加入 `run_schema_migration()`（`Once` 守卫，不重复执行）
> - 新路由已纳入 `create_remote_sync_routes()`，与现有 admin 认证中间件链路一致
> - 类型字段严格对齐前端 `types/collaboration.ts` 的 `CollaborationActiveTask` / `CollaborationFailedTask` / `CollaborationConfig`
> - `cargo check --bin web_server --features web_server` 通过（38.94s，仅上游依赖 warning）

**B1 · 任务队列 API**
```
GET  /api/remote-sync/tasks/active          → CollaborationActiveTask[]
POST /api/remote-sync/tasks/:id/abort       → { status, message }
```

**B2 · 失败任务**
```
GET  /api/remote-sync/tasks/failed?status=pending|exhausted  → CollaborationFailedTask[]
POST /api/remote-sync/tasks/failed/:id/retry                 → { status, next_retry_at }
DELETE /api/remote-sync/tasks/failed?exhausted=true          → { cleaned: N }
```

**B3 · 协同组参数配置**
```
GET /api/remote-sync/envs/:id/config    → CollaborationConfig
PUT /api/remote-sync/envs/:id/config    → { status }
```

**B4 · SQLite schema** (`run_schema_migration` 中追加)
```sql
CREATE TABLE IF NOT EXISTS remote_sync_tasks (
  task_id        TEXT PRIMARY KEY,
  env_id         TEXT NOT NULL,
  site_id        TEXT,
  task_name      TEXT NOT NULL,
  file_path      TEXT,
  progress       REAL DEFAULT 0,
  status         TEXT NOT NULL,          -- Pending / Running / Completed / Failed / Cancelled
  started_at     TEXT,
  updated_at     TEXT NOT NULL,
  FOREIGN KEY (env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS remote_sync_failed_tasks (
  id              TEXT PRIMARY KEY,
  task_type       TEXT NOT NULL,         -- DatabaseQuery / Compression / IncrementUpdate / MqttPublish
  env_id          TEXT NOT NULL,
  site_id         TEXT,
  site_name       TEXT,
  error           TEXT NOT NULL,
  retry_count     INTEGER DEFAULT 0,
  max_retries     INTEGER DEFAULT 5,
  first_failed_at TEXT NOT NULL,
  next_retry_at   TEXT,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  FOREIGN KEY (env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS remote_sync_env_config (
  env_id                TEXT PRIMARY KEY,
  auto_detect           INTEGER DEFAULT 1,
  detect_interval       INTEGER DEFAULT 30,
  auto_sync             INTEGER DEFAULT 0,
  batch_size            INTEGER DEFAULT 10,
  max_concurrent        INTEGER DEFAULT 3,
  reconnect_initial_ms  INTEGER DEFAULT 1000,
  reconnect_max_ms      INTEGER DEFAULT 30000,
  enable_notifications  INTEGER DEFAULT 1,
  log_retention_days    INTEGER DEFAULT 30,
  updated_at            TEXT NOT NULL,
  FOREIGN KEY (env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
);
```

**估时**：B1+B2+B3+B4 · 3 小时

### M3 · 实时通道

> **F1 前端 composable 已落地 ✅ (2026-04-21)**：
> - `ui/admin/src/composables/useCollaborationStream.ts` 新增（~180 行）
> - Workbench `onMounted` 自动连接，`realtimeConnected` 驱动 ONLINE 徽标
> - `?mock=sse` 或 `?dev=1` 或 `import.meta.env.DEV` 走 **本地 mock**：每 4s 推 active_task_update、每 32s 推 failed_task_new、每 60s 推 sync_completed
> - 生产模式连 `/api/remote-sync/events/stream`，断线指数退避重连（消费 `reconnect_initial_ms` / `reconnect_max_ms`）
> - 剩余：B5 后端 SSE 产出真实事件后即可切换；前端零改动

**B5 · 后端 SSE** ✅ (2026-04-21)
> - `remote_sync_handlers.rs` 新增 `RemoteSyncEvent` 枚举（6 种事件类型）
> - `REMOTE_SYNC_EVENT_TX`: `Lazy<broadcast::Sender>` 容量 256
> - `emit_remote_sync_event()` 公开函数供其他模块发送事件
> - `remote_sync_events_stream` handler: `BroadcastStream` → `Sse::new` + `KeepAlive(15s)`
> - 路由已注册: `GET /api/remote-sync/events/stream`
> - `cargo check` 通过

```
GET /api/remote-sync/events/stream         → text/event-stream
```
事件类型 (JSON payload)：
```
data: {"type":"active_task_update","task":{...}}
data: {"type":"failed_task_new","task":{...}}
data: {"type":"site_status_change","site_id":"sjz","detection_status":"Syncing","progress":42}
data: {"type":"sync_completed","site_id":"sjz","file_count":12}
data: {"type":"sync_failed","site_id":"gz","error":"..."}
data: {"type":"keepalive"}
```

**F1 · 前端 composable** `useCollaborationStream.ts`
- 包装 `EventSource`
- 订阅事件 → 调 store action 更新
- 断线指数退避重连（消费 `collabConfig.reconnect_*_ms`）
- `realtimeConnected` 跟随

**估时**：前端 60 min + 后端 60 min

### M4 · 部署与验证

**T1 · 3 站点 mock 部署**
- 本地起 3 个 web_server 实例 + 1 个 mqtt broker
- `site-main` 作为本站，`site-peer-1` / `site-peer-2` 作为 peer
- 各自 DbOption.toml 指向同一个 MQTT broker 和 file server

**T2 · 断线重连冒烟**
- 手动 `docker stop mqtt`
- 前端 `ONLINE → OFFLINE`，5s 内尝试重连
- 恢复后 `OFFLINE → ONLINE`，`realtimeConnected=true`

**T3 · 失败任务重试冒烟**
- 让 `site-peer-2` 的 file server 返回 500
- 触发增量检测 → 失败任务进队列
- 前端「重试」按钮 → 发 `POST /tasks/failed/:id/retry` → 后端调度重试

**R1 · 发版**
- `CHANGELOG.md` v2.0 条目
- git tag `v2.0-collaboration`

**估时**：T1 + T2 + T3 + R1 · 3 小时

## 总估时

| 里程碑 | 估时 |
|---|---|
| M1 前端自成体系 | **1 小时** |
| M2 后端扩展 | 3 小时 |
| M3 实时通道 | 2 小时 |
| M4 部署验证 | 3 小时 |
| **合计** | **9 小时** |

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| Phase 7 破坏 admin 现有页面 | git commit 每步原子 · vue-tsc + build 双检 · 保留原 Workbench 为 `CollaborationWorkbenchView-v1.vue` 注释版本以便快速回滚 |
| M2 后端 API 契约和前端 stub 不一致 | 在本 ROADMAP 里已锁定契约；后端实现时对着 types/collaboration.ts 核对 |
| M3 SSE 在某些反向代理下不工作 | 后端响应 `Cache-Control: no-cache` + `X-Accel-Buffering: no`；前端降级到 30s 轮询 |
| M4 部署环境有 firewall | 先在开发机上 docker-compose 起 3 实例验证；再上预发 |

## 本轮执行（立即）

按 ROADMAP 的 **M1**（Phase 7 + Phase 10）作为本轮目标。
