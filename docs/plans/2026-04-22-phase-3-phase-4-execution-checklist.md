# Phase 3 + Phase 4 执行清单 · M3 端到端联通 + 文档部署

> 父计划：`docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md`
>
> 前置：Phase 1 代码层完成（7 commit），Phase 2 新前端全栈完成（3 commit），mbd-iso cfg-gate 已解锁 M1 冒烟。
>
> 本阶段产出 M3（端到端闭环）+ M4（文档 + 部署就绪）。

## Phase 3 · M3 端到端联通（估时 2h）

### 3.0 解锁前提（已完成 ✅）

- [x] `fix(mbd): cfg-gate compute_branch_layout_result` — 未开启 `mbd-iso` feature 时函数不参与编译
- [x] `cargo check --features web_server` 零 error

### 3.1 启动 plant-model-gen 后端

```bash
cd D:\work\plant-code\plant-model-gen
cargo run --bin web_server --features web_server
```

- [ ] 观察首屏日志：出现 `🎯 [collab-migrate] 异地协同 schema 对齐完成` 和 `Listening on 0.0.0.0:9099`（默认端口由 `DbOption.toml.server_release_ip` 决定）
- [ ] 若首次启动耗时较长（cargo link），耐心等 30-120s
- [ ] 如崩溃，查看 stack trace 决定是否要其他 stub

### 3.2 启动 plant-collab-monitor 前端（若已起则跳过）

```bash
cd D:\work\plant-code\plant-collab-monitor
npm run dev
```

- [ ] 访问 `http://localhost:3200/` 首页 HTTP 200
- [ ] 侧栏 3 分组 11 条路由可切换

### 3.3 关键 API 冒烟（M1 + M3）

对 `http://127.0.0.1:9099`（后端原生，不经前端 proxy）逐条 curl：

| # | Method | Path | 期望 | 关联 Phase |
|---|---|---|---|---|
| 1 | GET | `/api/site-config` | 200 · `{status:"success", config:{…}}` | 1.1 |
| 2 | GET | `/api/site/info` | 200 · 包含 `location`、`mqtt_host` | 1.1 |
| 3 | GET | `/api/remote-sync/envs` | 200 · array | 1.4（plant 原生）|
| 4 | GET | `/api/remote-sync/topology` | 200 · 包含站点拓扑 | 1.4 |
| 5 | GET | `/api/mqtt/nodes` | 200 · array | 1.2 |
| 6 | GET | `/api/mqtt/subscription/status` | 200 · `{is_running, location}` | 1.3a |
| 7 | GET | `/api/sync/status` | 200 · sync service 状态 | plant 原生 |
| 8 | GET | `/api/sync/queue` | 200 · tasks | plant 原生 |
| 9 | GET | `/api/sync/events/stream` | SSE 200 · `Content-Type: text/event-stream` | plant 原生 |

**M1 = 端点全绿** · **M3 = 通过前端 proxy 访问也全绿**（即 `http://localhost:3200/api/...`）

### 3.4 前端 11 视图实地验收

以浏览器打开 `http://localhost:3200/` 逐页过：

| # | Path | 验收点 |
|---|---|---|
| 1 | `/dashboard` | 点"拉取后端状态"能显示 `/api/sync/status` JSON |
| 2 | `/topology` | TopologyManager 组件加载完成（可能依赖 `/api/remote-sync/*`，能显示空状态）|
| 3 | `/topology-viz` | SVG 区域可见（空数据也应有占位）|
| 4 | `/tasks` | TaskQueue 组件显示，点"刷新"调 `/api/sync/queue` |
| 5 | `/history` | SyncHistory 组件显示，点"刷新"调 `/api/sync/history` |
| 6 | `/mqtt/messages` | MqttMessageViewer 加载 |
| 7 | `/mqtt/nodes` | MqttNodeMonitorEnhanced 加载 |
| 8 | `/logs` | LogViewer 组件显示，SSE EventSource 建连 |
| 9 | `/archives` | ArchivesManager 加载 |
| 10 | `/site-config` | SiteConfig 表单展示 DbOption.toml 的所有字段 |
| 11 | `/settings` | SettingsManager 加载 |

**发现的问题全部记录在 `plant-collab-monitor/BUGS.md`**（按 P0/P1/P2 分级）

### 3.5 SSE + WebSocket 实时通道验证

- [ ] 在后端手动触发一次 sync 事件（或 tail log），确认前端 LogsView 的 EventSource 能收到推送
- [ ] WebSocket `/ws/tasks` 若存在，补 composable `useWebSocket` 验证

### 3.6 前端构建产物自检

```bash
cd D:\work\plant-code\plant-collab-monitor
npm run build
```

- [ ] `dist/` 目录生成
- [ ] 主 bundle < 2 MB（目前 1.48 MB / 408 KB gzipped）
- [ ] 纯静态（index.html + assets）能用静态 server 单独预览

### 3.7 M3 里程碑 commit

```bash
# plant-model-gen
git commit -m "docs(collab): M3 端到端联通验证通过 · API 冒烟 9/9"
# plant-collab-monitor
git commit -m "docs: M3 · 11 视图实地验收通过 + BUGS 清单"
```

## Phase 4 · 文档 + 部署（估时 2h）

### 4.1 plant-collab-monitor 完善文档

`plant-collab-monitor/README.md` 增补段落：

- [ ] 环境要求（Node ≥ 20）
- [ ] 快速开始：`npm i && npm run dev`
- [ ] 环境变量：`VITE_API_TARGET` / `VITE_API_BASE` 说明
- [ ] 生产构建 + Nginx 反代示例
- [ ] 与 `plant-model-gen/ui/admin/#/collaboration` 的定位差异

### 4.2 plant-model-gen 文档更新

- [ ] `docs/architecture/异地协同API汇总清单.md`（**新建**）
  - 按功能分组列出 40+ endpoint
  - 每条标注 Method / Path / Handler 源文件 / Request / Response sample
  - 明确标记 stub / TODO（1.3a 的简化 handler / 1.1 的 reload 降级）

- [ ] `docs/架构文档/异地协同架构.md`（**更新 / 或新建**）
  - 新增一节「plant-collab-monitor 位置」
  - 三角架构图（web-server legacy · plant-model-gen 后端 · plant-collab-monitor + admin 双前端）

- [ ] `CHANGELOG.md` 新增一条
  - `feat(collab): 异地协同 API 汇入 plant-model-gen · 从 web-server 剥离独立前端 plant-collab-monitor`
  - 列出 Phase 1 新增路由、Phase 2 新前端、已知 stub

### 4.3 部署脚本

- [ ] 更新 `plant-model-gen/shells/deploy/deploy_all_with_frontend.sh`
  - 支持打包 plant-collab-monitor：`cd ../plant-collab-monitor && npm ci && npm run build`
  - 把 `dist/` 同步到目标服务器（`/var/www/plant-collab-monitor/`）
  - 可通过 env `COLLAB_MONITOR_DIR` 覆盖源目录

- [ ] 新增 Nginx 反代示例片段到 `shells/deploy/nginx-plant-collab-monitor.conf`（注释型）：

```nginx
location /monitor/ {
  alias /var/www/plant-collab-monitor/;
  try_files $uri $uri/ /monitor/index.html;
}
location /api/ {
  proxy_pass http://127.0.0.1:9099/api/;
  proxy_set_header Host $host;
  proxy_set_header X-Real-IP $remote_addr;
}
location /ws/ {
  proxy_pass http://127.0.0.1:9099/ws/;
  proxy_http_version 1.1;
  proxy_set_header Upgrade $http_upgrade;
  proxy_set_header Connection "upgrade";
}
```

### 4.4 web-server legacy 标记

- [ ] 在 `D:\work\plant-code\web-server\` 根新建 `MIGRATION_NOTICE.md`
  - 说明异地协同前端已迁出到 `D:\work\plant-code\plant-collab-monitor\`
  - 说明后端 API 已汇入 `D:\work\plant-code\plant-model-gen\`
  - 保留 `frontend/` 目录作为历史参照

### 4.5 最终 commit（Phase 4 · 文档部署）

```bash
# plant-model-gen
git commit -m "docs(collab): Phase 4 · API 汇总清单 + 架构文档 + 部署脚本"
# plant-collab-monitor
git commit -m "docs: Phase 4 · README 增补 + Nginx 反代示例"
# web-server (在 main 或 feature 分支)
git commit -m "docs: MIGRATION_NOTICE · 异地协同已迁出"
```

## 完成条件

- [ ] Phase 3.3 所有 9 个 curl 通过（至少返回合法 HTTP 200 JSON，即使 stub 也可）
- [ ] Phase 3.4 前端 11 视图至少能加载、无控制台红色 error
- [ ] Phase 4 三个文档就绪
- [ ] 部署脚本在本地至少能 `--dry-run`

→ 整个计划 14h 完工。

## 风险应急

| 风险 | 应对 |
|---|---|
| `cargo run --features web_server` 首次启动超时 | 耐心等 3-5 分钟，或先 `cargo build --features web_server --bin web_server` 预热 |
| 端口 9099 被占用 | `netstat -ano \| findstr 9099` → taskkill 或换端口 `--port 9100` |
| DbOption.toml 里字段缺失导致 handler 崩溃 | `db_options/DbOption.example.toml` 不存在时看 `db_options/DbOption-mac.toml` 对照补齐 |
| 前端某个 view 因 `useApi` 调不存在的 `/api/incremental/*` 报 404 | 临时 `try { ... } catch { }` 包住，不影响其它 view |
| Nginx 反代配置与现有 admin 冲突 | 预留 `/monitor/` 前缀，完全独立于 `/admin/` |

## 参考文档链接

- Phase 1 执行清单：`docs/plans/2026-04-22-phase-1-execution-checklist.md`
- 父计划：`docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md`
- 架构图：`design/collab-consolidation/01-architecture.html`
- 迁移 Swimlane：`design/collab-consolidation/02-api-migration-swimlane.html`
- 同步时序：`design/collab-consolidation/03-sync-sequence.html`
- 甘特时间轴：`design/collab-consolidation/04-roadmap-timeline.html`
