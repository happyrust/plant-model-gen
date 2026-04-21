# 异地协同 v2 · 开发计划

> 制定日期：2026-04-21
>
> 产出目标：把 `design/collaboration-v2/index.html` v0.4 原型的能力完整迁入 `ui/admin/` 生产代码。
>
> 里程碑：完成后 `/admin/#/collaboration` 达到 v0.4 截图同等的视觉与交互能力。

## 前置完成情况

| Phase | 状态 | 产出 |
|---|---|---|
| 1 · Design token | ✅ | `ui/admin/src/style.css` 追加 `--collab-*` |
| 2 · TopologyPanel 组件 | ✅ | `components/collaboration/CollaborationTopologyPanel.vue` |
| 3A · Workbench 追加折叠预览 | ✅ | `CollaborationWorkbenchView.vue` 挂载 `<details>` |
| 4 · 类型 + Store 扩展 | ✅ **本轮完成** | `types/collaboration.ts` +7 类型 · `stores/collaboration.ts` +7 refs +6 actions |
| 5A · ActiveTasksCard | ✅ **本轮完成** | `components/collaboration/CollaborationActiveTasks.vue` |
| 5B · FailedTasksCard | ✅ **本轮完成** | `components/collaboration/CollaborationFailedTasks.vue` |
| 5C · ConfigDrawer | ✅ **本轮完成** | `components/collaboration/CollaborationConfigDrawer.vue` |
| 6 · TopologyPanel 增强 | ✅ **本轮完成** | 节点内嵌 5 态 chip · site_id 角标 · progress bar |
| 8 · 暗色主题 | ✅ **本轮完成** | `style.css` 追加 `[data-theme="dark"]` 下 `--collab-*` 覆盖 |

**本轮 bundle 增量**: CollaborationWorkbenchView 82.91 → 83.98 kB (+1.07 kB)，构建仍在 0.65s。

## 剩余 Phase（按依赖顺序）

```
Phase 4  类型 + Store 扩展             ←打底，所有后续组件依赖
  ↓
Phase 5A ActiveTasksCard 组件
Phase 5B FailedTasksCard 组件           ←可并行
Phase 5C ConfigDrawer  组件
  ↓
Phase 6  TopologyPanel 增强（状态 chip / site_id / progress）
  ↓
Phase 7  Workbench 4-Tab 重构（破坏性）
  ↓
Phase 8  暗色主题支持
  ↓
Phase 9  实时通道（SSE 优先，WebSocket 候选）←需要后端
  ↓
Phase 10 回归 + 文档
```

---

## Phase 4 · 类型 + Store 扩展（纯追加，零风险）

**目标**：为后续组件提供类型基础，不改任何现有运行时逻辑。

**改动**：

1. `ui/admin/src/types/collaboration.ts`：
   - `CollaborationSiteCard` 追加：`site_id?: string`、`detection_status?: CollaborationDetectionStatus`、`progress?: number | null`、`pending_items?: number`、`synced_items?: number`
   - 新增 `type CollaborationDetectionStatus = 'Idle' | 'Scanning' | 'ChangesDetected' | 'Syncing' | 'Completed' | 'Error'`
   - 新增 `interface CollaborationActiveTask { task_id; site_id; site_name; task_name; file_path; progress; status; }`
   - 新增 `interface CollaborationFailedTask { id; task_type; site; error; retry_count; max_retries; first_failed_at; next_retry_at; }`
   - 新增 `interface CollaborationConfig { auto_detect; detect_interval; auto_sync; batch_size; max_concurrent; reconnect_initial_ms; reconnect_max_ms; enable_notifications; log_retention_days; }`
   - 新增 `interface CollaborationToast { id; type; icon; title; message; at; }`
2. `ui/admin/src/stores/collaboration.ts`：
   - 新增 `ref<CollaborationActiveTask[]>` `activeTasks`（默认空数组）
   - 新增 `ref<CollaborationFailedTask[]>` `failedTasks`（默认空数组）
   - 新增 `ref<CollaborationConfig>` `collabConfig`（带默认值常量）
   - 新增 `ref<boolean>` `realtimeConnected`（默认 false，Phase 9 才真的连接）
   - 新增 `ref<CollaborationToast[]>` `toasts`
   - 新增 actions：`pushToast(t)`, `dismissToast(id)`, `fetchActiveTasks()`, `fetchFailedTasks()`, `fetchConfig()`, `saveConfig(c)` —— 先留空实现或 mock 返回

**风险**：零。纯类型 + 纯追加。

**验证**：`npx vue-tsc -b` 零错误；`npm run build` 通过。

**估时**：15 分钟。

---

## Phase 5A · ActiveTasksCard 组件

**目标**：把 v0.4 的「活跃任务条」变成可复用 Vue 组件。

**新文件**：`components/collaboration/CollaborationActiveTasks.vue`

**Props**：`items: CollaborationActiveTask[]`、`loading?: boolean`

**Emit**：`abort: [taskId]`（可选）

**视觉**：严格对齐 v0.4 的 `.ic-active-card` 结构（scoped style 消费 `--collab-*`）。

**挂载点**：Phase 7 时挂到 Workbench 的「日志 Tab 顶部」或「Dashboard 区」。Phase 5A 本身只提供组件。

**估时**：20 分钟。

## Phase 5B · FailedTasksCard 组件

**目标**：把 v0.4 的失败任务队列抽成组件。

**新文件**：`components/collaboration/CollaborationFailedTasks.vue`

**Props**：`items: CollaborationFailedTask[]`、`loading?: boolean`

**Emit**：`retry: [id]`, `cleanup: []`

**估时**：15 分钟。

## Phase 5C · ConfigDrawer 组件

**目标**：4 分组 8 项参数配置抽屉。

**新文件**：`components/collaboration/CollaborationConfigDrawer.vue`

**Props**：`open`, `config`, `disabled?`, `save: (c) => Promise<void>`

**Emit**：`close`

**估时**：25 分钟。

---

## Phase 6 · TopologyPanel 增强

**目标**：原版 `CollaborationTopologyPanel.vue` 补齐 v0.4 新增的 site_id 角标、5 态 chip、progress、站点内数字 tabular-nums。

**改动**：扩展现有组件模板和 scoped style，不新增文件。

**风险**：低，已挂载到 Workbench 的折叠预览块，用户会看到变化但不会破坏其他页面。

**估时**：20 分钟。

---

## Phase 7 · Workbench 4-Tab 重构（破坏性）

**目标**：把 `CollaborationWorkbenchView.vue` 从一页式 6 卡重构为 4-Tab 壳（拓扑/站点/洞察/日志），对齐 v0.4 原型的信息架构。

**改动**：
1. 模板：删除直接挂载 Overview/Sites/Insights/Logs Panel 的代码
2. 新增 `<Tabs>` 壳（可用 radix-vue 或手写）
3. 保留 `GroupDetailHeader`（作为 sticky 顶部）
4. Tab 1（拓扑）→ `CollaborationTopologyPanel`（从 `<details>` 提升为默认 Tab）
5. Tab 2（站点）→ `GroupSitesPanel`（+ Phase 8 会重构成表格形态）
6. Tab 3（洞察）→ `GroupInsightsPanel` + `CollaborationFailedTasks`
7. Tab 4（日志）→ `GroupLogsPanel`（顶部放 `CollaborationActiveTasks`）
8. URL hash 路由 `#topo / #sites / #insight / #logs`

**风险**：中 · 改动现有主视图。需要：
- 先在 feature branch 验证
- 跑 `npm run build` 保证编译通过
- 手动验收：浏览器访问 `/admin/#/collaboration`，四个 Tab 都能切换、数据能显示

**回滚预案**：保留原 v0.3 代码在 git 历史，如果破坏可 revert 单个 commit。

**估时**：40 分钟。

---

## Phase 8 · 暗色主题

**目标**：`--collab-*` 扩展出 `.dark` 变体，Workbench 相关组件自动跟随 admin 主题切换（如果 admin 有全局主题开关则接入，否则组件内独立切换）。

**改动**：
1. `style.css` 追加 `.dark { --collab-*: ...; }`
2. 确认 TopologyPanel / ActiveTasksCard / FailedTasksCard / ConfigDrawer 的 scoped style 都只使用 CSS 变量（Phase 5 已遵守）

**风险**：低。

**估时**：15 分钟。

## Phase 9 · 实时通道（依赖后端）

**前置条件**：后端提供其一
- SSE：`GET /api/remote-sync/events/stream`（推荐，单向足够）
- WebSocket：`/ws/remote-sync`

**改动**：
1. `ui/admin/src/composables/useCollaborationStream.ts` 新建
2. 在 Workbench mount 时连接，断线重连带指数退避（消费 `collabConfig.reconnect_*_ms`）
3. 事件类型：`active_task_update` / `failed_task_new` / `site_status_change` / `sync_completed` / `sync_failed`
4. Store 根据事件更新 `activeTasks / failedTasks / siteCards / toasts`
5. `realtimeConnected` 跟随连接状态

**风险**：中 · 需要后端配合。如果 Phase 9 推迟，v0.4 的视觉价值已有，实时性通过轮询兜底。

**估时**：60–90 分钟（前端），后端 60 分钟。

---

## Phase 10 · 回归 + 文档

**改动**：
1. 更新 `docs/development/admin/异地协同功能架构文档.md`：补一节「v2 迁移后信息架构」
2. CHANGELOG.md 写一条记录
3. 清理 `design/collaboration-v2/` 下不再需要的备份（保留 README/GAP_ANALYSIS/DEVELOPMENT_PLAN + 最终截图）

**估时**：20 分钟。

---

## 总估时

| Phase | 估时 |
|---|---|
| 4 | 15 min |
| 5A | 20 min |
| 5B | 15 min |
| 5C | 25 min |
| 6 | 20 min |
| 7 | 40 min |
| 8 | 15 min |
| 9 | 90 min + 后端 |
| 10 | 20 min |
| **合计** | **≈ 4 小时**（不含 Phase 9 后端） |

## 执行策略

- 每个 Phase 结束立刻跑 `npx vue-tsc -b` 和 `npm run build`
- Phase 7（破坏性）前把 Workbench 当前版本 commit 成干净节点
- Phase 9 如果后端未就绪，用 mock SSE（`EventSource` + 本地静态 `events.json`）先验证前端，后端就绪再切换 URL
- 每个 Phase 的 commit 消息用 `collaboration(vN): ...` 前缀

## 风险汇总

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| Phase 7 破坏现有页面 | 中 | 中 · admin 首页不可用 | 保留 git 回滚点；先在 `<details>` 预览里验收拓扑再做 Workbench 重构 |
| Phase 9 后端 API 未就绪 | 高 | 低 · 实时性缺失但不影响基本功能 | 前端先做 mock SSE；后端就绪再切 URL |
| vue-tsc 严格模式下新类型联动报错 | 中 | 低 · 编译失败 | Phase 4 单独跑 type check，及时修 |
| v0.4 的纯 CSS token 被 Tailwind 覆盖 | 低 | 中 · 视觉走样 | scoped style + 前缀 `.collab-v2` 命名空间 |
