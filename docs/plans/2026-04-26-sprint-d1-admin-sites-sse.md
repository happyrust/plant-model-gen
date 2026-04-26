# Sprint D1 · admin sites 接 SSE 实时化（修 G7/G8）

> 上游：`docs/plans/2026-04-26-site-admin-next-steps.md` § 3 Sprint D · D1（2d，修 G7/G8 admin sites 接 SSE）
> 关联：`docs/plans/2026-04-26-sprint-c-verification-report.md`（Sprint C 5/6 完成基线）

---

## 0. 一句话目标

把 admin sites 列表/详情页的 30s/10s 双 polling 替换为 **SSE 增量推送**，做到「启动一个站点立刻看到状态翻转」+「资源指标 5s 一刷而非 10s」+「关闭多余的轮询请求」。

---

## 1. 背景与现状

### 1.1 现状（已调研结论）

**后端**（`src/web_server/`）：

- ✅ SSE 框架就位：`sse_handlers.rs::SyncEvent` enum + `sync_control_center::SYNC_EVENT_TX` broadcast channel + `sync_events_handler` 暴露 `/api/sync/events`（带初始 snapshot 防漏首事件）
- ✅ `update_runtime`（`managed_project_sites.rs`）是状态写盘的 **single entry point**：`start_site`（L3146）/ `stop_site`（L3266）/ parse 失败回退（L3204-3213）/ start 失败回退（L3155-3164）等所有路径都走它
- ⏸ 当前 `SyncEvent` 12 个变体均面向 collab/sync/MQTT，**无 admin sites 状态事件**
- ⏸ `runtime_status` 端点（L3412）只读、靠前端 polling 触发；`ResourceCollector` 周期采集后写 SQLite，无主动推送

**前端**（`ui/admin/src/`）：

- ✅ `composables/useCollaborationStream.ts`：完整 SSE composable（指数退避重连 + autoConnect + dispatchEvent + dev mock）—— 可作为 `useAdminSitesStream` 模板
- ✅ `composables/usePolling.ts`：极简 setInterval 封装，当前 `SitesView` / `SiteDetailView` 都在用（30s 列表 + 10s detail runtime+logs）
- ✅ `stores/sites.ts`：`fetchSites` 全量刷新；CRUD action 后都跟 `await fetchSites()`，**全量刷新开销大且会和 SSE patch 互相干扰**

**Gap 量化**（用户能感受到的）：

| 痛点 | 当前 | 目标 |
|---|---|---|
| 启动 site → 看到 Running | 最长 **30s 延迟**（列表 polling 间隔） | **≤ 1s**（SSE 推送） |
| 资源指标更新 | **10s polling**，多详情页 N×polling | **5s 节流推送**，单 SSE 通道 |
| 解析进度可见性 | 仅终态（Pending/Running/Done/Failed） | 中间状态实时（含 progress） |
| 多详情页打开 | 每页 2 个 polling × N 页 = 2N 请求/10s | 0 polling + 1 SSE |
| 站点 C/U/D 跨标签页同步 | 只有当前标签页知道 | 所有打开 admin UI 的标签页同步 |

### 1.2 设计决策

| 决策 | 选项 | 选择 | 理由 |
|---|---|---|---|
| **broadcast channel** | 复用 `SYNC_EVENT_TX` / 新开 admin 专用 channel | **复用** | 单一事件流，前端按 `type` 字段过滤；admin sites 状态本就在 list endpoint 暴露，无新增信息泄露 |
| **SSE 端点** | 复用 `/api/sync/events` / 新开 `/api/admin/sites/events/stream` | **复用 + 别名** | 主路径用 `/api/sync/events`，admin SSE 也走它；别名端点留给后续做 admin auth 加固时切换 |
| **资源指标推送频率** | 跟 ResourceCollector 同 5s / 节流到 10-15s | **5s（同采集周期，无额外节流）** | 采集已是节流，SSE broadcast 极轻 |
| **事件粒度** | 每字段单独事件 / 整 site snapshot 事件 | **snapshot + delta 混合** | status/parse_status 翻转用 snapshot；resource 单独走 resource event；create/delete 单独走列表级事件 |
| **前端 polling 是否保留** | 删 / 留作 fallback | **留 60s 兜底刷新 + SSE 主线** | SSE 偶有断流时不至于状态错位；SSE 重连成功立即触发一次全量刷新 |
| **断流期处理** | drop / replay queue | **drop + 重连后 force fetchSites** | broadcast channel 本就是 drop 语义；admin sites 状态最终一致即可，无需 replay |

---

## 2. 事件型谱（payload 设计）

新增 4 个 `SyncEvent` 变体，**全部 `admin_site_` 前缀**避免与已有 collab 变体混淆：

```rust
SyncEvent::AdminSiteSnapshot {
    site_id: String,
    project_name: Option<String>,
    status: String,         // "Running" / "Stopped" / "Starting" / ...
    parse_status: String,   // "Idle" / "Running" / "Done" / "Failed"
    last_error: Option<String>,
    timestamp: String,
}

SyncEvent::AdminSiteResource {
    site_id: String,
    cpu_percent: Option<f32>,
    memory_percent: Option<f32>,
    disk_percent: Option<f32>,
    risk_level: String,     // "normal" / "warning" / "critical"
    timestamp: String,
}

SyncEvent::AdminSiteCreated {
    site_id: String,
    project_name: String,
    timestamp: String,
}

SyncEvent::AdminSiteDeleted {
    site_id: String,
    timestamp: String,
}
```

**为什么不需要 `AdminSiteUpdated`**：
- 元数据修改（project_name 等）也走 `AdminSiteSnapshot`，前端拿到 site_id + project_name 后直接 patch 即可
- 全量元数据更新少见且不实时（用户改完会自己刷新），不值得专门一个事件类型

---

## 3. 推送时机（注入点）

### 3.1 `update_runtime`（single source of truth）

`managed_project_sites.rs::update_runtime`（同步函数，事务写盘）末尾注入：

```rust
// SQLite 事务 commit 后 push snapshot
push_admin_site_snapshot(site_id, &updated_site);
```

覆盖路径（自动跑通无遗漏）：
- ✅ `start_site` Starting / Running / Failed
- ✅ `stop_site` Stopping / Stopped / Failed
- ✅ `restart_site`（= stop_site + start_site，自动覆盖）
- ✅ `parse_site` Running / Done / Failed
- ✅ 后台 `run_start_pipeline` / `run_parse_pipeline` 失败回退

### 3.2 CRUD handler

直接在 `admin_handlers.rs` 的 `create_site` / `update_site` / `delete_site` handler 内、写盘成功后 push：

| Handler | 推送事件 | 备注 |
|---|---|---|
| `create_site` | `AdminSiteCreated` | 列表级新增 |
| `update_site` | `AdminSiteSnapshot` | 元数据更新触发 patch |
| `delete_site` | `AdminSiteDeleted` | 列表级删除 |

### 3.3 ResourceCollector

`managed_project_sites.rs` 中资源采集 loop（5s 周期）每次采集完成后：

```rust
push_admin_site_resource(site_id, &metrics);
```

仅当 site `status == Running` 时推送（停止站点不刷资源）。

---

## 4. Phase 拆分与执行顺序

### Phase 1（0.5d）· 后端事件型谱 + push helper

- [ ] `sse_handlers.rs::SyncEvent` 增加 4 个变体（`#[serde(tag="type", content="data")]` 已有）
- [ ] `sync_control_center.rs` 或新文件 `admin_site_events.rs` 定义 4 个 helper：
  - `push_admin_site_snapshot(site_id, ManagedProjectSite | ManagedSiteRuntimeStatus)` 
  - `push_admin_site_resource(site_id, ManagedSiteResourceMetrics)`
  - `push_admin_site_created(site_id, project_name)`
  - `push_admin_site_deleted(site_id)`
- [ ] helper 内部容错：`SYNC_EVENT_TX.send(...).ok();`（broadcast 无订阅者时丢弃，不报错）
- [ ] `cargo check --bin web_server --features web_server` 0 error

### Phase 2（0.25d）· `update_runtime` 注入

- [ ] `update_runtime` 末尾（事务 commit 之后、return Ok 之前）调 `push_admin_site_snapshot`
- [ ] 拿到 `updated_site` 的方式：要么再读一次（极轻），要么从 RuntimeUpdate + 内存表 merge 出 snapshot
- [ ] 简单先读一次（`get_site` 同进程 SQLite read 极快），后续若性能瓶颈再优化为 merge
- [ ] 验证：`curl -N /api/sync/events` + 另一终端 `start_site` → SSE 应连续推 Starting / Running 两条 snapshot

### Phase 3（0.25d）· CRUD handler 注入

- [ ] `admin_handlers.rs::create_site` 末尾推 `AdminSiteCreated`
- [ ] `admin_handlers.rs::update_site` 末尾推 `AdminSiteSnapshot`（含最新元数据）
- [ ] `admin_handlers.rs::delete_site` 末尾推 `AdminSiteDeleted`
- [ ] 验证：admin UI A 标签页 create/update/delete，B 标签页 SSE 收到事件

### Phase 4（0.5d）· 前端 useAdminSitesStream + store handler

- [ ] 复制 `useCollaborationStream.ts` 为 `useAdminSitesStream.ts`，回调签名换成 4 个新事件
- [ ] `stores/sites.ts` 新增 4 个 patcher：`patchSiteSnapshot` / `patchSiteResource` / `addSiteFromEvent` / `removeSiteFromEvent`
- [ ] CRUD action 中**不再 `await fetchSites()`**，仅触发后端写盘后等 SSE 来 patch（保留作为 fallback：action 后 1s 内未收到 SSE 自己 fetchSites）
- [ ] 重连成功事件 → 触发一次 `fetchSites()` 全量同步（防止断流期间错过事件）
- [ ] 编译检查（pnpm build）

### Phase 5（0.25d）· View 接入 + UI 状态徽标

- [ ] `SitesView.vue` 顶部加「实时已连接 / 重连中 #N · Xs 后重试」徽标
- [ ] `SitesView.vue` polling 间隔从 30s → 60s（兜底刷新）；保留 polling 但延长间隔
- [ ] `SiteDetailView.vue` 移除 runtime 10s polling（SSE 推送 snapshot 已覆盖），保留 logs polling（日志不在 SSE 范围）
- [ ] `SiteDetailView.vue` resource 区接入 `AdminSiteResource` 事件 → 实时更新仪表盘
- [ ] 编译 + 浏览器手测

### Phase 6（0.25d）· 验证

- [ ] **后端 curl 验证**：
  - `curl -N http://127.0.0.1:3100/api/sync/events` 持续接收
  - 另一终端 `curl -X POST .../sites/{id}/start` → SSE 应 1s 内推送 `AdminSiteSnapshot { status: "Starting" }` 和 `{ status: "Running" }`
  - 创建/删除/更新 site 同样验证
- [ ] **浏览器联调**：
  - 双标签页打开 admin UI，A 标签页 create/start，B 标签页 1s 内列表刷新
  - 关闭/重启 web_server，前端徽标进入「重连中」并指数退避
  - 重连成功后徽标恢复 + 列表自动 fetchSites 一次
- [ ] **回归**：`shells/smoke-collab-api.sh` 仍 20/20 PASS（不破坏现有 SSE 流）

---

## 5. 验收清单

- [ ] **A1（功能）**：admin UI 启动 site → ≤ 1s 看到 Running 翻转
- [ ] **A2（功能）**：A 标签页创建/删除 site → B 标签页 ≤ 2s 内列表同步
- [ ] **A3（功能）**：详情页 CPU/内存/磁盘 5s 一刷
- [ ] **A4（断流）**：杀死 web_server → 前端进入重连，恢复后 ≤ 5s 全量刷新
- [ ] **A5（性能）**：单标签页打开 admin UI，5min 内 polling 请求数 ≤ 5（仅 60s 兜底 + 启动时 1 次 fetchSites）
- [ ] **A6（兼容）**：`shells/smoke-collab-api.sh` ≥ 20/20 PASS
- [ ] **A7（编译）**：`cargo check --bin web_server --features web_server` 0 error；前端 `vue-tsc -b` 0 error

---

## 6. 风险与回退

| 风险 | 等级 | 缓解 |
|---|---|---|
| update_runtime 内 `get_site` 引入读放大（每次状态变更读一次） | 🟢 低 | 同进程 SQLite 单表 by-pk read ≪ 1ms；如有瓶颈改 merge |
| broadcast 无订阅者时 send 报错 | 🟢 低 | helper 内 `.ok()` 忽略，已是惯用模式 |
| ResourceCollector 5s 节流过密导致 SSE 客户端缓冲膨胀 | 🟡 中 | 资源事件按 site_id 仅留最新（前端 patch 即可，不堆积） |
| 前端两标签页 SSE 同步竞争 | 🟢 低 | 本就最终一致，互不干扰；fetchSites 是幂等的 |
| 老浏览器无 EventSource | 🟢 低 | admin UI 目标 Chromium 90+，全部支持 |

**回退策略**：所有改动隔离在新增的 `push_admin_site_*` helper 与新 `useAdminSitesStream` composable 中；如果出现状态错位，前端 polling 兜底仍然工作（60s 一刷），单一 git revert 即可恢复纯 polling 模式。

---

## 7. 不做的事

- ❌ 不动 SSE 通道架构（`SYNC_EVENT_TX` 单 broadcast 复用）
- ❌ 不做 admin SSE 端点的 admin auth 加固（事件内容不敏感；如需 auth 后续单独 issue）
- ❌ 不做 logs SSE 推送（本 Sprint 范围；日志 polling 保留，logs 量大不适合 broadcast）
- ❌ 不做事件 replay/persist（broadcast drop 语义已够用，最终一致性足够）

---

## 8. 提交策略

| Commit | 范围 | 描述 |
|---|---|---|
| 1 | Phase 1+2 | `feat(admin-sites): D1 Phase1+2 · SSE 事件型谱 + update_runtime 注入推送 (G7/G8 后端基线)` |
| 2 | Phase 3 | `feat(admin-sites): D1 Phase3 · CRUD handler push events` |
| 3 | Phase 4 | `feat(admin-sites): D1 Phase4 · useAdminSitesStream + store SSE patcher` |
| 4 | Phase 5 | `feat(admin-sites): D1 Phase5 · SitesView/SiteDetailView 实时化 + 连接徽标` |
| 5 | Phase 6 | `docs(plans): Sprint D1 验收报告` |

每个 commit 独立可发布，验证通过即推送 origin。
