# JD/JH 处理 SJ 退回批注后续开发计划

## Goal

让 PMS 外部流程中同一 `form_id` 下的 SJ、JD/JH、SH/PZ 能围绕同一批注处理状态闭环：SJ 标记已修改/不需解决后，JD/JH 能稳定执行同意或驳回，后续工作流校验也能读取同一真源状态。

## Current Phase

Phase 2: 环境验证与接口回归

## Phases

### Phase 1: 根因修复基线

- [x] 保留后端 `AnnotationStateView.taskId` 到前端 `AnnotationReviewState`。
- [x] `ReviewPanel` 展开批注处理时优先使用批注状态的 canonical `taskId`。
- [x] 后端 `agree/reject` 写入前校验必须已有 SJ `fixed/wont_fix` 状态。
- **Status:** complete

### Phase 2: 环境验证与接口回归

- [ ] 安装或补齐 NASM，使 `aws-lc-sys` 能完成本机编译。
- [ ] 重新运行 `cargo check --bin web_server --features web_server`。
- [ ] 启动 `web_server`，用 HTTP JSON 调用验证 `/api/review/annotation-states/apply`。
- [ ] 覆盖三类接口场景：正确 taskId 可同意/驳回、错误 taskId 返回冲突、未处理批注返回冲突。
- **Status:** in_progress

### Phase 3: 前端交互回归

- [ ] 构造同一 `form_id`、不同内部 `taskId` 的 SJ/JD/JH 场景。
- [ ] 验证表格/卡片展示仍按 `form_id` 收敛，不跨单据展示批注。
- [ ] 验证 JD/JH 展开 SJ 处理过的批注后，按钮可选且提交使用 SJ 状态所在 `taskId`。
- [ ] 验证驳回原因必填、同意可选备注、成功后时间线刷新。
- **Status:** pending

### Phase 4: 工作流门禁回归

- [ ] 验证 `agree` 工作流推进时 annotation gate 读取到 agreed 状态。
- [ ] 验证存在 rejected 批注时 `agree` 被拦截并推荐 return。
- [ ] 验证 `return` 在存在 open/rejected 批注时允许驳回。
- **Status:** pending

### Phase 5: 文档与收敛

- [ ] 将 `form_id` 外部聚焦模式下的批注状态血缘规则写入三维校审开发文档。
- [ ] 将 NASM 环境依赖补充到本地后端验证说明或部署前置检查。
- [ ] 评估是否需要为 `AnnotationReviewState` 的 `taskId` 血缘新增单元测试。
- **Status:** pending

## Key Questions

1. 后端 `review_round` 在跨内部 task 的 PMS 外部流程中是否总能保持同一轮次语义？
2. JD 与 JH 是否都映射为同一个前端审核角色，还是需要在 UI 文案中区分校对/审核岗位？
3. 评论线程是否也应跟随 canonical `taskId`，还是只要求处理状态跟随？

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| 前端保留后端状态行的 `taskId` | 同一 `form_id` 可能恢复到不同内部任务；提交必须回到状态真源所在任务。 |
| 展开批注时优先使用 `item.reviewState.taskId` | 这是当前 UI 层能拿到的最接近后端真源的 canonical taskId。 |
| 后端拒绝无 SJ 处理状态的 `agree/reject` | 避免错误 task 维度静默新建确认状态，问题能尽早暴露。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `cargo check --bin web_server --features web_server` 失败：`NASM command not found` | 1 | 记录为环境阻塞；需安装 NASM 后重跑。 |
| `planning-with-files` 的 `session-catchup.py` 路径不存在 | 1 | 记录为非阻塞；继续手工创建计划文件。 |

## Notes

- 不运行 `cargo test`，遵守仓库规则。
- `web_server` 回归应使用启动服务 + HTTP POST/GET 验证，而不是测试编译。
