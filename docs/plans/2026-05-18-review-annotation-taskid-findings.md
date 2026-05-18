# JD/JH 处理 SJ 退回批注开发发现

## Requirements

- 使用文件化 planning 方式，用中文提出下一步详细开发方案。
- 方案聚焦 JD/JH 无法对 SJ 处理过的退回批注执行同意/驳回的问题。
- 不覆盖根目录已有 DuckLake planning 文件。

## Research Findings

- `review_annotation_states` 的后端真源维度是 `(form_id, task_id, annotation_type, annotation_id, review_round)`。
- 外部 PMS 按同一 `form_id` 打开时，SJ、JD/JH 可能恢复到不同内部 `taskId`。
- 前端读取侧已经在外部 form 聚焦模式下按 `form_id` 拉取状态，但提交侧原来仍把 `currentTask.id` 传给 `ReviewCommentsTimeline`。
- 后端 `AnnotationStateView` 已返回 `taskId`，但前端转换为 `AnnotationReviewState` 时曾丢弃该字段。
- `ReviewCommentsTimeline` 的正式提交要求同时具备 `formId + taskId`，因此错误或缺失的 `taskId` 会直接导致同意/驳回不可用或写到错误状态行。

## Technical Decisions

| Decision | Rationale |
|----------|-----------|
| 扩展 `AnnotationReviewState` 保存 `formId/taskId/workflowNode/reviewRound` | 避免跨层转换时丢失后端状态血缘。 |
| `normalizeAnnotationReviewStateView()` 负责桥接后端状态行到前端状态模型 | 数据契约收口在 API adapter，组件不直接理解后端原始字段。 |
| `ReviewPanel.resolveAnnotationActionTaskId()` 优先使用 `item.reviewState.taskId` | 批注卡片项已包含归一化状态，是组件层最自然的提交上下文来源。 |
| 后端为 `agree/reject` 添加前置状态校验 | 防止在错误 task 维度上自动创建 `fixed+agreed` 或 `open+rejected` 状态。 |
| 在 `reviewApi.test.ts` 锁定 adapter 血缘字段 | 该测试 seam 小且稳定，直接覆盖本次“后端返回 taskId 被前端丢弃”的根因。 |

## Issues Encountered

| Issue | Resolution |
|-------|------------|
| 后端编译验证被 `aws-lc-sys` 的 NASM 依赖阻塞 | 作为环境问题记录，后续安装 NASM 后重跑 `cargo check`。 |
| 根目录已有其他任务的 planning 文件 | 新计划文件放到 `docs/plans/`，避免覆盖既有 DuckLake 计划。 |
| `session-catchup.py` 不存在 | 记录到计划错误表，不阻断本次开发文件编写。 |
| Review 面板回归测试失败 33 项 | 失败集中在 `DesignerCommentHandlingPanel.test.ts` 全套和 `ReviewPanel.test.ts` 部分旧用例；本次新增的 `AnnotationTableView` 与 view mode bus 测试通过，需单独确认测试基线漂移原因。 |

## Resources

- `plant3d-web/src/types/auth.ts`
- `plant3d-web/src/api/reviewApi.ts`
- `plant3d-web/src/components/review/ReviewPanel.vue`
- `plant-model-gen/src/web_api/review_annotation_state.rs`
- `plant-model-gen/src/web_api/platform_api/annotation_check.rs`

## Visual/Browser Findings

- 本轮未执行浏览器可视化验证。
- 后续 UI 回归应使用 PMS 嵌入入口或本地模拟器复现同一 `form_id`、不同 `taskId` 的跨角色流程。
