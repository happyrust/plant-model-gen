# Platform API — HTTP 请求示例

面向 PMS 后端调用 `plant-model-gen`（`web_server`）时的手工/联调示例。默认假设服务在 `http://127.0.0.1:3100`（以 `DbOption.toml` 中 `[web_server]` 为准）。

> PMS 入站接口统一走 `[platform_auth]`：
> - `enabled = true` 时，`token` 必须是可通过后端验签的 JWT
> - `enabled = false` 时，`token` 必须与 `platform_auth.debug_token` 完全一致
> - `review_auth.enabled` 只影响浏览器侧 `/api/review/*`，不再决定 PMS S2S 是否放行

## 相关文档导航

- **Guides 总入口**：`docs/guides/README.md`
- **workflow/sync 按 form_id 返回 records 的完整测试模拟**：`docs/guides/WORKFLOW_SYNC_FORM_ID_TEST_SIMULATION.md`
- **reviewer 页面恢复 + Playwright 截图教程**：`docs/guides/WORKFLOW_SYNC_FORM_ID_PLAYWRIGHT_TUTORIAL.md`
- **PMS mock 展示层联调页**：`docs/guides/PMS_WORKFLOW_SYNC_MOCK_PAGE.md`

---

## 1. 嵌入地址 `POST /api/review/embed-url`

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/embed-url' \
  -H 'Content-Type: application/json' \
  -d '{
    "project_id": "2410",
    "user_id": "kangwp",
    "workflow_role": "sj",
    "form_id": "FORM-ABC123",
    "token": "<PMS 入站 S2S token；由 [platform_auth] 控制，必填>",
    "workflow_mode": "manual",
    "extra_parameters": { "is_reviewer": false }
  }'
```

`workflow_role`：本单据上为当前用户指定的工作流角色（`sj` / `jd` / `sh` / `pz` / `admin`）。为兼容旧客户端仍接受顶层 JSON 键 `role`；**不再接受** `user_role`。

当前正式协议下，返回的公开嵌入 `url` 应只包含：

```text
...?user_token=<jwt>
```

`form_id / project_id / user_id / output_project` 仍可出现在 `data` 调试字段里，但不再进入正式路由 URL。

---

## 2. 流程预校验 `POST /api/review/workflow/verify`

`verify` 与 `workflow/sync` 使用**完全相同**的请求体；推荐调用顺序固定为：

```text
verify -> sync
```

`verify` 只做预判，不写 `review_tasks / review_forms / review_workflow_history`，也不触发任何异步通知。

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/workflow/verify' \
  -H 'Content-Type: application/json' \
  -d '{
    "form_id": "FORM-ABC123",
    "token": "<PMS 入站 S2S token>",
    "action": "agree",
    "actor": { "id": "liubo", "name": "刘某", "roles": "jd" },
    "next_step": { "assignee_id": "wangsh", "name": "王某", "roles": "sh" },
    "comments": "校核通过"
  }'
```

典型响应（放行）：

```json
{
  "code": 200,
  "message": "ok",
  "data": {
    "passed": true,
    "action": "agree",
    "current_node": "jd",
    "task_status": "submitted",
    "next_step": "sh",
    "reason": "验证通过，可继续流转",
    "recommended_action": "proceed"
  }
}
```

典型响应（预校验拦截，但请求体合法）：

```json
{
  "code": 200,
  "message": "存在待确认批注，请逐条确认后再继续",
  "error_code": "ANNOTATION_CHECK_FAILED",
  "annotation_check": {
    "passed": false,
    "recommended_action": "block",
    "current_node": "jd"
  },
  "data": {
    "passed": false,
    "action": "agree",
    "current_node": "jd",
    "task_status": "submitted",
    "next_step": "sh",
    "reason": "存在待确认批注，请逐条确认后再继续",
    "recommended_action": "block"
  }
}
```

说明：

- `HTTP 200 + passed=false`：表示请求体合法，但当前不允许流转。
- `HTTP 400 / 404`：表示请求缺字段、目标节点非法、`form_id` 不存在等硬错误。
- `HTTP 401`：S2S token 不合法。

---

## 3. 校审流程同步（送审/审批）`POST /api/review/workflow/sync`

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/workflow/sync' \
  -H 'Content-Type: application/json' \
  -d '{
    "form_id": "FORM-ABC123",
    "token": "<PMS 入站 S2S token>",
    "action": "active",
    "actor": { "id": "kangwp", "name": "康某", "roles": "sj" },
    "next_step": { "assignee_id": "liubo", "name": "刘某", "roles": "jd" },
    "comments": "通过，请领导审核",
    "metadata": { "decision_at": "2025/09/23 13:56:23" }
  }'
```

`action` 取值：`query`（只读查询）、`active`、`agree`、`return`、`stop`。

正式接入顺序应为：

```text
workflow/verify -> workflow/sync
```

其中：

- `workflow/verify`：只校验，不落库；
- `workflow/sync`：真正写入并返回最新聚合快照；
- 即使调用方跳过 `verify`，`workflow/sync` 仍会做同一套强校验。

请求字段约束补充：

- `comments` 表示平台流程引擎的当前节点整体审批意见。
- 模型中心接收该字段用于 workflow 动作上下文，但**不会**在模型中心再次持久化，也**不会**在 `workflow/sync` 响应中回传。
- 第三方若需要保存流程意见，应继续以平台流程系统自身记录为准。

典型响应（`action=query` 成功时）：

```json
{
  "code": 200,
  "message": "success",
  "data": {
    "models": ["1001-PIPE", "1002-VALVE"],
    "task_id": "task-12345678",
    "records": [
      {
        "id": "review_records:abc",
        "task_id": "task-12345678",
        "type": "batch",
        "annotations": [
          {
            "id": "anno-text-1",
            "type": "text",
            "content": "请补充支吊架说明"
          }
        ],
        "cloud_annotations": [
          {
            "id": "anno-cloud-1",
            "type": "cloud"
          }
        ],
        "rect_annotations": [],
        "obb_annotations": [],
        "measurements": [],
        "note": "模型复核意见汇总",
        "confirmed_at": "2026-03-27 11:05:18"
      }
    ],
    "annotation_comments": [
      {
        "id": "review_comments:def",
        "annotation_id": "anno-text-1",
        "annotation_type": "text",
        "author_id": "kangwp",
        "author_name": "康某",
        "author_role": "sj",
        "content": "该处与土建提资不一致",
        "reply_to_id": null,
        "created_at": "2026-03-27 11:05:20"
      }
    ],
    "attachments": [
      {
        "model": ["1001-PIPE"],
        "id": "file-001",
        "type": "markup",
        "route_url": "/files/review_attachments/20260327110518_cloud.png",
        "download_url": "/files/review_attachments/20260327110518_cloud.png",
        "public_url": "http://127.0.0.1:3100/files/review_attachments/20260327110518_cloud.png",
        "description": "云线截图",
        "file_ext": "png"
      }
    ],
    "form_exists": true,
    "form_status": "active",
    "task_created": true,
    "current_node": "jd",
    "task_status": "reviewing"
  }
}
```

说明：

- `records`：模型侧批注主体，包含文本批注、云线、框选、OBB、测量、备注等结构化数据。
- `annotation_comments`：挂在具体批注下的评论串。
- `attachments.route_url`：统一返回可路由拼接的相对路径，第三方拿到域名后可自行拼接。
- `attachments.public_url`：后端按 `web_server.public_base_url` / `backend_url` 计算出的绝对 URL；若未配置则可能为空。
- 任一核心聚合查询报错时，接口直接返回非 200，`data = null`，不会再回“空成功”。

---

## 3. 缓存预加载 `POST /api/review/cache/preload`

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/cache/preload' \
  -H 'Content-Type: application/json' \
  -d '{
    "project_id": "2410",
    "initiator": "kangwp",
    "token": "<PMS 入站 S2S token>"
  }'
```

---

## 4. 删除校审数据 `POST /api/review/delete`

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/delete' \
  -H 'Content-Type: application/json' \
  -d '{
    "form_ids": ["FORM-ABC123", "FORM-456"],
    "operator_id": "kangwp",
    "token": "<PMS 入站 S2S token>"
  }'
```

典型成功响应：

```json
{
  "code": 200,
  "message": "ok",
  "results": [
    {
      "form_id": "FORM-ABC123",
      "success": true,
      "message": "已清理 review 主链"
    },
    {
      "form_id": "FORM-456",
      "success": true,
      "message": "已清理 review 主链"
    }
  ]
}
```

删除范围：

- 软删：`review_forms`、`review_tasks`
- 物理删除：`review_form_model`、`review_records`、`review_attachment`、`review_workflow_history`、附件物理文件
- **不会删除 `review_comments`**

---

## 5. 获取 JWT（联调）`POST /api/auth/token`

若已启用 `review_auth`，可先取 token 再填到上述请求：

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/auth/token' \
  -H 'Content-Type: application/json' \
  -d '{
    "username": "kangwp",
    "project": "2410",
    "role": "sj",
    "workflow_mode": "manual"
  }'
```

响应中的 `data.token` 与 `data.form_id` 用于后续 `embed-url` / `workflow/sync`。`workflow_mode` 会写入 JWT claims，并在 `/api/auth/verify` 返回。

---

## 代码位置（重构后）

- 路由注册：`src/web_api/platform_api/mod.rs` → `create_platform_api_routes()`
- 入站处理：`embed_url.rs`、`workflow_sync.rs`、`cache_preload.rs`、`delete_handler.rs`（PMS S2S 统一走 `platform_auth`）
- 主单据与任务查询：`review_form.rs`
