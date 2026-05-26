# 三维校审外部流程 PRD

> 当前版本：2026-05-14  
> 状态：当前实现已默认 external，本文用于统一产品原则、接口契约、操作流程和后续验收口径。  
> 相关架构文档：`docs/架构文档/三维校审外部流程架构说明.md`

## 1. 背景

三维校审最初同时存在两类流程能力：

- Plant3D 内部任务流转：前端按钮直接调用 `/api/review/tasks/{id}/submit|return|approve`。
- PMS 外部流程驱动：PMS 作为流程平台，通过 `embed-url`、`workflow/verify`、`workflow/sync` 驱动单据流转。

当前产品方向已经明确：**三维校审默认由 PMS 外部流程驱动**。Plant3D 不再决定流程下一步，只负责三维校审数据的采集、保存、展示、预校验和状态镜像。

## 2. 产品目标

1. PMS 能把三维校审作为 iframe 嵌入，并以 `form_id` 作为跨系统单据主键。
2. 设计、校核、审核、批准各角色能在 Plant3D 中查看同一 `form_id` 下的数据快照。
3. 批注、云线、矩形、OBB、测量、附件、备注等校审数据能可靠保存并被 `workflow/sync` 聚合返回。
4. PMS 可以在流程推进前调用 `workflow/verify` 获取软阻断诊断。
5. PMS 可以调用 `workflow/sync` 推进 `active/agree/return/stop`，Plant3D 写入镜像状态与历史。
6. 驳回 `return` 不再检查批注状态；是否允许驳回由 PMS 流程与目标节点决定，Plant3D 只校验节点合法性。

## 3. 非目标

- 不由 Plant3D 前端推导下一节点或办理人。
- 不在 Plant3D 内部按钮中完成 PMS 流程审批。
- 不把 `targetNode` 当成可补全 `nextStep` 的事实源。
- 不把批注状态作为 `return` 的阻断条件。
- 不用 `workflow/verify` 写入任何任务或单据状态。

## 4. 核心用户与场景

| 用户 | 目标 | 入口 |
|---|---|---|
| 设计人员 `sj` | 创建/补齐编校审数据，处理退回批注 | PMS iframe 打开 Plant3D |
| 校核人员 `jd` | 查看三维数据，保存问题批注或通过 | PMS iframe 打开 Plant3D |
| 审核人员 `sh` | 复核校核结果并继续/退回 | PMS iframe 打开 Plant3D |
| 批准人员 `pz` | 最终批准或退回 | PMS iframe 打开 Plant3D |
| PMS 平台 | 控制流程状态与办理人 | HTTP API + postMessage |

## 5. 用户流程

### 5.1 新建或补齐编校审数据

1. PMS 调 `POST /api/review/embed-url` 获取 iframe URL。
2. Plant3D 进入 external 模式。
3. SJ 选择模型构件、填写包名、上传附件。
4. SJ 点击“保存编校审单数据”。
5. Plant3D 创建或更新内部 `review_tasks`，绑定 `form_id`。
6. Plant3D 发送 `plant3d.form_saved` 给 PMS。
7. PMS 决定是否进入后续流程，并负责调用 `workflow/verify` / `workflow/sync`。

### 5.2 外部送审 active

1. PMS 发送 `pms.workflow_pre_action`：

```json
{
  "type": "pms.workflow_pre_action",
  "formId": "FORM-xxx",
  "action": "active",
  "requestId": "req-1"
}
```

2. Plant3D 自动保存当前未保存的校审数据。
3. Plant3D 调 `workflow/verify`。
4. PMS 收到 `plant3d.workflow_pre_action_acked`。
5. PMS 若决定推进，发送 `pms.workflow_changed`，必须包含 `nextStep`：

```json
{
  "type": "pms.workflow_changed",
  "formId": "FORM-xxx",
  "action": "active",
  "nextStep": { "assigneeId": "JH", "name": "校核员", "roles": "jd" },
  "requestId": "req-2"
}
```

6. Plant3D 调 `workflow/sync` 写入镜像状态并返回快照。

### 5.3 外部同意 agree

1. PMS 判断当前节点与下一节点。
2. PMS 发送 `workflow_pre_action(agree)` 进行预校验。
3. Plant3D 检查节点合法性与通过前批注门禁。
4. PMS 发送 `workflow_changed(agree)`，必须显式包含下一节点 `nextStep`，除非当前已经是 `pz` 最终批准。
5. Plant3D 写入 `review_tasks/review_forms/review_workflow_history` 并回传快照。

### 5.4 外部驳回 return

1. 校核/审核/批准人员在 Plant3D 中保存问题批注或说明。
2. PMS 发起驳回前预校验：

```json
{
  "type": "pms.workflow_pre_action",
  "formId": "FORM-xxx",
  "action": "return"
}
```

3. Plant3D 自动保存未保存数据。
4. `workflow/verify(return)` 不检查批注状态，只检查 action 和当前节点基本合法性。
5. PMS 发送 `workflow_changed(return)`，必须包含目标前序节点与办理人：

```json
{
  "type": "pms.workflow_changed",
  "formId": "FORM-xxx",
  "action": "return",
  "nextStep": { "assigneeId": "SJ", "name": "设计人员", "roles": "sj" },
  "comments": "请按批注修改",
  "requestId": "req-return-1"
}
```

6. Plant3D 调 `workflow/sync(return)`：
   - 校验当前节点必须是 `jd/sh/pz`。
   - 校验目标节点必须位于当前节点之前。
   - 写入 `return_reason`。
   - 写入 workflow history。
   - 返回包含 records/comments/attachments/currentNode/taskStatus 的快照。

## 6. 功能需求

### FR-1：外部模式默认启用

- 未显式启用内部模式 feature 时，前端和后端均默认 external。
- external 模式下设计侧按钮文案为“保存编校审单数据”。
- external 模式下内部 submit/return 按钮不作为主流程入口。

### FR-2：form_id 作为主线索

- 所有新建任务、确认记录、附件、评论、历史必须绑定或可回溯到 `form_id`。
- `workflow/sync?action=query` 必须能按 `form_id` 聚合返回快照。

### FR-3：nextStep 必须外部传入

- 对会改变流程的 external 动作，PMS 必须传 `nextStep`。
- Plant3D 前端不得用本地 `currentTask` 或 `targetNode` 推导 `nextStep`。
- 缺失 `nextStep` 时返回明确错误：`missing_external_next_step`。

### FR-4：return 不做批注状态门禁

- `verify(return)` 不检查 `open/rejected/pending/approved` 数量。
- `sync(return)` 不检查 `open/rejected/pending/approved` 数量。
- return 仍需满足当前节点和目标节点合法性。

### FR-5：pre_action 自动保存

- 收到 `pms.workflow_pre_action` 时，Plant3D 应先保存未保存的批注/测量。
- 保存失败时返回 `saveOk=false`，PMS 不应继续推进。

### FR-6：workflow/sync 后刷新本地快照

- `workflow/sync` 成功后，iframe 内 `currentTask.currentNode/status/returnReason` 必须与后端返回一致。
- 已保存记录、历史、附件的显示应与 `form_id` 最新快照一致。

### FR-7：postMessage 来源约束

- Plant3D 应只接受可信 PMS origin 的 workflow message。
- 未配置可信来源时，生产环境应拒绝 workflow mutation 或至少记录风险日志。
- ack/synced 响应应回发到 `event.origin`。

## 7. 接口需求

### 7.1 `POST /api/review/workflow/verify`

最小请求：

```json
{
  "form_id": "FORM-xxx",
  "token": "<token>",
  "action": "agree"
}
```

响应：

```json
{
  "code": 200,
  "message": "ok",
  "data": {
    "passed": true,
    "action": "agree",
    "current_node": "jd",
    "task_status": "submitted",
    "reason": "验证通过，可继续流转",
    "recommended_action": "proceed"
  }
}
```

### 7.2 `POST /api/review/workflow/sync`

流程 mutation 请求：

```json
{
  "form_id": "FORM-xxx",
  "token": "<token>",
  "action": "agree",
  "actor": { "id": "JH", "name": "校核员", "roles": "jd" },
  "next_step": { "assignee_id": "SH", "name": "审核员", "roles": "sh" },
  "comments": "校核通过",
  "metadata": { "source": "pms.workflow_changed" }
}
```

query 请求：

```json
{
  "form_id": "FORM-xxx",
  "token": "<token>",
  "action": "query"
}
```

响应必须包含：

- `taskId`
- `records`
- `annotationComments`
- `attachments`
- `annotationStates`
- `currentNode`
- `taskStatus`
- `nextStepDetail`

## 8. 验收标准

| 编号 | 验收项 | 通过标准 |
|---|---|---|
| AC-1 | external 默认模式 | 未配置内部模式时，前端展示“保存编校审单数据”，不展示内部流转主按钮 |
| AC-2 | nextStep 外部传入 | external `workflow_changed(active/agree/return)` 缺少 `nextStep` 时失败，错误可诊断 |
| AC-3 | return 不查批注 | 无 open/rejected 批注时，`verify(return)` 和 `sync(return)` 不因 annotation_check 阻断 |
| AC-4 | 自动保存 | `pre_action` 前有未保存批注时，会先写入 `review_records` |
| AC-5 | 状态刷新 | `sync(return)` 后 iframe 本地显示 `currentNode=sj` 且可见退回原因 |
| AC-6 | form_id 回读 | `sync(query)` 能返回同一 form_id 下 records/comments/attachments |
| AC-7 | 来源安全 | 非可信 origin 的 workflow message 不触发 verify/sync |

## 9. 风险与开放问题

| 风险 | 说明 | 建议 |
|---|---|---|
| external 权限边界 | 当前代码对 external owner 校验较宽松 | 明确是否完全信任 PMS；若否，绑定 token claims 与 actor |
| postMessage 来源 | 当前 bridge 可在未配置 trustedOrigins 时接受消息 | 引入 PMS origin 配置 |
| 旧文档漂移 | 旧文档仍有 return annotation_check 说法 | 更新 breaking notice 与联调文档 |
| 本地状态滞后 | sync 后未刷新 currentTask 会影响后续操作 | 将 sync 响应落回 reviewStore |

## 10. 后续版本建议

1. 在 `embed-url` token claims 中加入可信 PMS origin。
2. 在 `workflow/sync` mutation 中统一落库 `actor_id/actor_role/source/request_id`。
3. 为 PMS 提供固定 HTTP 示例与错误码字典。
4. 增加真实 HTTP 验证脚本：`active -> return -> active -> agree -> agree -> agree`。
5. 将旧内部模式保留为 debug/feature flag，不作为默认产品路径。
