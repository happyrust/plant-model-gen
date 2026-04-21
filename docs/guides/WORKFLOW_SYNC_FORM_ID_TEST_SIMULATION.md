# workflow sync 按 form_id 返回校审数据：完整测试模拟文档

> 目标：验证 **校对 / 校核人员** 在校审面板中保存过的批注、云线、测量、备注等确认记录，在送审时通过 `POST /api/review/workflow/sync` 且仅提供 `form_id` 的情况下，能够被正确返回。

## 相关文档导航

- **Guides 总入口**：`docs/guides/README.md`
- **Playwright 教程 + 截图验收**：`docs/guides/WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md`
- **PMS mock 展示层联调页**：`docs/guides/PMS_WORKFLOW_SYNC_MOCK_PAGE.md`
- **Platform API HTTP 示例**：`docs/guides/PLATFORM_API_HTTP_EXAMPLES.md`

---

## 1. 测试结论

当前生产环境已经满足以下目标：

1. 校审面板点击“确认当前数据 / 保存”后，确认记录会落库到 `review_records`。
2. `review_records` 现行保存时会明确带上 `form_id`。
3. `workflow/sync` 查询 `records` 时，优先按 `form_id` 聚合。
4. 因此，送审阶段只要调用：
   - `POST /api/review/workflow/sync`
   - 请求体中携带 `form_id`

   就能够返回该单据下已经保存过的：
   - 文字批注 `annotations`
   - 云线 `cloud_annotations`
   - 矩形批注 `rect_annotations`
   - OBB 批注 `obb_annotations`
   - 测量 `measurements`
   - 备注 `note`

---

## 2. 适用范围

本文档适用于以下联调 / 验收场景：

- PMS / 流程平台侧验证 `workflow/sync` 返回的是否是 **单据 form_id 维度** 的校审数据
- 后端联调验证 `review_records` 是否真正按 `form_id` 落库
- 生产环境发布后做真实回归
- 提供给第三方或后续会话，作为固定测试脚本 / 手工核查依据

---

## 3. 相关接口与事实源

### 3.1 关键接口

| 接口 | 作用 |
|------|------|
| `POST /api/review/embed-url` | 获取或恢复 `form_id`，并签发 JWT |
| `POST /api/review/tasks` | 创建校审任务，并把任务与 `form_id` 绑定 |
| `POST /api/review/records` | 保存校审确认记录（批注 / 云线 / 测量 / 备注） |
| `POST /api/review/workflow/verify` | 对 `active / agree / return / stop` 做正式预校验，不写库 |
| `POST /api/review/workflow/sync` | 送审 / 查询时按 `form_id` 聚合返回 records 等数据 |
| `POST /api/review/delete` | 清理测试单据 |

### 3.2 关键数据表

| 表 | 用途 |
|----|------|
| `review_tasks` | 任务主表，记录 `task_id ↔ form_id` |
| `review_records` | 已确认批注 / 测量记录事实源 |
| `review_attachment` | 截图 / 文档附件 |
| `review_comments` | 批注评论 |

### 3.3 当前 records 事实源字段

`review_records` 当前关键字段为：

- `task_id`
- `form_id`
- `type`
- `annotations`
- `cloud_annotations`
- `rect_annotations`
- `obb_annotations`
- `measurements`
- `note`
- `confirmed_at`

---

## 4. 完整测试时序

```text
1. 平台 / 测试脚本调用 embed-url，得到 form_id + token
2. 使用 form_id 创建 review task
3. 模拟校对/校核人员在校审面板保存一批确认记录
   -> POST /api/review/records
4. 外部流程驱动先调用 workflow/verify
   -> POST /api/review/workflow/verify
   -> 只检查是否允许流转，不写 review_tasks / review_forms
5. 仅当 verify 通过时，再调用 workflow/sync
   -> POST /api/review/workflow/sync
6. 用 workflow/sync?action=query 回读 form_id 聚合快照
   -> POST /api/review/workflow/sync { form_id, token, action=query }
7. 断言返回 records 中包含：
   - annotations
   - cloud_annotations
   - measurements
   - note
8. 用 Surreal CLI + JSON 直查 review_records，验证 form_id 真实落库
9. 调用 /api/review/delete 清理测试数据
```

---

## 5. 推荐测试前置条件

### 5.1 本地环境

适用于本地联调：

- `web_server` 已启动
- 推荐地址：`http://127.0.0.1:3100`
- 数据库可用
- 不使用 Rust `test`
- 通过真实 HTTP POST/GET 验证
- PMS 入站接口统一按 `[platform_auth]` 校验
  - `platform_auth.enabled = true`：请求里的 `token` 必须是可验签 JWT
  - `platform_auth.enabled = false`：请求里的 `token` 必须与 `platform_auth.debug_token` 完全相等
- `review_auth.enabled = false` 只影响浏览器侧 `/api/review/*`，不会让 `embed-url / workflow/sync / delete` 自动放开

### 5.2 生产环境

适用于线上验证：

- 服务地址：`http://123.57.182.243`
- `/api/health` 应返回：

```json
{
  "database": "healthy",
  "status": "ok"
}
```

---

## 6. 手工模拟测试步骤

以下步骤同时适用于本地与生产；只需替换 `BASE_URL`。

### 6.1 第一步：生成 / 恢复 form_id

#### 请求

```http
POST /api/review/embed-url
Content-Type: application/json

{
  "project_id": "AvevaMarineSample",
  "user_id": "SJ",
  "form_id": "FORM-EXAMPLE-1234567890AB",
  "token": "<PMS 入站 S2S token>"
}
```

说明：

- 这里的 `token` 走 `[platform_auth]`
- 它不是浏览器侧 `Authorization: Bearer <JWT>`

#### 预期

返回：

- `data.token`
- `data.query.form_id`
- `data.lineage.form_id`

并断言：

- `data.query.form_id == 请求中的 form_id`

---

### 6.2 第二步：创建任务

#### 请求

```http
POST /api/review/tasks
Authorization: Bearer <token>
Content-Type: application/json

{
  "title": "workflow-sync-formid-test",
  "description": "验证 workflow sync 按 form_id 返回 records",
  "modelName": "AvevaMarineSample",
  "checkerId": "JH",
  "approverId": "SH",
  "reviewerId": "JH",
  "formId": "FORM-EXAMPLE-1234567890AB",
  "priority": "medium",
  "components": [
    {
      "id": "c1",
      "refNo": "24381_145018",
      "name": "管道A",
      "type": "PIPE"
    },
    {
      "id": "c2",
      "refNo": "24381_145020",
      "name": "阀门B",
      "type": "VALVE"
    }
  ]
}
```

#### 预期

返回：

- `task.id`
- `task.formId == FORM-EXAMPLE-1234567890AB`

---

### 6.3 第三步：模拟校审面板点击“确认当前数据”保存

这一阶段等价于 reviewer 在校审面板中完成：

- 文字批注
- 云线
- 测量
- 备注

然后点击保存。

#### 请求

```http
POST /api/review/records
Authorization: Bearer <token>
Content-Type: application/json

{
  "taskId": "<task_id>",
  "formId": "FORM-EXAMPLE-1234567890AB",
  "type": "batch",
  "annotations": [
    {
      "id": "anno-text-001",
      "type": "text",
      "content": "这里需要调整支吊架",
      "position": { "x": 1, "y": 2, "z": 3 }
    }
  ],
  "cloudAnnotations": [
    {
      "id": "anno-cloud-001",
      "type": "cloud",
      "shape": "ellipse",
      "points": [
        { "x": 0, "y": 0, "z": 0 },
        { "x": 1, "y": 1, "z": 1 }
      ]
    }
  ],
  "rectAnnotations": [],
  "obbAnnotations": [],
  "measurements": [
    {
      "id": "measure-001",
      "type": "distance",
      "value": 66.6,
      "unit": "mm"
    }
  ],
  "note": "workflow sync 应按 form_id 返回这批数据"
}
```

#### 预期

返回：

- `success = true`
- `record.id` 非空

并且后端会在 `review_records` 中写入：

- `task_id`
- `form_id`
- `annotations`
- `cloud_annotations`
- `measurements`
- `note`
- `confirmed_at`

---

### 6.4 第四步：送审时调用 workflow/sync

#### 请求

```http
POST /api/review/workflow/sync
Content-Type: application/json

{
  "form_id": "FORM-EXAMPLE-1234567890AB",
  "token": "<PMS 入站 S2S token>",
  "action": "query",
  "actor": {
    "id": "SJ",
    "name": "设计",
    "roles": "sj"
  }
}
```

#### 预期

返回结构中应包含：

- `data.models`
- `data.task_id`
- `data.records`
- `data.annotation_comments`
- `data.attachments`
- `data.form_exists`
- `data.form_status`
- `data.task_created`
- `data.current_node`
- `data.task_status`

支持动作集合固定为：

- `query`
- `active`
- `agree`
- `return`
- `stop`

其中 `data.records[0]` 应该包含：

- `annotations`
- `cloud_annotations`
- `measurements`
- `note`
- `confirmed_at`

#### 关键断言

至少断言：

```json
{
  "workflow_record_count": 1,
  "workflow_annotation_count": 1,
  "workflow_measurement_count": 1,
  "workflow_note": "workflow sync 应按 form_id 返回这批数据"
}
```

---

## 7. 数据库核查方式（CLI + JSON）

根据仓库要求，数据库验收应优先使用 **CLI + JSON**。

### 7.1 本地库核查示例

```bash
printf "SELECT form_id, task_id, note FROM review_records WHERE form_id = 'FORM-EXAMPLE-1234567890AB';\n" \
| surreal sql --json \
  --endpoint ws://127.0.0.1:8020 \
  --namespace 1516 \
  --database AvevaMarineSample \
  --username root \
  --password root
```

### 7.2 预期输出

```json
[[{
  "form_id": "FORM-EXAMPLE-1234567890AB",
  "note": "workflow sync 应按 form_id 返回这批数据",
  "task_id": "task-xxxxxxxx"
}]]
```

### 7.3 断言点

必须确认：

- `review_records.form_id` 已真实落库
- 不只是接口响应里“看起来像有关联”

---

## 8. 清理步骤

测试完成后，必须清理测试单据。

### 请求

```http
POST /api/review/delete
Content-Type: application/json

{
  "form_ids": ["FORM-EXAMPLE-1234567890AB"],
  "operator_id": "SJ",
  "token": "<token>"
}
```

### 预期

```json
{
  "code": 200,
  "message": "ok",
  "results": [
    {
      "form_id": "FORM-EXAMPLE-1234567890AB",
      "success": true,
      "message": "已清理 review 主链"
    }
  ]
}
```

删除范围说明：

- 会清 `review_form_model`
- 会清 `review_records`
- 会清 `review_attachment`
- 会清 `review_workflow_history`
- 会删除 `assets/review_attachments/*` 物理文件
- **不会清 `review_comments`**

---

## 9. 本次真实线上验收样本

以下是本次修复后的真实生产验收样本，便于后续比对。

### 9.1 生产环境

- `BASE_URL = http://123.57.182.243`
- 验收日期：`2026-03-28`
- 实际上线 commit：`799f62b6952eed9ba39922dabf669225615769c5`

### 9.2 本次线上测试单据

- `form_id = FORM-LIVE-5BE7A4EF5F39`
- `task_id = task-ca73dc87-c220-4232-96c4-de27bbe91e00`
- `record_id = record-73709ff1-3b3b-40dc-b457-d742473721fb`

### 9.3 线上 workflow/sync 验收摘要

```json
{
  "form_id": "FORM-LIVE-5BE7A4EF5F39",
  "task_id": "task-ca73dc87-c220-4232-96c4-de27bbe91e00",
  "record_id": "record-73709ff1-3b3b-40dc-b457-d742473721fb",
  "workflow_record_count": 1,
  "workflow_annotation_count": 1,
  "workflow_measurement_count": 1,
  "workflow_note": "线上 workflow sync 应按 form_id 返回 records"
}
```

### 9.4 线上数据库核查结果

```json
[[{
  "form_id":"FORM-LIVE-5BE7A4EF5F39",
  "note":"线上 workflow sync 应按 form_id 返回 records",
  "task_id":"task-ca73dc87-c220-4232-96c4-de27bbe91e00"
}]]
```

### 9.5 线上清理结果

```json
{
  "code": 200,
  "message": "ok",
  "results": [
    {
      "form_id": "FORM-LIVE-5BE7A4EF5F39",
      "success": true,
      "message": "已清理 review 主链"
    }
  ]
}
```

---

## 10. 常见误区

### 误区 1：以为 workflow/sync 的 records 只是按 task_id 查

现行实现不是单纯按 `task_id` 查。

当前逻辑是：

1. **先按 `form_id` 查 `review_records`**
2. 如果查不到，再 fallback 到旧的 `task_id` 兼容路径

所以对外语义已经是：

> `workflow/sync` 的 records 返回以 `form_id` 为主关联维度。

---

### 误区 2：以为点击画一条批注就会实时写库

不是。

当前 reviewer 的实际语义是：

- 编辑中的批注 / 云线 / 测量先存在前端临时态
- 只有点击“确认当前数据 / 保存”时
- 才会整批写入 `review_records`

所以当前记录模型是：

- **batch 确认记录**
- 不是逐操作实时流水

---

### 误区 3：以为附件和 comments 会跟 records 一起保存

不是。

当前持久化链路分为三类：

- `POST /api/review/records` → `review_records`
- `POST /api/review/comments` → `review_comments`
- `POST /api/review/attachments` → `review_attachment`

---

## 11. 推荐复用方式

### 11.1 手工接口联调

适合：
- 后端开发
- PMS 对接方
- 生产环境回归

按本文第 6 节逐步执行即可。

### 11.2 PMS 展示层联调页

如需直接生成本地可点击联调页，可使用：

```bash
BASE_URL=http://123.57.182.243 ./shells/run_pms_workflow_mock_page.sh
```

参考文档：

- `docs/guides/PMS_WORKFLOW_SYNC_MOCK_PAGE.md`

### 11.3 工作流端到端脚本

如需做完整工作流链路验证，可参考：

- `shells/test_workflow_e2e.sh`

但注意：
- 若本轮只验证 `form_id -> records` 返回语义，优先按本文档步骤执行，链路更短、更聚焦。

---

## 12. 验收清单

### 必须通过

- [ ] `embed-url` 返回目标 `form_id`
- [ ] `tasks` 创建成功，任务绑定同一 `form_id`
- [ ] `records` 保存成功
- [ ] `review_records` 实库可查到 `form_id`
- [ ] `workflow/sync(form_id)` 返回 `records`
- [ ] 返回的 `records[0].annotations` 数量正确
- [ ] 返回的 `records[0].measurements` 数量正确
- [ ] 返回的 `records[0].note` 与保存时一致
- [ ] `/api/review/delete` 清理成功

### 建议通过

- [ ] `/api/version` 的 `commit` 已是目标上线 commit
- [ ] `/api/health.status = ok`
- [ ] 若带附件 / 评论，也一并检查 `annotation_comments` / `attachments`

---

## 13. 相关文件索引

| 路径 | 作用 |
|------|------|
| `src/web_api/review_api.rs` | 保存确认记录到 `review_records` |
| `src/web_api/platform_api/workflow_sync.rs` | workflow/sync 聚合逻辑 |
| `docs/development/三维校审/form_id数据关联分析.md` | form_id 关联设计说明 |
| `docs/guides/PMS_WORKFLOW_SYNC_MOCK_PAGE.md` | PMS 模拟联调页说明 |
| `shells/run_pms_workflow_mock_page.sh` | 生成 PMS mock 页 |
| `shells/test_workflow_e2e.sh` | 工作流端到端脚本 |

---

## 14. 最终结论

当前系统已经具备以下能力：

> **校对 / 校核人员在校审面板中确认保存过的批注、云线、测量、备注等数据，会以确认记录的形式落库到 `review_records`，并显式带上 `form_id`；送审时调用 `workflow/sync(form_id)`，后端会优先按该 `form_id` 返回这些 records 数据。**
