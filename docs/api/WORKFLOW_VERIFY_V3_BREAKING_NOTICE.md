# PMS 接入方变更通知：`/api/review/workflow/verify` v3 重构

> 发布版本：v3 · 2026-05-10
>
> 影响接口：`POST /api/review/workflow/verify`、`POST /api/review/workflow/sync`
>
> 关联仓库 commit：`c4d7cb53` ~ `8a9581e3` on `main`
>
> 详细 HTTP 示例：`docs/guides/PLATFORM_API_HTTP_EXAMPLES.md`

---

## TL;DR

| 关注点 | 旧契约（v2 及之前） | 新契约（v3） |
|---|---|---|
| **verify 必填字段** | `form_id` + `token` + `action` + `next_step`（active/agree/return） | **`form_id` + `token` + `action`** |
| **verify 是否读 `next_step`** | 是 | **否，静默忽略** |
| **verify 是否读 `target_node`** | 注释承诺读，但代码 0 引用（事实上不读） | **字段已删除** |
| **annotation 检查策略** | 4 个 action 共用一套 `submit_next` 规则 | **按 action 4 套独立规则** |
| **sync return 是否查 annotation** | 否 | **否**（2026-05-14 起恢复为不查；驳回由 PMS 流程与 `next_step` 决定） |

**接入方需要做什么：**

1. **不必改**：旧请求体如果继续传 `next_step` / `actor` / `comments` 等，verify 路径会**静默忽略**，行为不会出错。
2. **可以减字段**：从此 verify 调用最小只需 `form_id + token + action`。
3. **必须留意 sync mutation 的 `next_step`**：`active / agree(非 pz) / return` 的下一节点与办理人必须由 PMS 显式传入；Plant3D 不应本地推导。

---

## 1. 接入方典型变更前后对比

### 1.1 verify 调用

**旧（仍可工作）：**

```http
POST /api/review/workflow/verify
Content-Type: application/json

{
  "form_id": "FORM-ABC123",
  "token": "<S2S token>",
  "action": "agree",
  "actor": { "id": "JH", "name": "校核员", "roles": "jd" },
  "next_step": { "assignee_id": "SH", "name": "审核员", "roles": "sh" },
  "comments": "校核通过"
}
```

**新（推荐最小契约）：**

```http
POST /api/review/workflow/verify
Content-Type: application/json

{
  "form_id": "FORM-ABC123",
  "token": "<S2S token>",
  "action": "agree"
}
```

后端从 token 的 JWT claims 自动推导操作人；`next_step` / `comments` / `actor` 在 verify 路径完全不读，传与不传结果一致。

### 1.2 sync 调用（基本不变）

`sync` 仍然要求 `next_step`（active/agree(非 pz)/return）。当前 `return` 路径**不做 annotation_check 强校验**，只校验当前节点、目标节点和终态等流程条件：

```http
POST /api/review/workflow/sync
{
  "form_id": "FORM-ABC123",
  "token": "...",
  "action": "return",
  "actor": { "id": "JH", "name": "校核员", "roles": "jd" },
  "next_step": { "assignee_id": "SJ", "name": "送审者", "roles": "sj" }
}
```

即使当前任务下没有 `open / rejected` 批注，`return` 也不因批注状态阻断。驳回原因应由 PMS 的 `comments` 和业务流程承载。

---

## 2. annotation_check 矩阵（v3 起按 action 4 套）

| action | 节点合法性 | 批注门要求 | 不满足时 |
|---|---|---|---|
| `active` | 仅 `sj` | 所有批注都被回复（`open == 0`） | 200 + passed=false + `recommended_action="block"` |
| `agree` | `jd` / `sh` / `pz` | `open == 0 && rejected == 0 && pending == 0` | 200 + passed=false + `recommended_action="return"`（有 rejected）或 `"block"`（仅 pending） |
| `return` | `jd` / `sh` / `pz` | 不查 annotation_check | — |
| `stop` | `jd` / `sh` / `pz` | 不查 annotation_check | — |

业务侧典型用法：

- **送审前自检**：调 verify(active)，确认所有批注回复完
- **校核/审核通过前自检**：调 verify(agree)，处理所有 pending；如有 rejected，按 `recommended_action="return"` 提示走驳回流程
- **驳回前自检**：调 verify(return)，只确认当前节点可驳回；是否允许驳回、驳回到谁、驳回原因由 PMS 流程决定

---

## 3. 状态码变化

| 状态码 | 旧 | 新 |
|---|---|---|
| 200 + passed=false | 业务规则阻断（owner / 节点 / 终态 / annotation） | 同；`return` 不因 annotation_check 阻断 |
| 400 | 缺 next_step / 缺 actor / action 不识别 / 节点不识别 | 仅 action 不识别、actor 在 debug_token 模式缺失等格式错；**verify 路径不再因缺 next_step 报 400** |
| 401 | token 校验失败 | 同 |
| 404 | form_id 不存在 | 同 |
| 409 | sync 路径业务冲突（终态 / 节点 / 缺 next_step 等） | 同；`return` 不因 annotation_check 返回 409 |

---

## 4. 软阻断结构化诊断字段

verify 软阻断响应中的 `data` 字段保持原结构，常用字段：

```json
{
  "passed": false,
  "action": "agree",
  "block_code": "ANNOTATION_CHECK_FAILED",
  "current_node": "jd",
  "task_status": "submitted",
  "next_step": "sh",
  "expected_next_node": "sh",
  "actor_id": "JH",
  "owner_id": "JH",
  "owner_source": "checker",
  "reason": "存在已驳回批注，请改走驳回流程",
  "recommended_action": "return"
}
```

**`recommended_action` 取值规范化**：`"proceed"` / `"block"` / `"return"`。

---

## 5. 升级建议

### 5.1 不需要立刻动的接入方

如果你当前调用 verify 时已经按完整请求体（带 actor + next_step）发，**完全不需要改**。后端会静默忽略 verify 路径不再消费的字段。

### 5.2 推荐升级的接入方

1. **可以瘦身请求体**：verify 调用减到 `form_id + token + action`，前端节省 token 编码、网络字节数和潜在的字段拼写错误。
2. **业务文案对齐 action-aware**：用户看到驳回失败时，错误文案应该指向节点、终态或 `next_step` 缺失，而不是批注数量。
3. **预校验流程保留 return 分支**：建议 return 也先 verify(return)，用于发现当前节点不可驳回、单据已终态等流程阻断。

### 5.3 必须修改的接入方（极少）

只有以下情况必须修代码：

- 你的业务需要 sync return → 必须由 PMS 显式传 `next_step`，Plant3D 不应根据 `targetNode/action/currentNode` 推导。
- 你曾经依赖 verify 缺 next_step 时返回 400 来做客户端校验 → v3 起这种情况返 200 + passed=true（如果其他条件都满足）；客户端校验请改为 verify 通过后 sync 前自校。

---

## 6. 测试覆盖

后端 4 个 commit 包含 28 条新测试：

- 18 条纯函数单测覆盖检查矩阵的全部分支（`annotation_check.rs::gate_decision_tests`）
- 10 条集成测试覆盖 verify 路径 8 用例 + sync 路径 2 用例（`platform_api/tests.rs::test_verify_*` / `test_sync_*`）

如果你的接入方有自己的契约测试，建议同步加：

- `verify(action="active")` 在 sj 节点 + 1 条 open 批注 → 期望 `passed=false`、`recommended_action="block"`、reason 包含 "未处理批注"
- `verify(action="return")` 在 jd 节点 + 全 approved 批注 → 期望不因 annotation_check 阻断；如节点合法则 `passed=true`
- `verify(action="stop")` 即使有 pending 批注仍 → 期望 `passed=true`

---

## 7. 联系人

回归报告 / 兼容问题反馈：通过本仓 issue 或 PMS 联调群。

