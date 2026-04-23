# 控制台 `q pos` 返回 404 — `/api/pdms/transform` 路由漏注册修复计划

**日期**: 2026-04-23
**状态**: 草案、待实施
**涉及仓库**:
- `plant-model-gen`（本次唯一修改仓库）
- `plant3d-web`（仅回归，不修改）

---

## 1. 问题现象

用户在 `plant3d-web` 三维场景控制台：

```
= 24381/145018   → CE set to: 24381_145018
q pos            → Failed to query transform: Error: HTTP 404 Not Found
q ori            → 同样 HTTP 404 Not Found
q pos wrt owner  → 同样 HTTP 404 Not Found（错误路径：`Failed to query element transform`）
q ori wrt owner  → 同样 HTTP 404 Not Found
```

请求 URL：

```
GET /api/pdms/transform/24381_145018
```

后端返回 Axum 默认 404。

---

## 2. 链路排查结论

| 层级 | 文件 / 位置 | 状态 |
| --- | --- | --- |
| 前端命令 | `plant3d-web/src/composables/usePdmsConsoleCommands.ts:223,258,282,299,326,343` | ✓ 正确调用 `pdmsGetTransform(id)` |
| 前端 API 客户端 | `plant3d-web/src/api/genModelPdmsAttrApi.ts:122-124` | ✓ 构造的 URL `/api/pdms/transform/{refno}` 正确 |
| 后端路由定义 | `plant-model-gen/src/web_api/pdms_transform_api.rs:6-13` | ✓ `create_pdms_transform_routes()` 定义完整（`get_transform` + `compute_transform`） |
| 后端模块导出 | `plant-model-gen/src/web_api/mod.rs:25` | ✓ `pub use pdms_transform_api::create_pdms_transform_routes;` |
| 后端路由挂载 | `plant-model-gen/src/web_server/mod.rs:69-77,319,1125-1143` | ✗ **未 import、未实例化、未 merge** |

### 2.1 根因

`web_server/mod.rs` 的 `use crate::web_api::{...}` 列表里没有 `create_pdms_transform_routes`，也没有 `let pdms_transform_routes = ...;`，更没有 `.merge(pdms_transform_routes)`。

因此 `axum` 的 Router 里从未登记 `/api/pdms/transform/*` 前缀，所有请求都落入 fallback（`.fallback(app_history_fallback)`），被前端同源反代或历史回退返回为 404。

### 2.2 排除其它路由是否存在同样问题

对 `src/web_api/` 下所有 `create_*_routes()` 做完整审计：

- 定义数量：19 个（含 `collision/e3d_tree/jwt_auth/mbd_pipe/noun_hierarchy/pdms_attr/pdms_model_query/pdms_transform/pipeline_annotation/platform_api/ptset/review_api/review_integration/room_tree/scene_tree/search/spatial_query/upload/version`）。
- 实际挂载：18 个。
- **唯一遗漏**：`pdms_transform`。

所以本次是孤立遗漏，不是系统性架构问题，无需大范围整改。

### 2.3 历史影响范围

所有依赖 `pdmsGetTransform` 的前端入口均失效：

1. 控制台命令 `q pos` / `q ori` / `q pos wrt owner` / `q ori wrt owner`（四处）——全部报错
2. 任何复用该 API 的后续扩展点也会在开发阶段直接 404

`scripts/verify_fixing_position.ps1` 调用的是 `/api/pdms/transform/compute/{refno}`，一并受影响（同一 Router 里的另一个 route 也挂不上）。

---

## 3. 修复方案

### 3.1 目标

- 让 `GET /api/pdms/transform/{refno}` 与 `GET /api/pdms/transform/compute/{refno}` 在 web server 启动后对外可用
- 保证回归脚本与控制台命令同时恢复
- 在 CI / 启动路径上补一个最小防御，避免同类漏注册再次发生

### 3.2 核心改动（最小闭环）

**文件**：`plant-model-gen/src/web_server/mod.rs`

```rust
// 位置 1：L69-77 的 use crate::web_api::{...}
use crate::web_api::{
    CollisionApiState, E3dTreeApiState, NounHierarchyApiState, SearchApiState,
    SpatialQueryApiState, UploadApiState, create_collision_routes, create_e3d_tree_routes,
    create_jwt_auth_routes, create_mbd_pipe_routes, create_noun_hierarchy_routes,
    create_pdms_attr_routes, create_pdms_model_query_routes,
    create_pdms_transform_routes,                          // ← 新增
    create_pipeline_annotation_routes,
    create_platform_api_routes, create_ptset_routes, create_review_api_routes,
    create_review_integration_routes, create_room_tree_routes, create_scene_tree_routes,
    create_search_routes, create_spatial_query_routes, create_upload_routes, create_version_routes,
};
```

```rust
// 位置 2：L319 附近，紧跟 pdms_attr_routes 初始化
let pdms_attr_routes = create_pdms_attr_routes();
let pdms_transform_routes = create_pdms_transform_routes();   // ← 新增

// 初始化 Ptset API
let ptset_routes = create_ptset_routes();
```

```rust
// 位置 3：L1129 附近的 Router builder
.merge(pdms_attr_routes)
.merge(pdms_transform_routes)       // ← 新增
.merge(ptset_routes)
.merge(pdms_model_query_routes)
```

### 3.3 防御性改动（可选，本轮建议一起做）

在 `plant-model-gen/src/web_server/mod.rs` 的路由装配末端，开发构建中打印已注册路径清单，便于人眼复核；或者给 `web_api/mod.rs` 加一层集中装配函数（见 §6.2）。本轮计划仅写入 TODO，不强制实现，避免改动面过大。

---

## 4. 验证计划

按 `AGENTS.md` 约定：不跑 `cargo test`；启动 web_server 后用 HTTP 验证。

### 4.1 启动

```powershell
# 或使用当前仓库里已有的启动脚本
cargo run --bin plant-model-gen -- web-server --config <your-config>
```

### 4.2 主路由冒烟

```powershell
# 拿一条确实存在的 refno
$refno = "24381_145018"

# 缓存路径
Invoke-RestMethod "http://127.0.0.1:<port>/api/pdms/transform/$refno" | ConvertTo-Json -Depth 6

# 计算路径
Invoke-RestMethod "http://127.0.0.1:<port>/api/pdms/transform/compute/$refno" | ConvertTo-Json -Depth 6
```

预期：返回 JSON，`success=true`（若 refno 数据完整）或 `success=false` + `error_message` 明确提示“未找到变换矩阵数据”。**不再出现 HTTP 404。**

### 4.3 既有脚本回归

```powershell
./scripts/verify_fixing_position.ps1
```

脚本里 4 个 NOZZ 样例必须全部 `success=true`，否则回归失败。

### 4.4 前端场景回归

在 `plant3d-web` 控制台依次执行：

```
= 24381/145018
q pos
q ori
q pos wrt owner
q ori wrt owner
```

预期：4 条均输出正确数字，不再出现 `Failed to query transform: Error: HTTP 404 Not Found`。

### 4.5 边界用例

| 场景 | 命令 | 预期 |
| --- | --- | --- |
| refno 存在、有 `pe_transform` | `GET /api/pdms/transform/<live_refno>` | 200 + success=true + world_transform 16 个 f64 |
| refno 存在、无 `pe_transform` | 同上 | 200 + success=false + `未找到变换矩阵数据` |
| refno 格式非法 | `GET /api/pdms/transform/foo` | Axum Path 解析失败 → 400（由 `RefnoEnum` 反序列化决定） |
| 自 refer 自己作为 owner | `q pos wrt owner` 对 root 元素 | 返回 World 位置，提示 `no owner` |

---

## 5. 风险评估

| 风险 | 等级 | 缓解 |
| --- | --- | --- |
| 修改 `web_server/mod.rs` 合并顺序影响其它 Route | 低 | 仅在已有 pdms_attr_routes 之后追加，不改变已存在的 merge 顺序 |
| `RefnoEnum` Path 提取对非法入参抛 400 而不是 success=false | 低 | 已经是既有行为，其它 pdms_* 路由一致，不做额外处理 |
| 编译期二进制体积变化 | 可忽略 | 该模块早已 include，仅差一个 Router 挂载 |
| 是否需要 admin 鉴权 | 中 | 其它 pdms_* 路由都是公开挂载（未 route_layer），保持一致；如需收敛另立一轮 |

---

## 6. 后续事项

### 6.1 单点 issue

建议开一个内部 issue：**「web_server 装配缺失 `/api/pdms/transform` 路由导致控制台 q pos/q ori 永远 404」**，挂上本文档。

### 6.2 架构改进（单独推进，非本轮必须）

考虑把 `src/web_api/mod.rs` 里再暴露一个 `assemble_all_web_api_routes()` 聚合函数，让 `web_server` 侧只调一次，避免再漏挂新路由。该调整涉及状态注入顺序，单独起一个计划处理。

### 6.3 前端防御

`plant3d-web/src/api/genModelPdmsAttrApi.ts` 在 `fetchJson` 中对 404 额外打一条“后端未部署此 API”的提示日志，便于下次问题更快定位。可与后端一起上线，也可稍后补。

---

## 7. 执行清单

- [ ] 修改 `plant-model-gen/src/web_server/mod.rs` 三处（import / let / merge）
- [ ] `cargo build -p plant-model-gen --features web_server` 确认编译通过
- [ ] 起 web_server，跑 §4.2 / §4.3 / §4.4
- [ ] 确认本地全部通过后，在目标部署机（123.57.182.243）复测一次
- [ ] 在当前开发分支提交，commit 信息示例：
      `fix(web_server): register pdms_transform_routes so q pos/ori no longer 404`
- [ ] 归档本计划到 `docs/plans/`（已完成）
- [ ] 若 6.2/6.3 落地，另起计划
