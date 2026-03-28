# form_id 数据关联分析

> 分析 `POST /api/review/embed-url` 返回的 `form_id` 在三维校审流程中的数据关联能力

## 1. form_id 的生成与确定

`form_id` 在 `POST /api/review/embed-url` 调用时确定，有三种来源（按优先级）：

| 优先级 | 来源 | 说明 |
|--------|------|------|
| 1 | `EmbedUrlRequest.form_id` | 外部系统直传，用于恢复已有单据 |
| 2 | JWT token claims 中的 `form_id` | 解码传入的 token 获取 |
| 3 | `generate_form_id()` 自动生成 | 格式 `FORM-XXXXXXXXXXXX`（12 位大写十六进制） |

确定后，`form_id` 会：
- 通过 `ensure_review_form_stub` 写入 `review_forms` 表
- 嵌入新签发的 JWT payload
- 作为 `data.query.form_id` 和 `data.lineage.form_id` 返回给调用方

## 2. embed-url 响应结构

```json
{
    "code": 200,
    "message": "ok",
    "url": "http://{host}/review/3d-view?user_token={jwt}&form_id={form_id}&user_id={uid}&project_id={pid}&output_project={pid}",
    "data": {
        "relative_path": "/review/3d-view",
        "token": "eyJhbGci...",
        "query": {
            "form_id": "FORM-ABC123DEF456",
            "is_reviewer": false
        },
        "lineage": {
            "form_id": "FORM-ABC123DEF456",
            "task_id": null,
            "current_node": null,
            "status": null
        },
        "form": {
            "form_id": "FORM-ABC123DEF456",
            "exists": true,
            "status": null,
            "task_created": false
        },
        "task": null
    }
}
```

## 3. form_id 关联的数据表

### 3.1 数据表总览

```
                     ┌──────────────────────┐
                     │    embed-url 调用     │
                     │  确定/生成 form_id    │
                     └──────────┬───────────┘
                                │
                     form_id (FORM-XXXXXXXXXXXX)
                         核心 Hub Key
                                │
          ┌─────────┬───────────┼───────────┬──────────────┐
          │         │           │           │              │
          ▼         ▼           ▼           ▼           ▼              ▼
   review_forms  review_tasks  review_    review_   review_       review_
   (单据主表)    (提资单/任务)  records    form_model opinion     attachment
                               (批注/测量) (模型关联) (审批意见)  (附件/云线)
```

### 3.2 各表字段说明

#### review_forms — 校审表单主表

| 字段 | 类型 | 说明 |
|------|------|------|
| `form_id` | string (UNIQUE) | 单据唯一标识 |
| `project_id` | string | 项目号 |
| `user_id` | string | 创建人 |
| `status` | option\<string\> | 单据状态 |
| `task_created` | option\<bool\> | 是否已创建提资单 |
| `deleted` | option\<bool\> | 软删除标记 |
| `created_at` | datetime | 创建时间 |

#### review_tasks — 提资单（工作流载体）

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | 任务 ID（`task-{uuid}`） |
| `form_id` | string | 关联的单据 form_id |
| `title` | string | 提资单标题 |
| `model_name` | string | 模型名称 |
| `status` | string | 状态（draft/submitted/reviewing/approved/rejected/returned） |
| `current_node` | string | 当前审批节点（sj/jd/sh/pz） |
| `requester_id/name` | string | 编制人（sj 节点） |
| `checker_id/name` | string | 校对人（jd 节点） |
| `approver_id/name` | string | 审核人（sh/pz 节点） |
| `components` | array | 组件清单 |
| `attachments` | array | 附件清单 |

#### review_form_model — 模型关联表

| 字段 | 类型 | 说明 |
|------|------|------|
| `form_id` | string | 关联的单据 form_id |
| `model_refno` | string | 模型参考号 |

#### review_records — 已确认批注/测量记录

| 字段 | 类型 | 说明 |
|------|------|------|
| `task_id` | string | 任务 ID（兼容历史查询） |
| `form_id` | string | 关联的单据 form_id；现行保存确认记录时会一并落库 |
| `type` | string | 当前通常为 `batch` |
| `annotations` | array | 文本批注等结构化批注 |
| `cloud_annotations` | array | 云线批注 |
| `rect_annotations` | array | 矩形批注 |
| `obb_annotations` | array | OBB 批注 |
| `measurements` | array | 测量数据 |
| `note` | string | 确认备注 |
| `confirmed_at` | datetime | 确认时间 |

#### review_opinion — 审批意见表

| 字段 | 类型 | 说明 |
|------|------|------|
| `form_id` | string | 关联的单据 form_id |
| `model_refnos` | array\<string\> | 关联的模型列表 |
| `node` | string | 审批节点（sj/jd/sh/pz） |
| `seq_order` | int | 意见顺序 |
| `author` | string | 审批人 |
| `opinion` | string | 历史兼容字段；现行 `workflow/sync` 不再把平台 `comments` 写入这里，也不再作为对外回传事实源 |

#### review_attachment — 附件表

| 字段 | 类型 | 说明 |
|------|------|------|
| `form_id` | string | 关联的单据 form_id |
| `model_refnos` | array\<string\> | 关联的模型列表 |
| `file_id` | string | 文件 ID |
| `file_type` | string | 类型（markup: 云线, file: 文件）；上传 `/api/review/attachments` 时兼容 `type` / `fileType` / `file_type` 三种字段名 |
| `download_url` | string | 下载地址 |
| `description` | string | 文件描述 |
| `file_ext` | string | 文件后缀 |

#### review_workflow_history — 工作流历史（间接关联）

| 字段 | 类型 | 说明 |
|------|------|------|
| `task_id` | string | 通过 review_tasks.form_id 间接关联 |
| `node` | string | 审批节点 |
| `action` | string | 操作（submit/return/approve/reject） |
| `operator_id/name` | string | 操作人 |
| `comment` | option\<string\> | 操作意见 |

## 4. form_id 在接口间的流转

### 4.1 创建阶段

```
平台 ──POST /api/review/embed-url──► 模型中心
     { project_id, user_id, token? }
                                     │
                              生成/确认 form_id
                              写入 review_forms
                                     │
平台 ◄───── 返回 form_id + token ────┘
     拼接嵌入 URL，iframe 加载校审页面
```

### 4.2 编制阶段

```
前端 ──POST /api/review/tasks──► 模型中心
     { title, model_name, form_id, ... }
                                  │
                           创建 review_tasks
                           关联 review_form_model
                           更新 review_forms.task_created = true
```

### 4.3 审批流转阶段

```
平台 ──POST /api/review/workflow/sync──► 模型中心
     { form_id, action, actor, next_step, ... }
                                          │
                                   推进 review_tasks 状态
                                   记录 review_workflow_history
                                   按 form_id 聚合返回 models + records + annotation_comments + attachments(route_url/public_url)
```

### 4.4 辅助校审数据查询

```
模型中心 ──POST /api/review/aux-data──► 平台
         { project_id, model_refnos, form_id, ... }
                                         │
                                  返回碰撞/质量/规则/二三维校验数据
```

### 4.5 删除同步

```
平台 ──POST /api/review/delete──► 模型中心
     { form_ids: [...], operator_id, token }
                                     │
                              删除 review_forms
                              删除 review_tasks
                              删除 review_form_model
                              删除 review_opinion
                              删除 review_attachment
                              删除附件文件
```

## 5. 结论

**`form_id` 已完全具备三维校审数据关联能力。** 它被设计为贯穿整个编校审流程的稳定业务主键：

- **唯一性**：`review_forms` 表上有 UNIQUE 索引
- **稳定性**：一旦创建不会变化，跨 open/save/submit/read 保持一致
- **全覆盖**：直接关联 5 张核心表，间接关联 workflow_history
- **跨系统**：JWT 中携带、URL 参数传递、S2S 接口流转均使用同一 `form_id`

### 注意事项

1. `review_workflow_history` 使用 `task_id` 而非直接使用 `form_id`，查询历史需先通过 `review_tasks.form_id` 获取 `task_id`
2. `.surql` 定义文件中表名为单数（`review_form`），运行时代码使用复数（`review_forms`），以 Rust 代码 `ensure_review_forms_schema()` 为准
3. 遗留的 ArangoDB `threed_review` / `review_data` 集合与新 SurrealDB 系统独立，互不关联

## 6. 源码参考

| 文件 | 内容 |
|------|------|
| `src/web_api/platform_api/` | embed-url、form stub、workflow sync/delete（原 `model_center_client.rs` 已拆分为本模块） |
| `src/web_api/review_api.rs` | 校审 CRUD API、任务工作流 |
| `src/web_api/jwt_auth.rs` | JWT 生成/验证、form_id 生成 |
| `src/web_api/review_integration.rs` | 辅助校审数据接口 |
| `rs_surreal/review/review_tables.surql` | 表结构定义（参考） |
