# 站点管理 · 下一步开发计划（2026-04-26）

> 上游：
> - 站点安全收口：`docs/plans/2026-04-24-admin-site-security-hardening-plan.md`
> - 下一冲刺规划：`docs/plans/2026-04-24-next-sprint-development-plan.md`
> - Sprint B 主计划：`docs/plans/2026-04-26-sprint-b-plan.md`
> - Sprint B Phase 11 子计划：`docs/plans/2026-04-26-sprint-b-phase11-b6-reload.md`
> - Sprint B 验收报告：`docs/plans/2026-04-26-sprint-b-verification-report.md`
> - 跨仓 PRD：`../plant-collab-monitor/docs/prd/2026-04-26-remote-site-prd.md`

---

## 0. 背景与边界

仓库内"站点（site）"语义实际有 **三套并存**，但同住一个 SQLite 文件 `deployment_sites.sqlite`：

| 系统 | 路由前缀 | 核心代码 | 用途 |
|---|---|---|---|
| **Admin 站点** | `/api/admin/sites/*` | `admin_handlers.rs` + `managed_project_sites.rs`(3679 行) | 本机托管多个项目的子进程（SurrealDB + web_server）启停/解析/日志/资源监控 |
| **Deployment Sites（旧）** | `/api/sites`、`/api/deployment-sites/*` | `handlers.rs` + `site_registry.rs` | 跨站点注册表 + TTL 心跳，给异地协同用 |
| **Site Config** | `/api/site-config/*` | `site_config_handlers.rs` | 当前 web_server 自身的 DbOption.toml 配置（DB/MQTT/解析开关/同步） |

本计划聚焦 **Admin 站点 + Site Config** 两大主战场，**Deployment Sites（旧）只在 Sprint F 概念去重时回头收口**，避免阶段性混杂。

---

## 1. 已落地能力（基线确认）

### Admin 站点
- CRUD + 生命周期：list / get / create / update / delete / parse / start / stop / runtime / logs（10 个 endpoint，全部经 `admin_auth_middleware`）
- 解析预览：`/api/admin/sites/preview-parse-plan`，5 种 mode（Full / Bootstrap / RebuildSystem / Selective / FastReparse）
- 5 个常用预设（前端 `parse-db-types.ts`）：快速部署 / 带字典 / 带元件库 / 全量系统数据
- 状态机：7 个 site 状态 × 4 个 parse 状态，`canStart/Stop/Parse/Delete/Edit` 守卫一致
- 风险评估：`risk_level / warnings / parse_health`，CPU/内存/磁盘三阈值（70/85/95%）
- 进程隔离：独立 process group + Unix `killpg` / Windows `taskkill /T`
- 安全：拒绝 `root/root` 等 5 类弱凭据 + 拒绝 `bind_host=0.0.0.0`，env 逃生 `AIOS_ALLOW_WEAK_DB_CREDS=1` / `AIOS_ALLOW_PUBLIC_BIND=1`
- 路径白名单 + canonicalize + 子站点 DbOption.toml `0600` 权限
- 写流程互斥：进程内 Mutex + SQLite `BEGIN IMMEDIATE`
- 错误反馈：详情页 banner 带动作标签 + dismiss

### Site Config（4-26 Sprint B 完成 5/7 phase）
- B1/B3/B7：MQTT set_master/client 写盘 + status 5 字段补齐 + 冒烟脚本 20/20 PASS
- B2：broker logs ring-buffer（capacity 200，倒序，5 字段每条）
- B4：SSE `MqttSubscriptionStatusChanged` 实时推送
- B6：reload diff（hot/static 字段分类，但**不真热改**）

---

## 2. Gap 清单（按风险 × 紧急度）

### 🔴 P0 阻塞 UX

| ID | Gap | 现状 | 影响 |
|---|---|---|---|
| G1 | save 后无法自动重启 | TODO @ `site_config_handlers.rs:352-355`；`restart_server` 是 stub | collab-monitor 改完看不到生效，必须 SSH kill+restart |
| G2 | DbOption 不可真热改 | `aios_core::get_db_option()` 是 OnceCell；hot_changed_keys 仅分类提示 | 改 `enable_log` 都需要重启 |
| G3 | reload baseline 永远报 `static_changed_keys: ["surrealdb"]` | env 覆盖让运行时 ≠ 文件 | 用户每次 reload 看到误报 |

### 🟠 P1 已知 bug / 一致性

| ID | Gap | 现状 | 影响 |
|---|---|---|---|
| G4 | 三套站点系统数据冗余 | 同库 3 表，site_registry 与 managed_project_sites 都有 status 字段但语义不同 | 后续做"远程站点纳管"会撞概念 |
| G5 | SSE 偶丢首条事件 | BroadcastStream 在 listener subscribe 与 send 同步时漏 | 可观测性受影响 |
| G6 | `/api/remote-sync/*` admin auth 失效 | 测试预期 503，实测 200 | 异地协同接口在未登录时也能访问 |
| G7 | runtime/logs 双 polling | 详情页每 10s 轮询两个端点 | 多详情页打开时后端压力 ×2N |
| G8 | 列表页 30s polling | 状态翻转有最长 30s 延迟 | 启动一个站点要等 30s 看到 Running |

### 🟡 P2 UX / 体积

| ID | Gap |
|---|---|
| G9 | 删除用 `window.confirm`，与设计原型 hi-fi 弹框不一致 |
| G10 | 没有 `/api/admin/sites/{id}/restart` 端点 |
| G11 | 没有批量启停/解析/删除 |
| G12 | 端口冲突仅 runtime 时报，Drawer 创建时不预检 |
| G13 | 日志区只能看末尾 120 行，无加载更多 / 全量下载 |
| G14 | 没有"克隆站点"快捷动作 |
| G15 | 列表页排序固定 `updated_at desc` |
| G16 | 详情 tab 状态不持久化 |
| G17 | viewer URL 仅 client 拼接 |

### 🟢 P3 安全 / 治理

| ID | Gap |
|---|---|
| G18 | 无审计日志 |
| G19 | 无 RBAC（任何 admin 能管所有站点） |
| G20 | 弱凭据黑名单仅 5 条 |
| G21 | 内网 IP 校验缺失（`192.168.*` / `10.*` 无提示） |
| G22 | 子站点 DbOption.toml 无 reload |
| G23 | `design/site-admin-flow-demo` 与生产代码偏离待回归 |
| G24 | runtime_dir/data_dir 缺失只展示 warning，无修复动作 |
| G25 | 无资源限制 / 自愈策略（OOM / CPU 持续高） |

---

## 3. 4-Sprint Backlog

### Sprint C（4-26 → 5-3，约 1 周）· 收口剩余 Sprint B + 修关键 bug

| 序 | 任务 | 估时 | 验收 |
|---|---|---|---|
| **C1** | **B5 Graceful Shutdown** | 2d | `AppState` 加 `shutdown_tx`；`axum::serve(...)` → `with_graceful_shutdown(...)`；`save_site_config` 成功后触发；外层 supervisor 自动拉起。验收：collab-monitor SettingsView 保存→ 5s 内 restart 完成 |
| **C2** | **B6 真热加载（rs-core 跨仓）** | 1.5d | `aios_core::DB_OPTION` `OnceCell` → `RwLock<Arc<DbOption>>`；新增 `set_db_option_from_file()`；`reload_site_config` hot_changed 非空时调用并返回 `actions: ["hot_reloaded"]`。同步回归 plant-model-gen + plant3d-web + pdms-io-fork。验收：改 `enable_log` 后 reload 立即生效 |
| **C3** | **修 G3 baseline 误报** | 0.5d | `reload_site_config` 跳过白名单中"由 env 覆盖的字段"；或在响应里区分 `static_changed_keys_user` vs `static_changed_keys_env`。验收：无人为改动时 baseline 返回 `actions: ["no_change"]` |
| **C4** | **修 G6 admin auth** | 0.5d | 排查 `admin_auth_middleware` 为何对 `/api/remote-sync/*` 不生效；smoke 脚本加 401/403 校验。验收：未登录 curl 返回 401/403 而非 200 |
| **C5** | **修 G5 SSE 漏首事件** | 0.5d | 在 `start_runtime` spawn 完成后再 push 事件；或 `SYNC_EVENT_TX` 加 lag-recovery 重发。验收：100 次 set_master/client，SSE 收齐 100 条 |
| **C6** | **新增 `/api/admin/sites/{id}/restart`** | 0.5d | 后端 stop → 等 SIGTERM 完成 → start，原子化（Mutex 内部）；前端 `SiteDetailHeader` + 列表行加 action。验收：点 "重启" 按钮 ≤ 10s 完成全状态翻转 |

**Sprint C 退出条件**：B5/B6 真落地，G3/G5/G6 修复，新增重启动作。`shells/smoke-collab-api.sh` 升级到 24 项 PASS。

---

### Sprint D（5-4 → 5-10）· 实时化 + 体验

| 序 | 任务 | 估时 |
|---|---|---|
| D1 | 修 G7/G8：admin sites 接 SSE | 2d |
| D2 | 修 G9/G10：hi-fi 删除弹框 + 重启动作 UI | 0.5d |
| D3 | 修 G11：批量操作 | 1d |
| D4 | 修 G12：端口冲突前端预检 + 新端点 `/api/admin/ports/check` | 0.5d |
| D5 | 修 G13：日志全量下载 + 加载更多分页 | 0.5d |
| D6 | 修 G14/G15/G16：克隆站点 + 表头排序 + tab URL 持久化 | 1d |

---

### Sprint E（5-11 → 5-17）· 安全 / 治理

| 序 | 任务 | 估时 |
|---|---|---|
| E1 | 修 G18：审计日志 + 详情页"操作历史" tab | 2d |
| E2 | 修 G19：RBAC 最小可用版（owner / member / viewer） | 2d |
| E3 | 修 G20/G21：弱凭据黑名单扩展 + 内网告警 | 0.5d |
| E4 | 修 G22：子站点 DbOption reload（转发到子进程） | 1d |
| E5 | 修 G24：runtime_dir/data_dir 一键重建 | 0.5d |

---

### Sprint F（5-18 → 5-24）· 架构治理 + 长尾

| 序 | 任务 | 估时 |
|---|---|---|
| F1 | 概念去重（G4）：写《站点系统语义对齐文档》+ 考虑合并 deployment_sites 到 plant-collab-monitor remote-site | 1d 设计 + 3d 实施 |
| F2 | 修 G25：自愈策略（OOM 退避重启 + CPU 持续高降级） | 2d |
| F3 | 修 G23：跑 `site-admin-flow-demo` 7 phase 对比生产，列差异清单 | 1d |
| F4 | 修 G17：viewer URL 后端化（`/api/admin/sites/{id}/viewer-url`） | 0.5d |

---

## 4. 立即落地点（Sprint C 起步）

**C1（B5 Graceful Shutdown）+ C3（修 baseline 误报）**：

- 工作量：~2.5d
- 解锁：collab-monitor SettingsView 真正可用
- 前置：无（不依赖 rs-core 改动）
- 验收单一：保存配置后 5s 内 web_server 自重启，前端从「保存中」直接跳到「已生效」

C1 先于 C2 落地，因为 C1 提供了"重启就能见"的兜底，C2 是 UX 优化。

### C1 实施步骤

1. `web_server/mod.rs::AppState` 加 `shutdown_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>`
2. `web_server/mod.rs:1260` 把 `axum::serve(listener, app).await` 改成：

   ```rust
   let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
   *app_state.shutdown_tx.lock().await = Some(shutdown_tx);
   let serve_result = axum::serve(listener, app)
       .with_graceful_shutdown(async move {
           let _ = shutdown_rx.await;
           println!("📴 收到 graceful shutdown 信号，5s 内停止接受新请求");
       })
       .await;
   ```

3. `site_config_handlers.rs::save_site_config` 成功后：

   ```rust
   if let Some(tx) = state.shutdown_tx.lock().await.take() {
       let _ = tx.send(());
       log::warn!("📴 已触发 graceful shutdown，supervisor 将重启");
   }
   ```

4. `site_config_handlers.rs::restart_server` 改为同样动作（不再是 stub）。
5. 文档：在 README 增加"systemd / nssm 自动重启"配置示例。

### C3 实施步骤

`site_config_handlers.rs::reload_site_config` 内 `diff_db_option` 之后：

- 保留现有 `hot_changed_keys`/`static_changed_keys` 字段
- 新增 `ENV_OVERRIDABLE_KEYS = &["surrealdb"]`（即由 `SURREAL_CONN_*` 环境变量影响的字段）
- 拆出 `static_changed_keys_env`：在 `static_changed_keys` 中过滤出来归到这一类
- `static_changed_keys_user`：剩下的真用户改动
- `requires_restart` 仅用 `static_changed_keys_user.is_empty() == false` 判断
- baseline（无人为改动）→ `actions: ["no_change"]`

向后兼容：保留 `static_changed_keys` 字段（= user + env 合并），前端可继续用旧字段；新字段是增量。

---

## 5. 验收 / 完成定义

### Sprint C 整体退出条件

- [ ] `cargo check --bin web_server --features web_server` 0 error
- [ ] C1 → curl POST `/api/site-config` 后 5s 内 web_server 实际退出，supervisor 自动拉起
- [ ] C2 → 改 `enable_log = false` 后 `curl /api/site-config/reload` 返回 `["hot_reloaded"]` 且日志确实关闭
- [ ] C3 → 启动后立即 `curl POST /api/site-config/reload` 返回 `actions: ["no_change"]`
- [ ] C4 → 未登录 `curl /api/remote-sync/envs` 返回 401/403
- [ ] C5 → 100 次连续 set_master/client，SSE 收齐 100 条事件
- [ ] C6 → 通过 admin UI 详情页"重启"按钮，10s 内站点状态走完 Running → Stopping → Stopped → Starting → Running
- [ ] `shells/smoke-collab-api.sh` ≥ 24/24 PASS

---

## 6. 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| C2 改 rs-core 影响三仓回归量 | 🟡 中 | 先 C1 落地，留兜底；C2 走独立会话 + 全仓 cargo check |
| F1 合并 deployment_sites 涉及数据迁移 | 🟡 中 | 先写迁移脚本 + 双写过渡期 |
| D1 SSE 全量化老浏览器无 EventSource | 🟢 低 | 浏览器目标是 Chromium 90+，全部支持 |
| 4 个 Sprint 累加 ~17d，跨度 4 周可能被打断 | 🟡 中 | 每个 Sprint 独立可发布；优先级随业务调整 |

---

## 7. 不做的事

- ❌ 重写 `managed_project_sites.rs`（3679 行已稳，重写风险高于收益）
- ❌ admin sites 改 GraphQL（现有 REST 已稳定）
- ❌ 引入 K8s-style 编排（单机管理够用）
- ❌ 在 Sprint C 内完成 F1 概念去重（先把站点功能修好再谈架构合并）

---

## 8. 历史 / 关联文档

- 4-13 admin 站点管理 UI 改造（折中混合版）：奠定了当前 admin UI 的视觉骨架
- 4-19 admin 站点部署整改：明确"本机多项目托管"边界
- 4-21 next-iteration P0/P1/P2：4-24 落地 P2/P3/P4，P0 延续到本次 Sprint C
- 4-22 phase-1/3/4 execution：异地协同 phase 1-4 已完成
- 4-24 admin-site-security-hardening：4-24 完成弱凭据/0.0.0.0 拦截
- 4-24 next-sprint-development-plan：本计划承接其 P0 残余项
- 4-26 sprint-b-plan：5/7 phase 完成，Phase 10/11-Plus 由本计划 C1/C2 接力
