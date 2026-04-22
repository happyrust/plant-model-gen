# Phase 1 执行清单 · API 汇入 plant-model-gen

> 父计划：`docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md`
>
> 分支：`feat/collab-api-consolidation`（已切）
>
> 估时：5h · 产出 M1 里程碑（API 就绪）

## 关键前置事实（动手前必读）

| 维度 | web-server（源） | plant-model-gen（目标） |
|---|---|---|
| `DbOption.toml` 路径 | 硬编码 `"DbOption.toml"`（根目录） | `std::env::var("DB_OPTION_FILE").unwrap_or("db_options/DbOption")` |
| 多站点支持 | ❌ 单站点 | ✅ 多站点（`site_runtime_dir(site_id)/DbOption.toml`） |
| `AppState.shutdown_tx` | 有 | 需确认（见 1.1.2） |
| `rusqlite` / `config` / `toml` 依赖 | 有 | 需确认 |
| `aios_core::get_db_option()` | 有 | 有（`handlers.rs:1` 已用） |
| MQTT client 实现 | `src/mqtt_service/` | 需对比（见 1.2） |

**核心迁移原则**：
1. **先单站点，再多站点**：Phase 1 先让所有 API 在"单站点"模式（`db_options/DbOption.toml`）下跑通；多站点是 Phase 5 增量。
2. **路径统一 env 化**：所有硬编码 `"DbOption.toml"` 改为读 `DB_OPTION_FILE` env + `db_options/DbOption` fallback。
3. **handler 签名保持**：`async fn(State<AppState>, ...) -> Result<Json<Value>, StatusCode>` 保持不变，保证前端零修改。
4. **逐步编译**：每小步都跑 `cargo check --features web_server`，不把编译错误堆到最后。

---

## 1.1 迁入 `site_config_handlers.rs`（NEW · ~45min）

### 1.1.1 依赖确认（10min）

- [ ] 跑 `cargo tree --features web_server | rg -E 'rusqlite|config|toml|chrono' | head`
- [ ] 确认 4 个依赖都在 Cargo.toml：`rusqlite`、`config`、`toml`、`chrono`
- [ ] 若缺，用 `cargo add` 补齐

### 1.1.2 AppState 字段对比（5min）

- [ ] `rg "pub struct AppState" src/web_server/` 定位结构体定义
- [ ] 对比 web-server 的 AppState，列出 plant 缺失字段：
  - 必定用到：`shutdown_tx` → 用于 `POST /api/site-config/restart`
- [ ] 如缺失，追加字段并在 AppState 构造处初始化

### 1.1.3 文件复制 + 路径改写（20min）

- [ ] `cp web-server/src/web_server/site_config_handlers.rs plant-model-gen/src/web_server/`
- [ ] 批量替换（使用 StrReplace）：
  - `"DbOption.toml"` → `get_db_option_path()`
  - 新增辅助函数：
    ```rust
    fn get_db_option_path() -> String {
        std::env::var("DB_OPTION_FILE")
            .unwrap_or_else(|_| "db_options/DbOption".to_string()) + ".toml"
    }
    fn get_db_option_name() -> String {
        std::env::var("DB_OPTION_FILE")
            .unwrap_or_else(|_| "db_options/DbOption".to_string())
    }
    ```
  - `cfg::File::with_name("DbOption")` → `cfg::File::with_name(&get_db_option_name())`
- [ ] `cargo check --features web_server`，直到零错误

### 1.1.4 模块注册（3min）

- [ ] `src/web_server/mod.rs` 顶部添加 `pub mod site_config_handlers;`

### 1.1.5 路由注册（5min）

- [ ] 在 `mod.rs` 的 router builder 里追加 7 条：
  ```rust
  .route("/api/site-config", get(site_config_handlers::get_site_config))
  .route("/api/site/info", get(site_config_handlers::get_site_info))
  .route("/api/site-config/save", post(site_config_handlers::save_site_config))
  .route("/api/site-config/validate", post(site_config_handlers::validate_site_config))
  .route("/api/site-config/reload", post(site_config_handlers::reload_site_config))
  .route("/api/site-config/restart", post(site_config_handlers::restart_server))
  .route("/api/site-config/server-ip", get(site_config_handlers::get_server_ip))
  ```

### 1.1.6 编译验证（2min）

- [ ] `cargo check --features web_server`
- [ ] 确认零 error，warning 可延后处理

### 1.1.7 Commit

- [ ] `git commit -m "feat(collab): 迁入 site_config_handlers (Phase 1.1 · NEW)"`

---

## 1.2 迁入 `mqtt_monitor_handlers.rs`（NEW · ~1h）

### 1.2.1 依赖评估（15min）

- [ ] 读取 web-server 版 `mqtt_monitor_handlers.rs` 顶部 use 语句
- [ ] 对比 plant 的 `mqtt_service/` 模块暴露的 API
- [ ] 列出需要调整的 import：
  - 若 web 依赖的类型 `plant` 里也有，直接调整 path
  - 若 plant 缺失，评估是否带 `src/mqtt_service/` 也迁一部分

### 1.2.2 文件复制 + 适配（30min）

- [ ] `cp web-server/src/web_server/mqtt_monitor_handlers.rs plant-model-gen/src/web_server/`
- [ ] 调整 import 路径，使其匹配 plant 侧 `mqtt_service/` 结构
- [ ] 如 plant 侧缺失关键 mqtt client API，先做**最小 stub**（返回空数组/占位），Phase 5 再补真实逻辑

### 1.2.3 模块 + 路由注册（10min）

- [ ] `mod.rs`：`pub mod mqtt_monitor_handlers;`
- [ ] 注册 13 条 `/api/mqtt/*` 路由

### 1.2.4 编译 + Commit（5min）

- [ ] `cargo check --features web_server`
- [ ] `git commit -m "feat(collab): 迁入 mqtt_monitor_handlers (Phase 1.2 · NEW)"`

---

## 1.3 合并 `sync_control_handlers.rs`（MERGE · ~1.5h）

### 1.3.1 Diff 审查（20min）

- [ ] `diff -u plant/src/web_server/sync_control_handlers.rs web/src/web_server/sync_control_handlers.rs > /tmp/sync_handlers.diff`
- [ ] 提取 web 侧独有的 `pub async fn` 清单（估计 ~43KB 增量）
- [ ] 按功能分组：
  - 任务管理：`add_sync_task`、`trigger_download`、`cancel_sync_task`、`clear_sync_queue`
  - 运行时：start/stop/pause/resume/restart
  - MQTT 控制：mqtt/start、mqtt/stop、mqtt/status
- [ ] 列出每个 handler 的依赖（types、store 字段、broadcast channel）

### 1.3.2 逐项合并（1h）

按分组来，每合并一组跑一次 `cargo check`：

**组 A · 任务管理（~20min）**
- [ ] 把 web 侧的 `add_sync_task`/`trigger_download` 等 handler 复制过来
- [ ] 调整对 `sync_control_center` 的调用（若 plant 侧方法签名不同需适配）
- [ ] `cargo check`

**组 B · 运行时控制（~20min）**
- [ ] start/stop/pause/resume handler（可能 plant 已有，需 diff）
- [ ] `cargo check`

**组 C · MQTT 控制（~20min）**
- [ ] mqtt/start、mqtt/stop、mqtt/status
- [ ] `cargo check`

### 1.3.3 路由注册 + Commit（10min）

- [ ] 在 `mod.rs` 补齐 web-server 特有的 `/api/sync/*` 和 `/api/sync/mqtt/*` 路由
- [ ] `cargo check --features web_server`
- [ ] `git commit -m "feat(collab): 合并 sync_control_handlers (Phase 1.3 · MERGE)"`

---

## 1.4 反向合并 `remote_sync_handlers.rs`（REVERSE MERGE · ~1h）

**策略**：plant 101KB 为主，仅补 web 独有 handler（plant 已经更全）。

### 1.4.1 Diff（15min）

- [ ] `diff -u plant/.../remote_sync_handlers.rs web/.../remote_sync_handlers.rs`
- [ ] 提取 web 独有的 handler 和 struct
- [ ] 评估哪些真的需要（有些可能是旧代码，plant 已用更好的方案替代）

### 1.4.2 选择性合并（30min）

- [ ] 对于 plant 缺失且 API 文档里有路由的 handler，合并进来
- [ ] 对于重复实现，**保留 plant 版**（更新）
- [ ] `cargo check`

### 1.4.3 路由完整性校验（10min）

- [ ] 对照父计划里列出的 20+ 条 `/api/remote-sync/*` 路由，确保每条都已注册
- [ ] 缺失的补齐

### 1.4.4 Commit（5min）

- [ ] `git commit -m "feat(collab): 反向合并 remote_sync_handlers (Phase 1.4)"`

---

## 1.5 对齐核心（`sync_control_center.rs` + `increment_manager.rs`）· ~45min

### 1.5.1 sync_control_center diff（20min）

- [ ] diff 两边
- [ ] 列出 web 独有的 public 方法（影响 handler）
- [ ] 若 handler 依赖这些方法，到这里补齐；否则跳过

### 1.5.2 increment_manager 核心函数确认（15min）

plant 侧必须存在这些函数（否则后续 handler 调用会爆）：
- [ ] `AiosDBManager::init_watcher`
- [ ] `AiosDBManager::async_watch`
- [ ] `AiosDBManager::execute_incr_update`
- [ ] `AiosDBManager::poll_sync_e3d_mqtt_events_with_backoff`
- [ ] 若缺失，**从 web-server 版复制对应函数过来**

### 1.5.3 Commit（5min）

- [ ] `git commit -m "feat(collab): 对齐 sync_control_center + increment_manager (Phase 1.5)"`

---

## 1.6 deployment_sites.sqlite schema 对齐 · ~30min

### 1.6.1 Schema 导出对比（10min）

```bash
sqlite3 plant-model-gen/deployment_sites.sqlite ".schema" > /tmp/plant_schema.sql
sqlite3 web-server/deployment_sites.sqlite ".schema" > /tmp/web_schema.sql
diff /tmp/plant_schema.sql /tmp/web_schema.sql
```

- [ ] 列出缺失/差异的表和字段

### 1.6.2 Migration SQL（15min）

- [ ] 写 `src/web_server/db_migrations/2026_04_22_collab_schema_align.sql`
- [ ] 在 web_server 启动时跑 migration（幂等：用 `CREATE TABLE IF NOT EXISTS`）
- [ ] 目标表：`remote_sync_envs`、`remote_sync_sites`、`remote_sync_logs`、`site_config`（1.1 已建）

### 1.6.3 Commit（5min）

- [ ] `git commit -m "feat(collab): deployment_sites.sqlite schema 对齐 (Phase 1.6)"`

---

## 1.7 路由注册总闸 · ~15min

- [ ] `rg "\.route\(\"/api/" src/web_server/mod.rs | wc -l` 看总数
- [ ] 对照父计划 §2 的 40+ endpoint 清单，一项项勾
- [ ] 漏的补齐

---

## 1.8 冒烟测试 · M1 里程碑 · ~15min

```bash
cargo run --bin web_server --features web_server
```

### 1.8.1 端点自检

```bash
curl -s http://127.0.0.1:9099/api/site-config | jq .status
curl -s http://127.0.0.1:9099/api/remote-sync/envs | jq .
curl -s http://127.0.0.1:9099/api/mqtt/nodes | jq .
curl -s http://127.0.0.1:9099/api/sync/status | jq .
curl -s http://127.0.0.1:9099/api/sync/events/stream   # SSE (Ctrl+C after 2s)
```

全部返回成功 JSON 或合法 SSE 流 = **M1 达成** ✅

### 1.8.2 文档交付

- [ ] 新建 `docs/architecture/异地协同API汇总清单.md`
- [ ] 列全 40+ endpoint 表（method/path/handler/request/response）

### 1.8.3 Final Commit

- [ ] `git commit -m "docs(collab): API 汇总清单 + M1 里程碑冒烟通过"`

---

## 风险应急

| 若发生 | 应对 |
|---|---|
| `cargo check` 爆多个编译错 | 不要全部一次性修，回退到上一个绿色状态，分子步骤再试 |
| AppState 字段差异大无法 stub | 只迁 1.1（site_config），其他 handler 放 Phase 1.5 后再做 |
| MQTT 依赖无法对齐 | 1.2 降级为 stub 版（返回空），M1 不阻塞 |
| SQLite schema 对齐失败 | 保留 plant 版 schema 不动，web-server 的老数据 dump + 人工入库 |
| 单站点假设不成立 | 1.1 的路径改为 `managed_project_sites::active_site_db_option_path()` 动态解析 |

## 快速回滚

每完成一个子 Phase 都是一个 commit。如果某步翻车：

```bash
git reset --hard HEAD~1
```

回到上一步，重新审视策略。

## 完成条件

- [ ] 1.1–1.6 所有 sub-phase 的 Commit 都落地
- [ ] 1.8 的 4 个 curl 全部 OK
- [ ] `docs/architecture/异地协同API汇总清单.md` 就绪

→ 进入 Phase 2（plant-collab-monitor 脚手架 + 移植）
