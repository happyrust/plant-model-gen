# 异地协同 Sprint B · 后端 stub 收口计划（2026-04-26）

> 上游：
> - 父计划：`docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md`
> - 异地协同 API 汇总清单（81 endpoint）：`docs/architecture/异地协同API汇总清单.md`
> - 跨仓 collab-monitor 前端 PRD（异地站点）：`../plant-collab-monitor/docs/prd/2026-04-26-remote-site-prd.md`
> - 跨仓 Gap 清单：`../plant-collab-monitor/docs/plans/2026-04-25-collab-monitor-completion-gap.md`
> - 跨仓 Sprint A/C 完成情况：`../plant-collab-monitor/docs/plans/2026-04-26-sprint-bc-plan.md`
> - 跨仓 e2e-smoke 验收报告：`../plant-collab-monitor/docs/e2e-smoke/2026-04-26-e2e-smoke-report.md`

---

## 0. 背景与现状

跨仓 plant-collab-monitor Sprint A + C 已完成 9 commits，关闭 12/14 个 Gap，**仅剩 G6（MQTT 7 个 stub）和 G7（site-config reload/restart stub）属本仓 plant-model-gen 范畴**。

本计划聚焦 plant-model-gen 后端，将 G6/G7 拆解为可独立提交的 7 个子任务（B1-B7），并定义本会话立即执行的 Phase 8（B1+B3+B7 三项最小可验证集），其余 B2/B4/B5/B6 留待后续会话。

---

## 1. Stub 清单与改造方案

### B1 · `set_as_master` / `set_as_client` 写盘（G6）

**位置**：`src/web_server/sync_control_handlers.rs:1046-1070`

**现状**：仅 `log::warn!` 打印，返回 success。前端 MqttNodesView 主从切换按钮点了无效。

**已存在的依赖**：`src/web_server/mqtt_monitor_handlers.rs:646-667` 的 `check_is_master_node_internal` 使用 `node_config` SQLite 表（schema：`location TEXT PRIMARY KEY, is_master BOOLEAN, updated_at TEXT`）。

**改造方案**：
- 提取 `check_is_master_node_internal` 中的表创建逻辑为 `ensure_node_config_table()`（pub(crate)）
- `set_as_master_node`：`INSERT OR REPLACE INTO node_config(location, is_master, updated_at) VALUES (?, 1, datetime('now'))`
- `set_as_client_node`：同上，is_master = 0
- 不写 DbOption.toml（DbOption 中无 is_master 字段；location 由部署期固定）

**估时**：30 min

### B2 · `get_mqtt_broker_logs_api`（G6）✅ Phase 9 完成

**位置**：`src/web_server/sync_control_handlers.rs` + `src/web_server/mqtt_monitor_handlers.rs`

**已落地**（commit Phase 9）：
- `mqtt_monitor_handlers.rs` 新增：
  - `BrokerLogEntry { timestamp, level, event, location, message }`
  - `BROKER_LOG_CAPACITY = 200`
  - `MQTT_BROKER_LOGS: Arc<RwLock<VecDeque<BrokerLogEntry>>>`
  - `push_broker_log(level, event, location, message)` helper
  - `read_broker_logs(limit)` helper（按时间倒序，最新在前）
- 注入 push_broker_log 时机：
  - `update_node_heartbeat` 节点首次上线 / 离线恢复
  - `check_offline_nodes` 节点心跳超时（仅 online→offline 翻转）
  - `update_subscription_status` ConnAck 状态切换
  - `set_as_master_node` / `set_as_client_node` 写盘成功/失败
  - `start_mqtt_subscription_api` / `stop_mqtt_subscription_api`
  - `clear_master_config_api` 清主配置
- `get_mqtt_broker_logs_api` 改造：
  - 支持 `?limit=N`（默认 200，上限 200）
  - 返回 `{ status, count, capacity, logs }`
- `shells/smoke-collab-api.sh` 增加 `?limit=10` + `set_master` 字段命中校验

**估时**：实际 ~1.5h（含编辑+cargo check）

### B3 · `get_mqtt_subscription_status` 字段补齐（G6）

**位置**：`src/web_server/sync_control_handlers.rs:1029-1044`

**现状**：返回 `is_running` / `is_server_running=false` / `location` / `subscribed_topics` 4 字段；但前端 MqttNodesView 期望额外 `is_master_node` / `node_role` / `connection_status` / `master_info` / `mqtt_server_port`。

**改造方案**：
- 调 `mqtt_monitor_handlers::check_is_master_node_internal`（提取为 pub(crate)）
- 计算 `node_role = if is_master_node { "master" } else { "client" }`
- 调 `get_subscribed_master_info(location)` 拿 master_info（已存在）
- 添加 `is_server_running` 真值（从 mqtt broker 内置进程读取）→ 简化先返回 false，B4 再接入
- 添加 `connection_status` 占位 `"connected"` / `"disconnected"`

**估时**：30 min

### B4 · `MqttSubscriptionStatusChanged` SSE 事件（G6）✅ Phase 12 完成

**位置**：`src/web_server/sse_handlers.rs` 已有 SyncEvent 枚举

**已落地**（commit Phase 12）：
- `sse_handlers.rs` `SyncEvent` 新增变体 `MqttSubscriptionStatusChanged { is_running, is_master_node, location, timestamp }`
  - 字段口径与 `GET /api/mqtt/subscription/status` 完全一致
- `sync_control_handlers.rs` 新增 `pub(crate) async fn push_subscription_status_event(location)` helper
  - 内部读 `REMOTE_RUNTIME` + `mqtt_monitor_handlers::check_is_master_node` 计算最新状态
  - 通过 `SYNC_EVENT_TX.send(...)` 广播
- 4 处推送注入：
  - `set_as_master_node`（success）
  - `set_as_client_node`（success）
  - `start_mqtt_subscription_api`（success）
  - `stop_mqtt_subscription_api`（success）
- `shells/smoke-collab-api.sh` 增加 `check_sse` 函数 + `[4/5] SSE 实时事件流` 块，验证 `/api/sync/events/stream` 200
- 前端 `plant-collab-monitor/src/views/LogsView.vue` 已订阅 `/api/sync/events/stream` 自动 prepend，无需改动

**估时**：实际 ~1.5h（含 cargo check 增量编译 ~38s）

### B5 · site-config save 自动 graceful restart（G7）

**位置**：`src/web_server/site_config_handlers.rs:352-355`

**改造方案**：
- AppState 加 `shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>`
- main.rs 用 `axum::Server::serve(...).with_graceful_shutdown(...)`
- save 成功后 `shutdown_tx.take().send(())`
- 进程外 supervisor（systemd / nssm）自动重启即可

**估时**：2d

### B6 · `reload_site_config` 真实现（G7）✅ Phase 11 完成

**位置**：`src/web_server/site_config_handlers.rs:388-406`

**核查结论**（本会话）：plan §1.B6 假设的 `aios_core::set_db_option_from_file` **不存在**；`get_db_option()` 是 `OnceCell::get_or_init`，全局静态不可变；想真热重载必须改 rs-core。

**已落地**（commit Phase 11，遵循 `docs/plans/2026-04-26-sprint-b-phase11-b6-reload.md`）：
- 新增常量 `HOT_RELOADABLE_KEYS`（12 字段白名单：enable_log / mesh_tol_ratio / gen_* / sync_chunk_size 等）
- 新增 `diff_db_option(current, new)` 用 `serde_json::to_value` 做字段级 diff
- `reload_site_config` 重写：
  - 读 `${DB_OPTION_FILE}.toml` → toml::from_str → DbOption
  - 与当前 `aios_core::get_db_option()` diff
  - 返回 `{ status, hot_changed_keys, static_changed_keys, requires_restart, actions, message }`
  - actions ∈ { "no_change", "log_only", "manual_restart_required", "read_failed", "parse_failed" }
- **本版本不真正应用配置变更**（OnceCell 限制）；语义为「字段变更检测 + 用户告知」
- 文件错/解析错走 `status: "error"`，HTTP 200 兼顾前端 UX

**升级路径**：rs-core OnceCell → RwLock<Arc<DbOption>> 后，hot_changed 非空时调 `set_db_option_from_file()`，actions 升级为 `["hot_reloaded"]`。属跨仓改动，留独立会话。

**估时**：实际 ~30 min（较 plan 0.5-1d 缩短，因不动 rs-core）

### B7 · 后端冒烟脚本（B/C 共同退出条件）

**位置**：`shells/smoke-collab-api.sh`（新建）

**改造方案**：
- 16 个核心 endpoint 自动化 curl
- 输出表格：`✓` / `✗` / `503` / 响应字段命中检查
- 用于 Phase 7-Plus 联调启后端后跑 + 集成 CI

**估时**：30 min

---

## 2. Phase 8 · 本会话立即执行（B1 + B3 + B7）

### 范围

最小可独立验证集，避免本会话超出 plant-model-gen 大型工程的合理改动节奏。

| 子任务 | 估时 | 依赖 |
|-------|------|------|
| B1 set_as_master/client 写 node_config | 30 min | 复用 mqtt_monitor_handlers 已有 schema |
| B3 subscription/status 字段补齐 | 20 min | B1 完成后扩展即可 |
| B7 smoke-collab-api.sh | 30 min | 独立 |

**Phase 8 总估时**：~1h

### 退出条件

- B1 改动后 `/api/mqtt/node/set-master` 真正写入 `node_config` 表，再次 `/api/mqtt/subscription/status` 返回 `is_master_node: true`
- B3 改动后 status 返回 5 个新字段（is_master_node / node_role / master_info / connection_status / mqtt_server_port）
- B7 脚本能在后端运行时跑通，至少 8/11 endpoint 绿
- `cargo check --features web_server` 0 errors（按 AGENTS.md 规范，不跑 cargo test）

### Phase 8 不做

- B2 broker logs（涉及 ring-buffer 设计，留下一会话）
- B4 SSE 事件（涉及前端联调验证，留 Phase 9）
- B5/B6 graceful shutdown / reload（涉及 axum 重构 + 模块新建，工作量大）

---

## 3. 完整 Sprint B 时间线（理想节奏）

| Day | Phase | 任务 | 状态 |
|-----|-------|------|------|
| **D1** | Phase 8 | B1 + B3 + B7 | ✅ 完成（commit `94bc86e`） |
| **D2** | Phase 9 | B2 broker logs ring-buffer | ✅ 完成（commit `c3a38ce`） |
| **D3** | Phase 12 | B4 SSE 事件推送（后端侧） | ✅ 完成（commit `5463e41`） |
| **D3** | Phase 11 | B6 reload 最小版（diff + 分类响应） | ✅ 完成（本会话） |
| D4-D5 | Phase 10 | B5 graceful shutdown（main.rs 重构） | 待 |
| D6 | Phase 12-Plus | B4 跨仓前端联调（MqttNodesView 订阅 SSE 自动 reload） | 待 |
| D7 | Phase 11-Plus | B6 真热加载（rs-core OnceCell → RwLock，跨仓改动） | 待 |
| D8 | Phase 7-Plus | 后端联调验收报告 | 待（依赖前述） |

**累计**：~8 人天，已完成 4/7 Phase（本仓 plant-model-gen 范畴）。

---

## 4. 验收

### Phase 8 验收

- [ ] `cargo check --features web_server` 0 errors
- [ ] B1 / B3 改动通过 curl 手测：
      ```bash
      curl -X POST http://127.0.0.1:3100/api/mqtt/node/set-master
      curl http://127.0.0.1:3100/api/mqtt/subscription/status | jq '.is_master_node'
      # → true
      ```
- [ ] B7 脚本 `bash shells/smoke-collab-api.sh` 能跑（可输出错误）

### Sprint B 全部完成验收

- [ ] G6 / G7 全部子任务完成
- [ ] `m1-smoke-test-result.md` 升级为 8/8 绿
- [ ] 跨仓 plant-collab-monitor MqttNodesView 主从切换按钮点击后真生效
- [ ] 跨仓 SettingsView 保存配置后无需手动重启

---

## 5. 风险

| 风险 | 等级 | 缓解 |
|------|------|------|
| `node_config` schema 变更可能与现有 `check_is_master_node_internal` 冲突 | 🟢 低 | 同一 schema，复用即可 |
| B5 graceful shutdown 涉及 main.rs 大改 | 🟡 中 | 拆为 axum upgrade + AppState 加字段两步走 |
| Sprint B 后端 stub 修复后跨仓前端字段不一致 | 🟡 中 | 联调时按 PRD §6 字段速查表对齐 |
| `cargo check` 在 Windows 上首次编译耗时 5-10 min | 🟢 低 | 用增量；本会话只 check 一次最终态 |
| AGENTS.md 规定不跑 cargo test，依赖 CLI/HTTP 验证 | 🟢 低 | 严格遵守；用 curl 验证 B1/B3 |

---

## 6. 立即执行节奏

### Phase 8 ✅ 完成

```
[Phase 8 完成 · commit 94bc86e]
  ├─ 8.1 B1 set_as_master/client 改造          [30 min]
  ├─ 8.2 B3 subscription/status 字段补齐       [20 min]
  ├─ 8.3 B7 smoke-collab-api.sh                [30 min]
  ├─ 8.4 cargo check --features web_server     [5-10 min]
  └─ 8.5 git commit Phase 8                    [ 5 min]
```

### Phase 9 ✅ 完成（本会话）

```
[Phase 9 完成]
  ├─ 9.1 mqtt_monitor_handlers.rs 加 ring-buffer + helper   [20 min]
  ├─ 9.2 注入 push_broker_log（6 处时机）                    [30 min]
  ├─ 9.3 sync_control_handlers::get_mqtt_broker_logs_api    [10 min]
  ├─ 9.4 smoke-collab-api.sh 增加 broker/logs 校验           [ 5 min]
  ├─ 9.5 cargo check --features web_server (增量 ~30s)      [ 1 min]
  └─ 9.6 git commit Phase 9                                  [ 5 min]
```

### Phase 12 ✅ 完成（本会话）

```
[Phase 12 完成 · commit 5463e41]
  ├─ 12.1 sse_handlers.rs 加 SyncEvent::MqttSubscriptionStatusChanged   [10 min]
  ├─ 12.2 sync_control_handlers.rs 加 push_subscription_status_event   [10 min]
  ├─ 12.3 4 处推送注入(set_master/set_client/start/stop)                 [15 min]
  ├─ 12.4 smoke-collab-api.sh 加 check_sse + [4/5] SSE 块                [10 min]
  ├─ 12.5 cargo check --features web_server (增量 ~38s)                  [ 1 min]
  └─ 12.6 git commit Phase 12                                            [ 5 min]
```

### Phase 11 ✅ 完成（本会话）

```
[Phase 11 完成]
  ├─ 11.0 写 docs/plans/2026-04-26-sprint-b-phase11-b6-reload.md (子计划)  [10 min]
  ├─ 11.1 site_config_handlers.rs 加 HOT_RELOADABLE_KEYS + diff_db_option   [10 min]
  ├─ 11.2 重写 reload_site_config (读 toml + diff + 分类返回)                [15 min]
  ├─ 11.3 cargo check --features web_server (增量 ~14s)                     [ 1 min]
  └─ 11.4 git commit Phase 11                                                [ 5 min]
```

### Phase 10 / 11-Plus / 12-Plus 后续会话推进

- Phase 10 = B5 graceful shutdown（涉及 main.rs + AppState 重构，工作量 2d）
- Phase 11-Plus = B6 真热加载（rs-core OnceCell → RwLock<Arc<DbOption>>，跨仓改动需独立会话）
- Phase 12-Plus = B4 跨仓前端联调（MqttNodesView 订阅 SSE 自动 reload `subscription/status`，0.5d）
