# 异地协同 UI 发布收口计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 完成异地协同 UI 修复后的发布前收口，明确源码修复、静态构建产物、验证证据与剩余运行态边界。

**Architecture:** 本轮不扩展功能，只围绕 `ui/admin` 的协同工作台修复和 `src/web_server/static/admin` 的生产构建产物做发布就绪确认。可安全执行的动作包括差异审计、构建/预览验证和文档归档；`apply`、`activate`、`stop` 等会改变运行态的操作必须等用户明确授权。

**Tech Stack:** Vue 3、Vue Router hash history、Vite build output、Rust `web_server` 静态资源目录。

---

## 1. 当前状态

已完成并验证的内容：

- `ui/admin/src/views/CollaborationWorkbenchView.vue` 已把内部 Tab 状态从 `location.hash` 改为路由 query `tab`，避免 Vue Hash 路由被污染成 `#/sites`。
- `npm run build` 已通过，覆盖 `vue-tsc -b && vite build`。
- 浏览器复验已覆盖登录跳转、协同页加载、站点排序、日志特殊字符高亮、新增站点抽屉取消、metadata 成功诊断、MQTT 成功诊断。
- 临时 env/site、临时日志、临时 `metadata.json`、临时 MQTT broker 均已清理或停止。

剩余发布前问题：

- 当前工作区包含 `src/web_server/static/admin/assets` 的旧 hash 删除与新 hash 新增，需要在提交前作为一组构建产物纳入。
- 若要做更深运行态闭环，仍需用户明确允许执行 `applyEnv`、`activateEnv`、`stopRuntime`，因为这些操作会写配置或影响 watcher/MQTT 运行态。

## 2. 下一步方案

采用“三段式收口”：

1. **差异审计**：确认源码修复、计划文档、静态产物 hash 替换属于同一发布包；识别不应纳入的临时文件。
2. **发布验证**：复跑最小安全验证：`ui/admin` 构建、静态首页 HTTP 冒烟、必要时浏览器打开协同页确认路由 query 行为。
3. **归档与交接**：更新计划/进度，标明哪些操作已完成、哪些运行态操作需要授权后再做。

不做：

- 不运行单元测试或 cargo test。
- 不执行 `apply`、`activate`、`stop`。
- 不回滚用户已有改动或清理未知文件。

## 3. 执行任务

### Task 1: 差异审计

**Files:**

- Inspect: `ui/admin/src/views/CollaborationWorkbenchView.vue`
- Inspect: `src/web_server/static/admin/index.html`
- Inspect: `src/web_server/static/admin/assets/*`

**Steps:**

1. 执行 `git status --short`、`git diff --name-status`、`git diff --stat`。
2. 检查 `index.html` 当前引用的新 hash 资源是否都存在。
3. 记录是否存在临时验证残留文件。

**Expected:** `index.html` 引用资源存在；静态资产表现为旧 hash 删除、新 hash 新增；无临时 metadata 或 env/site 文件需要随提交纳入。

### Task 2: 最小发布验证

**Files:**

- Run in: `ui/admin`
- Serve from: `src/web_server/static/admin`

**Steps:**

1. 执行 `npm run build`。
2. 启动或复用静态预览服务。
3. 访问 `/admin/static/` 或 Vite preview 首页确认 HTTP 200。

**Expected:** 构建 0 error；首页返回 200；如端口占用，记录实际端口。

### Task 3: 文档与进度收尾

**Files:**

- Modify: `docs/plans/2026-04-27-remote-collab-ui-release-readiness-plan.md`
- Modify if needed: `docs/plans/2026-04-27-remote-collab-ui-browser-verification-plan.md`

**Steps:**

1. 把 Task 1/2 结果写入本计划的执行记录。
2. 更新 `my-mcp-20` 进度存档。
3. 向用户汇报剩余授权项。

**Expected:** 发布收口状态清晰；未授权的运行态操作不被误认为已完成。

## 4. 执行记录

执行时间：2026-04-27

| 项目 | 结果 | 备注 |
|---|---|---|
| 计划文件创建 | 通过 | 本文件已创建 |
| 工作区差异审计 | 通过 | 已确认当前包含 `CollaborationWorkbenchView.vue`、`src/web_server/static/admin/index.html`、静态 assets hash 替换及三个计划文档 |
| `index.html` 引用资源存在性 | 通过 | `index-Cb19ziIk.js`、`client-CRheT3WI.js`、`runtime-dom.esm-bundler-MBNOHA1O.js`、`app-config-B9bCphE1.js`、`auth-BX37bPLx.js`、`index-laq6dEpu.css` 均存在 |
| `npm run build` | 通过 | `vue-tsc -b && vite build` 0 error，Vite 构建耗时约 624ms |
| 预览首页 HTTP 冒烟 | 通过 | 复用已运行的 `127.0.0.1:4173/admin/static/`，返回 `StatusCode=200; Length=885` |
| `git diff --check` | 通过 | 首次发现 `src/web_server/static/admin/index.html` 为 CRLF 导致 trailing whitespace 报告；已规范为 LF 后复查通过 |
| 提交前文件范围确认 | 通过 | 当前发布相关范围为 1 个源码修复、1 个静态入口、静态 assets 旧 hash 删除/新 hash 新增、3 个计划文档；未发现需要纳入的临时 metadata/env/site 文件 |
| 运行态 apply/activate/stop | 未执行 | 需用户明确授权 |

## 5. 完成定义

- [x] 创建下一步发布收口计划。
- [x] 初步确认静态首页引用的新 hash 资源存在。
- [x] 复跑最小构建或预览验证。
- [x] 更新进度存档。
- [x] 明确提交前应纳入的文件范围。

## 6. 提交前文件范围建议

建议纳入同一提交：

- `ui/admin/src/views/CollaborationWorkbenchView.vue`
- `src/web_server/static/admin/index.html`
- `src/web_server/static/admin/assets/*` 中本次构建产生的新 hash 文件与对应旧 hash 删除
- `docs/plans/2026-04-27-remote-collab-ui-browser-verification-plan.md`
- `docs/plans/2026-04-27-vue-migration-m4-deployment-verification-plan.md`
- `docs/plans/2026-04-27-remote-collab-ui-release-readiness-plan.md`

提交前确认结果：`src/web_server/static/admin/assets` 的删除/新增应作为同一次 `npm run build` 的完整输出纳入；不要只提交 `index.html` 或只提交部分 hash 资源。
