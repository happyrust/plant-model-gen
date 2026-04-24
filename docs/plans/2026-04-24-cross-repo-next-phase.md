# 跨仓库下一阶段开发计划 · 2026-04-24

> 承接 plant3d-web 评论真源 PROMOTE 闭环 + plant-model-gen 控制台 Q POS 修复需求，统筹两个仓库的下一步工作。

---

## 0. 当前状态

| 仓库 | 最新提交 | 待执行 |
|------|----------|--------|
| plant3d-web | `f1bf0d0` fix(test) 批注样式断言 | CUTOVER 未执行、DUAL_READ 代码仍在 |
| plant-model-gen | `77d8545` feat(admin,collab,review) | Q POS fallback 计划未执行 |

### plant3d-web 关键状态

- 批注表格 MVP++ (PR 1–10): ✅ 全部完成并提交
- 评论真源 PROMOTE (Step 1–4): ✅ 全部完成并提交
- DUAL_READ flag: 仍为 `true`，`commentThreadDualRead.ts` 仍存在
- CUTOVER: **未执行**

### plant-model-gen 关键状态

- Console Q POS: `get_transform` 硬依赖 `pe_transform` 缓存表，缓存不可用时链路失效
- `compute_transform` 实时计算 API 已存在但未被 `get_transform` 利用
- Admin 站点部署整改计划: 5 阶段计划已撰写，尚未开始

---

## 1. Sprint 1: CUTOVER + Q POS Fallback (~6h)

### Task A: plant3d-web CUTOVER（~4h）

依据 `plant3d-web/docs/plans/2026-04-24-next-phase-development-plan.md` Phase D。

| 步骤 | 文件 | 改动 |
|------|------|------|
| A1 | `src/review/flags.ts` | 移除 `REVIEW_C_COMMENT_THREAD_STORE_DUAL_READ` 常量 |
| A2 | `src/review/flags.test.ts` | 移除对应测试 |
| A3 | `src/review/services/commentThreadDualRead.ts` | 删除文件 |
| A4 | `src/review/services/commentThreadDualRead.test.ts` | 删除文件 |
| A5 | `src/components/review/embedFormSnapshotRestore.ts` | 移除 `isReviewCommentThreadStoreActive()` 分支 |
| A6 | `src/components/review/ReviewCommentsTimeline.vue` | 移除 dual-read 分支 |
| A7 | `src/review/services/sharedStores.ts` | 简化 dual-read 相关逻辑 |
| A8 | `src/review/services/sharedStores.test.ts` | 更新测试 |
| A9 | `src/composables/useToolStore.ts` | `annotations` 降级为 computed 只读或移除 |
| A10 | `src/review/services/commentEventLog.ts` | 移除 `dual_read_diff` 类型写入 |
| A11 | 全项目搜索 | 确认无 DUAL_READ 引用残留 |

**门槛**: vitest 全绿、无 DUAL_READ 引用残留。

### Task B: plant-model-gen Q POS Fallback（~2h）

依据 `plant-model-gen/docs/plans/2026-04-23-console-q-pos-transform-fallback.md`。

| 步骤 | 文件 | 改动 |
|------|------|------|
| B1 | `src/web_api/pdms_transform_api.rs` | `get_transform` 增加 compute fallback |
| B2 | 同上 | 抽取 `compute_world_transform_fallback` helper |
| B3 | `cargo build -p plant-model-gen` | 编译验证 |

### Task C: plant3d-web Q POS 前端文案（~0.5h）

| 步骤 | 文件 | 改动 |
|------|------|------|
| C1 | `src/composables/usePdmsConsoleCommands.ts` | Q POS fallback 标签改为 `Attr, local — world not available` |
| C2 | 同上 | Q ORI fallback 同理 |

---

## 2. Sprint 2: E2E + 代码清理（~4h）

待 Sprint 1 完成后执行。

| Task | 描述 | 预估 |
|------|------|------|
| E2E 验证 | annotation-table-ribbon.spec.ts 对齐 | ~1h |
| E2E 关键路径 | Designer 表格搜索、Reviewer 工作台 | ~1h |
| 代码清理 | @deprecated 方法移除、类型清理 | ~2h |

---

## 3. Sprint 3: Admin 站点部署整改（~8h）

待 Sprint 2 完成后执行，依据 `plant-model-gen/docs/plans/2026-04-19-admin-站点部署整改计划.md`。

| Phase | 描述 | 预估 |
|-------|------|------|
| Phase 1 | 安全默认值收口 | ~2h |
| Phase 2 | 状态机动作约束 | ~3h |
| Phase 4 | Viewer URL 配置化 | ~2h |
| Phase 5 | 错误反馈补强 | ~1h |

---

## 4. 执行顺序

```
Sprint 1 (CUTOVER + Q POS)  → Sprint 2 (E2E + 清理) → Sprint 3 (Admin 整改)
        ~6h                          ~4h                       ~8h
```

**推荐立即执行 Sprint 1**，CUTOVER 是最高优先级（PROMOTE 已闭环，拖延增加双份代码维护成本）。

---

## 5. 风险

| 风险 | 可能性 | 缓解 |
|------|--------|------|
| CUTOVER 后遗漏引用导致运行时错误 | 低 | 全量搜索 + type-check |
| Q POS fallback 首次请求延迟 | 低 | 控制台点查场景，毫秒级可接受 |
| Admin 整改范围蔓延 | 中 | 严格按阶段切分，每阶段独立验证 |
