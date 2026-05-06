# rs-core SUL_DB Idle 弹性方案 · 开发文件

> 日期：2026-05-07  
> 状态：待评审  
> 触发：`docs/plans/...` 在 plant3d-web 仓内的 `2026-05-07-pms-simulator-6case-fail-triage-plan.md` §11.5 B1  
> 关联仓：`d:/work/plant-code/rs-core`（主要修改）+ `d:/work/plant-code/plant-model-gen`（消费者验证）  
> 范围：rs-core 全局 `SUL_DB` / `KV_DB` / `SECOND_SUL_DB` 单例的 idle 弹性

---

## 1. 一句话定位

rs-core 的 `pub static SUL_DB: Lazy<Surreal<Any>>` 是全局单连接，WS 长时间 idle 后被远端关闭，下次 `query.await` **永久 hang**，导致 `axum` handler 不返回、客户端 `socket hang up` / curl 52。需要在不破坏既有调用约定的前提下，加上**超时兜底 + 心跳保活 + 失败重连**三层弹性。

---

## 2. 背景

### 2.1 触发现场

`docs/plans/...../2026-05-07-pms-simulator-6case-fail-triage-plan.md` §11.2 已记录：

- 旧 backend 跑 1.5h 后 `/api/projects` `Empty reply from server`（curl 52）
- 同期 `/api/health` 200（不查 SUL_DB）→ 判定为 SUL_DB query 路径死锁
- 重启 backend 即恢复，6/6 PMS regression PASS

### 2.2 现状代码

- `rs-core/src/rs_surreal/mod.rs:92`：`pub static SUL_DB: Lazy<Surreal<Any>> = Lazy::new(Surreal::init);`
- `rs-core/src/rs_surreal/connection_manager.rs`：已有 `SurrealConnectionManager`，**但只在主机变更时重连**，对 idle 失败场景无任何处理（注释明说 "SurrealDB 的 `Lazy<Surreal<Any>>` 不支持显式 close"）
- `rs-core/src/rs_surreal/query_ext.rs:13`：已有 `SurrealQueryExt::query_response`，但只包了一层日志 location，**没有 timeout**
- 所有 handler 直接 `SUL_DB.query(...).await` 或 `query_response(...)`，无超时

### 2.3 关联事故

- `plant3d-web/docs/plans/2026-05-07-return-stop-scenario-fix-plan.md` §2.2 同样记录了 SurrealDB WS 单连接并发争抢现象（这次是 idle 而非并发，但底层是同一份单例）

---

## 3. 失败模式

### 3.1 复现条件

| 条件 | 必需性 |
|---|---|
| backend 持续运行 ≥ N 分钟（N 取决于 SurrealDB / 中间网关 idle timeout） | 必需 |
| 期间无主动 SUL_DB 查询（business idle） | 必需 |
| 之后任何走 SUL_DB 的 handler 被首次调用 | 触发 |

### 3.2 期望与实际

| 维度 | 期望 | 实际 |
|---|---|---|
| 后端调用 `query.await` | 数 ms 内成功或失败 | 永久 hang |
| handler 行为 | 返 200/4xx/5xx | 不返回 → axum 关 socket → curl 52 |
| 客户端可见 | 5xx + 错误信息 | `Empty reply from server` |

### 3.3 影响

- 所有依赖 SUL_DB 的接口（≥ 700 处调用）
- 拖累 PMS regression、巡检式 monitor、面板加载、自动化 E2E
- 错误信号 silent loss（log 也不打）

---

## 4. 方案对比

### 方案 A：调用层 timeout 包裹（最小侵入）

每个调用点：
```rust
let resp = tokio::time::timeout(Duration::from_secs(30),
    project_primary_db().query(sql)).await
    .map_err(|_| anyhow!("query timeout"))??;
```

- ✅ 局部、不动单例
- ❌ 700+ 处改动；新写代码也容易漏
- ❌ 不解决 idle，只解决"hang 看得到 5xx"
- ❌ 不重连，连续 N 次 timeout 后服务仍部分死

**结论**：仅作为兜底中转。

### 方案 B：`SurrealQueryExt` 内置 timeout + 失败标记（中等改动 · 推荐）

在 `query_ext.rs::query_response_with_location` 内：
```rust
let fut = db.query(sql);
let result = tokio::time::timeout(QUERY_TIMEOUT, fut).await
    .map_err(|_| { CONNECTION_MANAGER.mark_disconnected_blocking(); ... })??;
```

加：
1. `QUERY_TIMEOUT`：环境变量可调，默认 30s（兼容慢查询，比可疑 hang 短）
2. timeout 命中 → 把 manager 状态推到 `Disconnected`
3. 下次 `query_response` 入口检查 manager 状态，`Disconnected` 时先 `connect_or_reconnect` 再继续

迁移：把 700+ 处 `db.query()` 渐进迁到 `db.query_response()`。
保留 `db.query()` 不动（避免一次性大改），但在 lint 加 `#[deprecated]` 提醒。

- ✅ 集中点（query_ext.rs）
- ✅ 复用现有 connection_manager 状态机
- ⚠️ 需要解决 "Lazy<Surreal<Any>> 不支持 close" 的限制 → 用 `db.signin()` + `use_ns_db_compat()` 重新建立会话即可（不真重建 TCP；服务器侧 SurrealDB 会重新认证）
- ⚠️ 全量 700+ 调用迁移分多个 PR
- ❌ idle 中真正断了，重连能不能成功取决于 SurrealDB SDK 内部 WS 状态

**结论**：作为本计划的主线。

### 方案 C：心跳保活 + RwLock<Arc<Surreal<Any>>> 真热加载（最大改动）

后台 task 每 30s 跑 `RETURN 1`，确保 WS 不 idle；同时把 `SUL_DB` 改成 `RwLock<Arc<Surreal<Any>>>`（参考 collab-monitor AGENTS.md Phase 20 已有先例改 `OnceCell<DbOption> → RwLock<Arc<DbOption>>`）。

- ✅ 一劳永逸
- ❌ 改动面大（所有 `&SUL_DB` → 异步 `read().await.clone()` 或类似）
- ❌ Lazy 不支持 close，但 RwLock<Arc<>> 可以 swap 整个 Surreal 实例
- ❌ 跨仓影响 700+ 调用

**结论**：作为方案 B 之后的"长期阶段 3"，不在本期推进。

### 推荐顺序

**B（主线，本期落地）→ A（在迁移完成前补 1-2 处特别热的 handler 兜底）→ C（下季度评估）**

---

## 5. 推荐方案 B 详细设计

### 5.1 模块 / 文件改动清单

| 文件 | 改动 |
|---|---|
| `rs-core/src/rs_surreal/query_ext.rs` | `query_response_with_location` 加 timeout + 状态推送 |
| `rs-core/src/rs_surreal/connection_manager.rs` | 新增 `health_check()` + `try_revive(&Surreal<Any>, last_config)` |
| `rs-core/src/rs_surreal/mod.rs` | 暴露 `last_known_config()`（或类似）；新增 `pub const QUERY_TIMEOUT_DEFAULT: Duration = ...` |
| `plant-model-gen/src/web_server/handlers.rs` | 高频接口（`api_get_projects` 等）从 `db.query()` 迁到 `db.query_response()`（首批 ~10 处）|

### 5.2 接口契约

```rust
// query_ext.rs（伪代码）
pub trait SurrealQueryExt {
    async fn query_response(&self, sql: impl AsRef<str>) -> Result<Response>;
    async fn query_response_with_timeout(&self, sql: impl AsRef<str>, timeout: Duration) -> Result<Response>;
    // ...
}

// 默认实现 query_response 调用 query_response_with_timeout(sql, QUERY_TIMEOUT_DEFAULT)
// 内部：
//   1. CONNECTION_MANAGER.is_disconnected() => try_revive(self) 不行就直接返回错误
//   2. tokio::time::timeout(timeout, db.query(sql))
//   3. timeout / Err(连接级错误) => CONNECTION_MANAGER.mark_disconnected()
//   4. 错误向上抛
```

### 5.3 数据流

```
caller
  -> SurrealQueryExt::query_response(sql)
      -> CONNECTION_MANAGER.snapshot_state()
         | Connected
         |    -> tokio::time::timeout(30s, raw_query)
         |       | Ok(resp)            => return Ok(resp)
         |       | Err(query_err)      => mark_dead_if_io_error; return Err
         |       | TimeoutErr          => mark_disconnected; return Err("query timeout")
         | Disconnected
              -> try_revive(last_config)
                 | Ok                   => 重试一次 raw_query
                 | Err                  => return Err("revive failed")
```

### 5.4 错误模型

新增/扩展 error kind：

- `QueryTimeout(label: String)` — 超过 `QUERY_TIMEOUT`
- `ReviveFailed(reason)` — 重连失败
- `ConnectionDead` — `mark_disconnected` 后调用方收到的明确语义

调用方应当：
- timeout/dead → axum 返 503（建议加 `Retry-After`）
- revive failed → 503

### 5.5 心跳（可选，作为方案 B 增量）

后台 task：

```rust
// rs-core/src/rs_surreal/heartbeat.rs（新文件）
pub fn spawn_heartbeat(db: &'static Surreal<Any>, interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let _ = tokio::time::timeout(Duration::from_secs(5),
                db.query("RETURN 1")).await;
        }
    })
}
```

由 `plant-model-gen` 在 `web_server::start` 启动时 `spawn_heartbeat(&SUL_DB, Duration::from_secs(45))`。

---

## 6. 任务拆解

### Phase 1：query_ext.rs 包 timeout（最小可上线，~ 4h）

- [ ] **P1.1** `query_ext.rs`：拆 `query_response_with_location` → 加 `query_response_with_timeout`
- [ ] **P1.2** 新增 `QUERY_TIMEOUT_DEFAULT = 30s`，环境变量 `SUL_DB_QUERY_TIMEOUT_MS` 覆盖
- [ ] **P1.3** timeout 命中调 `CONNECTION_MANAGER.mark_disconnected()`
- [ ] **P1.4** 入口先 snapshot 状态，Disconnected 时 try_revive
- [ ] **P1.5** 错误类型扩展：`QueryTimeout` / `ReviveFailed` / `ConnectionDead`
- [ ] **P1.6** `cargo check -p rs-core`（debug，最小范围）

退出条件：rs-core 编译通过 + 单元测试（mock Surreal）覆盖 timeout/revive 路径。

### Phase 2：handlers.rs 首批迁移（~ 2h）

- [ ] **P2.1** `api_get_projects`（线上事故路径，最高优先级）
- [ ] **P2.2** `api_create_project` / `api_update_project` / `api_delete_project`
- [ ] **P2.3** `projects_health_scheduler` 内部循环
- [ ] **P2.4** `query` → `query_response` 替换；调用点确认错误向上抛 503
- [ ] **P2.5** 用 backend debug 跑起来 + curl 验证

退出条件：handlers 编译过；`/api/projects` 在重启 / idle 30 分钟后调用都返 200 或 503，永不 hang。

### Phase 3：心跳保活（~ 2h）

- [ ] **P3.1** `rs-core/src/rs_surreal/heartbeat.rs` 新建
- [ ] **P3.2** `plant-model-gen/src/web_server/mod.rs` 启动时 spawn
- [ ] **P3.3** 配置项：`web_server.heartbeat_interval_ms`（DbOption.toml）

退出条件：backend 运行 2 小时 + 验证 `/api/projects` 仍正常。

### Phase 4：剩余迁移（~ 4h，可拆多 PR）

- [ ] **P4.1** review_api、scene-tree、room、PDMS 等 modules 的 `query` → `query_response`
- [ ] **P4.2** 并发热点（如 `review/workflow/sync`）单独评审，避免 timeout 误伤大查询

### Phase 5：验证（~ 2h）

- [ ] **P5.1** `npm run test:pms:simulator` 6/6 PASS
- [ ] **P5.2** 手动模拟 idle：backend 启动后等 1 小时 → curl `/api/projects` 应在 30s 内返 200 或 503
- [ ] **P5.3** 长跑回归：backend 跑 4 小时 + 每 30s 一次 `/api/projects` smoke

---

## 7. 验证策略

按 plant-model-gen/AGENTS.md 与 rs-core/AGENTS.md：

- ✅ **CLI / curl / 真实 web_server**（首选）
- ❌ 不写一次性 `cargo test`（仅在最小路径无法覆盖时补 mock 单元）

具体：

| 测试 | 工具 | 通过条件 |
|---|---|---|
| timeout 触发 | 注入 mock Surreal `query` 永远 pending | 30s 内返 `QueryTimeout` |
| Idle 重连 | backend 启动后 sleep N 分钟 + curl `/api/projects` | 200 或 503 + 后续 30s 内 200 |
| 心跳生效 | tracing 日志可见每 45s `RETURN 1` | tail backend.log |
| PMS regression | `npm run test:pms:simulator` | 6/6 PASS |

---

## 8. 风险登记

| ID | 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|---|
| R1 | timeout 30s 误伤大查询（如复杂 spatial / scene-tree 计算） | 中 | 计算路径偶发 503 | 计算密集接口单独提供 `query_response_with_timeout(sql, 90s)`；在 Phase 4 标记 |
| R2 | revive 失败后状态机卡死 | 中 | 全局 503 | revive 失败重试 N 次后仍卡 → graceful exit web_server，由 systemd / supervisor 重拉 |
| R3 | `Lazy<Surreal<Any>>` 真断了 signin 仍救不回来 | 中 | 同 R2 | 加内核迹证据（log SDK 状态） + 备选方案 C 触发条件 |
| R4 | 心跳频率太高占用 SurrealDB CPU | 低 | 性能下降 | 默认 45s，环境变量可调 |
| R5 | 700+ 调用迁移破坏既有逻辑 | 中 | 局部回归 | 分批小 PR，每批走 PMS regression + 关键 smoke |

---

## 9. 不做的事（YAGNI）

- 不替换 `Lazy<Surreal<Any>>` 为 `RwLock<Arc<Surreal<Any>>>`（方案 C，下季度）
- 不改 SurrealDB SDK 版本（无关）
- 不引入连接池（deadpool 等）
- 不修改 `auto_start_surreal` 自启逻辑（与本问题无关，但日志显示二次启动 PID 93400 端口冲突自然退，可单独清理）
- 不动 SurrealKV / KV_DB 路径（除非 Phase 4 时确认必要）

---

## 10. 决策检查点

| 节点 | 何时 | 决策项 |
|---|---|---|
| Phase 1 完成 | rs-core build OK | 是否合 PR；timeout 默认 30s 是否合理 |
| Phase 2 完成 | handlers 首批迁移 | 是否合 PR；扩展剩余 modules |
| Phase 3 启动 | 心跳上线 | 是否在所有项目（含 collab-monitor）默认开 |
| Phase 5 通过 | 长跑回归通过 | 关闭本计划；启动 backlog C |

---

## 11. 关联文件

```
d:/work/plant-code/rs-core/
├── src/rs_surreal/
│   ├── mod.rs                               # SUL_DB / KV_DB / project_primary_db()
│   ├── connection_manager.rs                # SurrealConnectionManager
│   ├── query_ext.rs                         # SurrealQueryExt（timeout 接入点）
│   └── (heartbeat.rs)                       # 新建

d:/work/plant-code/plant-model-gen/
├── src/web_server/
│   ├── handlers.rs                          # api_get_projects 等
│   └── mod.rs                               # spawn_heartbeat 调用点
├── docs/plans/
│   ├── 2026-05-07-rs-core-sul-db-idle-resilience-plan.md   # 本文件
│   └── 2026-05-07-pms-simulator-6case-fail-triage-plan.md（事故复盘原始）
└── db_options/DbOption.toml                 # 心跳配置项
```

---

## 12. 上线后观察项

- backend.log 中 `QueryTimeout` 频次（应 ≤ 1 次/天）
- `mark_disconnected` 调用次数（应在 idle 阈值附近触发）
- `/api/projects` 95p / 99p 响应时间（应在 100ms 内不变）
- 长跑稳定性（backend 连续运行 7 天 不需要 restart）

