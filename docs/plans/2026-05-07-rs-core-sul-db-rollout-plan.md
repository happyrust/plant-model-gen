# rs-core SUL_DB 弹性 Rollout 执行方案

> 日期：2026-05-07  
> 状态：待评审  
> 关联：
> - `docs/plans/2026-05-07-rs-core-sul-db-idle-resilience-plan.md`（B1 总体方案，本文件承接其 §6）  
> - `<plant3d-web>/docs/plans/2026-05-07-pms-simulator-6case-fail-triage-plan.md`（事故复盘）  
> 本文件聚焦：Phase 1 完成后到生产稳定的具体执行编排，含子任务、提交策略、验证里程碑、风险与回滚。

---

## 0. 一句话定位

把 B1 plan 中已落地的 Phase 1 基础设施（query_ext timeout + connection_manager mark/revive）端到端打通到 backend 全量调用路径与稳定性观测，让 SurrealDB idle hang 这一类故障**长期不再复现**。

---

## 1. 当前位置

### 1.1 已落地（Phase 1）

| 改动 | 文件 | 状态 |
|---|---|---|
| query_response 加 30s timeout（env 可调） | `rs-core/src/rs_surreal/query_ext.rs` | ✅ cargo check OK |
| `ConnectionState::Disconnected { last_config }` | `rs-core/src/rs_surreal/connection_manager.rs` | ✅ |
| `mark_disconnected` 保留 last_config | 同上 | ✅ |
| `is_disconnected()` / `try_revive(&Surreal<Any>)` | 同上 | ✅ |
| 事故路径 `api_get_projects` 迁到 `query_response` | `plant-model-gen/src/web_server/handlers.rs` | ✅ cargo check OK |

### 1.2 待落地

| 阶段 | 范围 |
|---|---|
| Phase 2 | handlers.rs 剩余 query 调用迁移 + Any-typed 弹性入口 |
| Phase 3 | rs_surreal/heartbeat.rs spawn |
| Phase 4 | review_api / scene_tree / room / pdms 等 modules 迁移 |
| Phase 5 | 长跑验证（idle 1h + 6 case PMS regression 复跑） |

---

## 2. 三个执行候选

### 候选 X：纯前推（Phase 2 → 3 → 4 → 5 顺序，串行）

- 适合：稳，不抢功
- 缺点：周期长（≥ 2 个工作日），用户上线慢，Phase 3 心跳一上线即可缓解 idle，但要等 Phase 2 完才上

### 候选 Y：先纵向打穿（Phase 3 心跳先于 Phase 2/4，~~"灯泡先点上"~~），再横向覆盖

- 适合：让生产**最快**拿到 idle 缓解（心跳直接消除 idle 触发条件）
- 缺点：handlers 大部分还没迁，万一心跳偶尔失败仍可能命中老 hang 路径（但 query_ext timeout 已兜底）

### 候选 Z（推荐）：分两次冲刺，每次冲刺都可上线

冲刺 1（生产可上线）：
- Phase 3（heartbeat）+ 给 `Surreal<Any>` 加专用入口 `query_response_resilient`（封装 mark/revive）
- 提交后即可重启 backend 验证

冲刺 2（迁移收尾）：
- Phase 2 + Phase 4：把 700+ 处 query 渐进迁到 `query_response` 或 `query_response_resilient`
- 与冲刺 1 解耦，可分多个 PR 推

**理由**：心跳是"系统级缓解"，不依赖每个调用点改写；专用入口让事故路径（api_get_projects 等）继续受益；冲刺 2 的迁移是"防御加固"，每个 PR 只改一组接口，CI/手动验证容易。

---

## 3. 推荐方案 Z 的详细任务卡

### 冲刺 1 · 生产可上线（约 5 h）

#### S1.1 — Any-typed 弹性入口（1.5 h）

新增 `rs-core/src/rs_surreal/any_resilient.rs`（或在 `query_ext.rs` 末尾）：

```rust
use crate::rs_surreal::{CONNECTION_MANAGER, SUL_DB};

pub async fn query_response_resilient(sql: impl AsRef<str>) -> anyhow::Result<Response> {
    if CONNECTION_MANAGER.is_disconnected().await {
        if let Err(e) = CONNECTION_MANAGER.try_revive(&SUL_DB).await {
            log::warn!("[sul-db] try_revive 失败: {e}");
        }
    }
    let result = SUL_DB.query_response(sql).await;
    if let Err(ref err) = result {
        if is_connection_dead(&err.to_string()) {
            CONNECTION_MANAGER.mark_disconnected().await;
        }
    }
    result
}

fn is_connection_dead(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("query timeout")
        || lower.contains("send_error")
        || lower.contains("io error")
        || lower.contains("websocket")
        || lower.contains("channel closed")
        || lower.contains("connection")
}
```

- [ ] **S1.1.1** 新增 helper + 单元测试（mock CONNECTION_MANAGER）
- [ ] **S1.1.2** 把 `api_get_projects` 从 `query_response` 迁到 `query_response_resilient`（5 处 query，2 处已迁，扩展到剩余 3 处 `count_sql` 等）
- [ ] **S1.1.3** `cargo check -p aios_core` + `cargo check --bin web_server --features web_server`

#### S1.2 — 心跳保活（1.5 h）

新增 `rs-core/src/rs_surreal/heartbeat.rs`：

```rust
pub fn spawn_heartbeat(db: &'static surrealdb::Surreal<surrealdb::engine::any::Any>,
    interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match tokio::time::timeout(Duration::from_secs(5), db.query("RETURN 1")).await {
                Ok(Ok(_)) => log::trace!("[sul-db] heartbeat ok"),
                Ok(Err(e)) => {
                    log::warn!("[sul-db] heartbeat err: {e}");
                    crate::rs_surreal::CONNECTION_MANAGER.mark_disconnected().await;
                }
                Err(_) => {
                    log::warn!("[sul-db] heartbeat timeout");
                    crate::rs_surreal::CONNECTION_MANAGER.mark_disconnected().await;
                }
            }
        }
    })
}
```

- [ ] **S1.2.1** 新增模块 + 测试（手动验证 RETURN 1）
- [ ] **S1.2.2** plant-model-gen `web_server::start` 启动后 spawn `&SUL_DB`、interval=45s（DbOption.toml 可配）
- [ ] **S1.2.3** DbOption.toml 加 `[web_server] heartbeat_interval_ms = 45000`，optional

#### S1.3 — backend 验证（1 h）

- [ ] **S1.3.1** kill 当前 backend (PID 56592) → cargo build --bin web_server --features web_server → 启动新 backend
- [ ] **S1.3.2** 执行 `npm run test:pms:simulator` 6/6 PASS（基线）
- [ ] **S1.3.3** sleep 1h（或 `Start-Sleep -Seconds 3600`），观察 `backend.log` 心跳条目；之后 curl `/api/projects` 应在 30s 内返 200/503，永不 hang
- [ ] **S1.3.4** 如 1h 后跑 6 case 全 PASS → 冲刺 1 接收

#### S1.4 — 提交（0.5 h）

按 atomic commit 拆三个：

- (a) `plant3d-web`：simulator UX 增强 + 2 plan 文件（commit msg `feat(simulator): 全屏/折叠/skipIframeSrc + 事故复盘 plan`）
- (b) `rs-core`：query_ext timeout + connection_manager mark/revive + heartbeat 模块 + any_resilient（commit msg `feat(rs-core): SUL_DB idle 弹性 Phase 1+冲刺1`）
- (c) `plant-model-gen`：handlers api_get_projects 迁移 + plan + heartbeat spawn（commit msg `feat(web_server): SUL_DB query 迁 query_response_resilient + 心跳`）

### 冲刺 2 · 防御加固（约 8–10 h，可拆 N 个 PR）

#### S2.1 — handlers.rs 剩余 SUL_DB query 迁移（4 h）

目录：

| 类别 | 计数 | 工时 |
|---|---|---|
| `api_create_project` / `api_update_project` / `api_delete_project` | 6 | 1h |
| `projects_health_scheduler`（背景任务） | 3 | 0.5h |
| `api_database_status` / `api_demo` 等管理面 | 8 | 1.5h |
| `check_model_status` / `model_status` 系列 | 12 | 1h |

每子组单独 PR，PR 描述附 cargo check + curl smoke 证据。

#### S2.2 — review_api / scene_tree / pdms / room 模块迁移（3 h）

review_api 涉及 `review_primary_db` / `review_db`，与 SUL_DB 不同——确认是否复用 CONNECTION_MANAGER（可能不复用，应单独管理）；
scene_tree / pdms / room 直接用 SUL_DB.query_response，按 SurrealQueryExt 路径生效 timeout，无需特殊。

#### S2.3 — 灰度时长扩到 24h（1 h）

backend 跑 24h 自动监控：每 5 min 一次 `/api/projects` smoke。任何一次失败邮件/IM 提醒。失败定义：响应时间 > 35s 或 5xx。

---

## 4. 提交策略

### 4.1 commit 原则

- 每个 commit 必须独立可编译可重启（cargo check + 启动 web_server 健康）
- commit 范围不跨仓
- 新增依赖不允许进入这两组冲刺（除非用户明确批）
- 失败的 commit 用 `git revert` 回滚，不强推

### 4.2 PR 模板（建议）

```
## Why
事故链 / 现象 / 根因 / 修复策略一句话

## What
- query_ext: 加 timeout
- connection_manager: 加 mark/revive
- handlers: 迁 N 处 query

## Verify
1. cargo check -p aios_core
2. cargo check --bin web_server --features web_server
3. npm run test:pms:simulator → 6/6 PASS
4. 长跑 1h /api/projects smoke → 200/503，不 hang

## Refs
- docs/plans/2026-05-07-rs-core-sul-db-idle-resilience-plan.md
- docs/plans/2026-05-07-rs-core-sul-db-rollout-plan.md
```

### 4.3 PR 序列（推荐）

| # | PR 名 | 仓 | 依赖 |
|---|---|---|---|
| 1 | feat(simulator): 全屏/折叠/skipIframeSrc + 事故复盘 plan | plant3d-web | 无 |
| 2 | feat(rs-core): SUL_DB idle 弹性 Phase 1 | rs-core | 无 |
| 3 | feat(rs-core): SUL_DB heartbeat + any_resilient 入口 | rs-core | PR2 |
| 4 | feat(web_server): api_get_projects 迁 query_response_resilient + heartbeat 启动 | plant-model-gen | PR2, PR3 |
| 5+ | 后续 handlers / modules 迁移（每批 ≤ 5 文件） | plant-model-gen | PR4 |

---

## 5. 验证里程碑

| Milestone | 工具 | 通过条件 |
|---|---|---|
| M1 编译 | `cargo check` | rs-core / plant-model-gen 两仓 OK |
| M2 启动 | `web_server.exe --config DbOption.toml` | 5s 内 `/api/health=200` |
| M3 短链路 | `npm run test:pms:simulator` | 6/6 PASS（含 contract smoke 7/7） |
| M4 idle | sleep 1h + curl `/api/projects` | 200/503，**永不 hang** |
| M5 心跳 | tail backend.log | 每 45s 一次 `[sul-db] heartbeat ok` |
| M6 长跑 | sleep 24h + 5min/次 smoke | 失败率 ≤ 0.1% |

---

## 6. 风险与回退

| ID | 风险 | 缓解 | 回退 |
|---|---|---|---|
| R1 | timeout 30s 误伤大查询 | 计算密集接口提供 90s 变体；env 全局拉到 60s 应急 | 单接口注释关闭 timeout（重新走 raw `db.query`） |
| R2 | revive 失败死循环 | revive Err 后只记 log 不再次重试；下次 query 入口尝试一次 | 移除 wiring，仅保 timeout（保底降级） |
| R3 | heartbeat 频率过高 SurrealDB CPU 负担 | 默认 45s，env 可调到 120s | 配置项 `heartbeat_interval_ms=0` 关闭 |
| R4 | review_db / model_kv 复用同一 CONNECTION_MANAGER 不当 | review_db 单独 manager（PR 中明确） | 撤销 review wiring，仅保 SUL_DB |
| R5 | handlers 迁移破坏返回格式 | 每 PR 跑 `npm run test:pms:simulator` | git revert |

---

## 7. YAGNI（本 sprint 不做）

- 不引入连接池（deadpool）
- 不替换 `Lazy<Surreal<Any>>` 为 `RwLock<Arc<Surreal<Any>>>`（B1 plan §4 方案 C，留下季度）
- 不动 SurrealKV / KV_DB（独立路径，与本事故无关）
- 不修 `auto_start_surreal` 重复启动 `surreal.exe` 的端口冲突日志（无功能影响）
- 不为本 sprint 写 cargo test（按 AGENTS.md，CLI/curl/真实 web_server 验证为准）

---

## 8. 决策点（人在环上）

| 节点 | 触发 | 决策 |
|---|---|---|
| Pre-Sprint1 | 评审本 plan | 是否启用候选 Z |
| Sprint1 完成 | M1–M5 通过 | 是否上线 / 启动 Sprint2 |
| Sprint2 PR ≥ 5 | 迁移过半 | 是否拉一次回归 master |
| Post-Sprint2 | M6 完成 | 是否标记 B1 整体 closed |

---

## 9. 关联进度状态

```
B1 Phase 1: ✅ 完成（query_ext timeout + connection_manager mark/revive）
本 plan Sprint1: ⏳ 待评审
本 plan Sprint2: ⏳ 等 Sprint1
回退路径：每个 PR 单独可 revert，不破坏 B1 Phase 1 基础设施
```

---

## 10. 现状 working tree（截至 2026-05-07 02:32）

```
plant3d-web/  6 个修改 + 1 个新 plan
rs-core/      2 个原有修改 (lib.rs / options.rs，与 B1 无关) + 2 B1 修改 (query_ext.rs / connection_manager.rs)
plant-model-gen/  1 个 B1 修改 (handlers.rs) + 1 个 B1 plan + 1 个 rollout plan
```

建议：进入 Sprint1 前先 commit Phase 1 检查点（按 §4.3 PR 序列 PR1+PR2 拆开），便于回退。

