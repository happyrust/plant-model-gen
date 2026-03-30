# Platform API — HTTP 请求示例

面向 PMS 后端调用 `plant-model-gen`（`web_server`）时的手工/联调示例。默认假设服务在 `http://127.0.0.1:3100`（以 `DbOption.toml` 中 `[web_server]` 为准）。

> 当 `[review_auth].enabled = true` 时，下列接口中带 `token` 的需为有效 JWT（与 `编校审交互接口设计.md` 一致）。

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
    "form_id": "FORM-ABC123",
    "token": "<平台签发的 JWT，可选；三段式 JWT 时须与 form_id 一致>",
    "extra_parameters": { "is_reviewer": false }
  }'
```

---

## 2. 校审流程同步（送审/审批）`POST /api/review/workflow/sync`

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/workflow/sync' \
  -H 'Content-Type: application/json' \
  -d '{
    "form_id": "FORM-ABC123",
    "token": "<JWT，与 form_id 一致>",
    "action": "active",
    "actor": { "id": "kangwp", "name": "康某", "roles": "sj" },
    "next_step": { "assignee_id": "liubo", "name": "刘某", "roles": "jd" },
    "comments": "通过，请领导审核",
    "metadata": { "decision_at": "2025/09/23 13:56:23" }
  }'
```

`action` 取值：`query`（只读查询）、`active`、`agree`、`return`、`stop`。

请求字段约束补充：

- `comments` 表示平台流程引擎的当前节点整体审批意见。
- 模型中心接收该字段用于 workflow 动作上下文，但**不会**在模型中心再次持久化，也**不会**在 `workflow/sync` 响应中回传。
- 第三方若需要保存流程意见，应继续以平台流程系统自身记录为准。

典型响应（已按“模型批注包 + 路由 URL”语义对齐）：

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

---

## 3. 缓存预加载 `POST /api/review/cache/preload`

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/review/cache/preload' \
  -H 'Content-Type: application/json' \
  -d '{
    "project_id": "2410",
    "initiator": "kangwp",
    "token": "<JWT>"
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
    "token": "<JWT>"
  }'
```

---

## 5. 获取 JWT（联调）`POST /api/auth/token`

若已启用 `review_auth`，可先取 token 再填到上述请求：

```bash
curl -sS -X POST 'http://127.0.0.1:3100/api/auth/token' \
  -H 'Content-Type: application/json' \
  -d '{
    "username": "kangwp",
    "project": "2410",
    "role": "sj"
  }'
```

响应中的 `data.token` 与 `data.form_id` 用于后续 `embed-url` / `workflow/sync`。

---

## 代码位置（重构后）

- 路由注册：`src/web_api/platform_api/mod.rs` → `create_platform_api_routes()`
- 入站处理：`embed_url.rs`、`workflow_sync.rs`、`cache_preload.rs`、`delete_handler.rs`（删除为软删，无出站回调）
- 主单据与任务查询：`review_form.rs`
