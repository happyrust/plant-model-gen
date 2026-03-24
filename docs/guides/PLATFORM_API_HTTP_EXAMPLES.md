# Platform API — HTTP 请求示例

面向 PMS 后端调用 `plant-model-gen`（`web_server`）时的手工/联调示例。默认假设服务在 `http://127.0.0.1:3100`（以 `DbOption.toml` 中 `[web_server]` 为准）。

> 当 `[review_auth].enabled = true` 时，下列接口中带 `token` 的需为有效 JWT（与 `编校审交互接口设计.md` 一致）。

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
- 入站处理：`embed_url.rs`、`workflow_sync.rs`、`cache_preload.rs`、`delete_handler.rs`
- 主单据与任务查询：`review_form.rs`
- 出站通知（可选）：`outbound_notify.rs` + `db_options/DbOption.toml` 的 `[external_review]`
