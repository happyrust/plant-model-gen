# 控制台 Q POS 失效修复 · 开发计划

日期：2026-04-23
作者：codex (agent)
状态：drafting → executing

## 1. 背景

前端 3D 控制台里的 PDMS 风格命令（`Q POS` / `Q ORI` / `Q POS WRT OWNER` / `Q ORI WRT OWNER`）最近失效：选中元素后输入 `q pos` 得不到世界坐标，或得到语义错误的局部坐标。

## 2. 链路与失效根因

前端调用链：

- `plant3d-web/src/composables/usePdmsConsoleCommands.ts::Q POS`（第 221–241 行）
- → `pdmsGetTransform(refno)`（`plant3d-web/src/api/genModelPdmsAttrApi.ts:122`）
- → 后端 `GET /api/pdms/transform/{refno}` → `plant-model-gen/src/web_api/pdms_transform_api.rs::get_transform`（第 27–97 行）
- → SurrealDB `SELECT world_trans.d as world_trans FROM pe_transform:⟨refno⟩ WHERE world_trans != none`

失效根因：

1. **硬依赖 `pe_transform` 缓存表**：`get_transform` 唯一数据源是 `pe_transform`。当项目初始化未写入 / 写入结构变更 / 被刷新删除时，直接返回 `Ok(None)`（错误信息：`未找到变换矩阵数据`）。
2. **实时计算 API 未被利用**：同文件第 119–206 行已经存在 `compute_transform`（路由 `/api/pdms/transform/compute/{refno}`），它通过 `aios_core::transform::get_world_mat4` 实时算，不依赖缓存。但前端控制台、`pdmsGetTransform` 都没有使用它，缓存一坏就全链路失效。
3. **前端 fallback 语义错误**：`Q POS` 在后端失败后，`usePdmsConsoleCommands.ts` 第 229–233 行会回退到读 `attrs.POS / attrs.POSITION`，但该值是**局部坐标**，却被打印成 `Position (Local)`，用户期望的是世界坐标，造成"坐标错乱"的错觉。

## 3. 目标（本轮）

让 `Q POS` / `Q ORI` / `Q POS WRT OWNER` / `Q ORI WRT OWNER` 在 `pe_transform` 不可用的项目里仍然能返回正确的世界坐标，同时不破坏现有请求语义。

## 4. 方案

采用 **P0（后端兜底）+ P1（前端语义澄清）** 组合：

### P0 · 后端 `get_transform` 自动 fallback（核心修复）

在 `plant-model-gen/src/web_api/pdms_transform_api.rs::get_transform` 中：

- 保留现有「查 `pe_transform` 表」的路径作为快速路径。
- 当 SurrealDB 返回 `None` / 查询失败 / 字段缺失时，自动调用 `aios_core::transform::get_world_mat4(refno, false)` 实时计算世界矩阵。
- Owner 仍然按原来方式从 `pe:⟨refno⟩.owner` 取；若 `owner` 查询也失败，用 `aios_core::get_named_attmap` 的 `get_owner()` 兜底（复用 `compute_transform` 已验证的方式）。
- 响应结构 (`TransformResponse`) 不变：`success/refno/world_transform/owner/error_message`。这样前端 `pdmsGetTransform` 和 `usePdmsConsoleCommands` 不需要改动。
- 为可观测性：在 `error_message` 或日志里带上 `source=cache | compute`（可选，不影响成功路径）。

### P1 · 前端 fallback 文案澄清（小改）

在 `plant3d-web/src/composables/usePdmsConsoleCommands.ts::Q POS`：

- fallback 读属性得到的是**局部**坐标，把输出标签从 `Position (Local)` 改为 `Position (Attr, local — world not available)`，并追加一条 `info` 日志提示 "world transform unavailable, showing attribute POS"。避免用户把局部当成世界。
- `Q ORI` fallback 同理。

### 暂不做（留给后续）

- 统一 `/api/pdms/transform/{refno}` 与 `/compute` 为单一端点（迁移面较大，先不动）。
- 增加 `pe_transform` 自动重建任务。
- 修 `Q WPOS / Q WORI` 的属性依赖。

## 5. 变更清单

| 文件 | 动作 | 说明 |
|---|---|---|
| `plant-model-gen/src/web_api/pdms_transform_api.rs` | 改 | `get_transform` 增加 compute fallback；抽 helper 复用 `compute_transform` 里的逻辑 |
| `plant3d-web/src/composables/usePdmsConsoleCommands.ts` | 改 | `Q POS` / `Q ORI` fallback 分支文案与日志调整 |

## 6. 验证策略

按 AGENTS.md 规定：**不使用 cargo test**，改用"起服务 + POST/GET 验证"。

1. `cargo build -p plant-model-gen` 保证编译通过（不跑 tests）。
2. 对一个已知 refno（例如选中的 EQUI）本机起 web_server 后：
   - `curl -s "http://127.0.0.1:<port>/api/pdms/transform/{refno}" | jq`，确认 `pe_transform` 有缓存时和没有缓存时都能拿到 `world_transform`。
   - 对比 `/api/pdms/transform/compute/{refno}` 的 `world_transform` 与 fallback 后的一致。
3. 前端 `plant3d-web` 本地起 dev server，在 3D 控制台依次输入 `q pos` / `q ori` / `q pos wrt owner` / `q ori wrt owner`，确认：
   - 缓存存在时输出与之前一致。
   - 缓存不存在时能拿到世界坐标（不再报 `未找到变换矩阵数据`）。
   - fallback 到属性时，输出文案带 `Attr, local — world not available`。

## 7. 执行步骤

1. [ ] 写本计划文件（当前步骤）。
2. [ ] 修改 `pdms_transform_api.rs`：抽 `compute_world_transform_fallback` helper；`get_transform` 走 cache → fallback → compute 链。
3. [ ] 修改 `usePdmsConsoleCommands.ts` 的 `Q POS` / `Q ORI` fallback 文案。
4. [ ] `cargo build -p plant-model-gen` 编译自检。
5. [ ] lint 检查（readLints）。
6. [ ] 向用户汇报结果，附带验证命令样例。

## 8. 风险

- `aios_core::transform::get_world_mat4` 内部也会访问 DB 或属性缓存，fallback 路径下第一次请求可能比缓存版慢（毫秒级）。对控制台点查无影响。
- 如果某个 refno 既没缓存、也无法实时计算（例如无 owner 链路），两路都失败。保持当前"返回 `success=false` + `error_message`"语义即可，前端已有 fallback。
