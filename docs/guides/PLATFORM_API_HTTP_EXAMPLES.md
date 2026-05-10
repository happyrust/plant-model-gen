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

`verify` 与 `workflow/sync` 共用 [`SyncWorkflowRequest`] 类型，但**消费的字段完全不同**：

| 字段 | verify | sync |
|---|---|---|
| `form_id` / `token` / `action` | 必读 | 必读 |
| `actor` | 仅 debug_token 模式必填；其他场景 handler 自动从 token claims 推 | 同 verify |
| `next_step` | **静默忽略**——传也不会被读 | active/agree(非pz)/return 必填；stop/agree(pz) 可省 |
| `comments` | 静默忽略 | 落 `review_workflow_history.comment` |
| `metadata` | 静默忽略（保留兼容） | 静默忽略 |

也就是说，**生产链路下 verify 的最小请求体只需要 `form_id + token + action` 三个字段**。

推荐调用顺序：

```text
verify -> sync
```

`verify` 只做预判，不写 `review_tasks / review_forms / review_workflow_history`，也不触发任何异步通知。

### 2.1 检查矩阵（按 action）

| action | 允许的 current_node | annotation 要求 | 不满足时 recommended_action |
|---|---|---|---|
| `active` | 仅 `sj` | 所有批注都被回复（`open == 0`） | `block` |
| `agree` | `jd` / `sh` / `pz` | `open == 0 && rejected == 0 && pending == 0` | `return`（有 rejected）/ `block`（仅 pending） |
| `return` | `jd` / `sh` / `pz` | 至少 1 条 `open` 或 `rejected`（"有问题才能驳回"） | `block`（"无问题批注，不允许驳回"） |
| `stop` | `jd` / `sh` / `pz` | 不做 annotation_check | — |

任何业务规则触发的阻断都走「软阻断」：`HTTP 200 + passed=false + 结构化诊断`。

### 2.2 最小请求体示例

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/workflow/verify' \
  -H 'Content-Type: application/json' \
  -d '{
    "form_id": "FORM-ABC123",
    "token": "<PMS 入站 S2S token>",
    "action": "agree"
  }'
```

兼容历史调用方携带 `actor` / `next_step` 也无错误，handler 会按表 2.1 静默忽略 `next_step`。

### 2.3 典型响应（放行）

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

### 2.4 典型响应（软阻断·有 rejected 批注）

```json
{
  "code": 200,
  "message": "存在已驳回批注，请改走驳回流程",
  "error_code": "ANNOTATION_CHECK_FAILED",
  "annotation_check": {
    "passed": false,
    "recommended_action": "return",
    "current_node": "jd"
  },
  "data": {
    "passed": false,
    "action": "agree",
    "block_code": "ANNOTATION_CHECK_FAILED",
    "current_node": "jd",
    "task_status": "submitted",
    "next_step": "sh",
    "reason": "存在已驳回批注，请改走驳回流程",
    "recommended_action": "return"
  }
}
```

### 2.5 典型响应（软阻断·无问题批注却要 return）

```json
{
  "code": 200,
  "message": "无未处理或被驳回的批注，不允许驳回",
  "error_code": "ANNOTATION_CHECK_FAILED",
  "annotation_check": {
    "passed": false,
    "recommended_action": "block",
    "current_node": "jd"
  },
  "data": {
    "passed": false,
    "action": "return",
    "block_code": "ANNOTATION_CHECK_FAILED",
    "current_node": "jd",
    "task_status": "submitted",
    "reason": "无未处理或被驳回的批注，不允许驳回",
    "recommended_action": "block"
  }
}
```

### 2.6 状态码语义

- `HTTP 200 + passed=false`：业务规则阻断（节点不匹配 / 批注门未通过 / owner 不一致 / 终态等）
- `HTTP 400`：action 不可识别、解析失败等格式错
- `HTTP 404`：`form_id` 没有对应单据
- `HTTP 401`：token 不合法
- `HTTP 500`：DB 异常

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

- `workflow/verify`：只校验，不落库；接口形态见 §2
- `workflow/sync`：真正写入并返回最新聚合快照
- 即使调用方跳过 `verify`，`workflow/sync` 仍会做同一套强校验，包括 §2.1 的 annotation 矩阵

请求字段约束补充：

- `comments`：平台流程引擎当前节点的整体审批意见，写入 `review_workflow_history.comment`，但**不会**在 `workflow/sync` 响应中回传。
- `next_step`：sync 路径下 `active` / `agree(非 pz)` / `return` 必填；`stop` 与 `agree(pz)` 可省。verify 路径忽略此字段，详见 §2。
- `metadata`：当前未被任何代码读取，保留兼容。

annotation 检查在 sync 路径与 verify 完全一致：

- `active`：sj 节点 `open == 0`
- `agree`：jd/sh/pz 节点 `open == 0 && rejected == 0 && pending == 0`
- `return`：jd/sh/pz 节点至少 1 条 `open` 或 `rejected`（"无问题批注，不允许驳回"）
- `stop`：不做 annotation_check

不满足时返 `HTTP 409 + error_code = "ANNOTATION_CHECK_FAILED"` + `annotation_check` 诊断字段。

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
