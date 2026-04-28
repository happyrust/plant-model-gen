# PDMS Hardening M3-M5 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 完成 PDMS 变换修复后续硬化里尚未落地的 M3/M4/M5：补齐控制台相关 API 的 HTTP 冒烟脚本、补齐 web_server 启动期路由清单打印、补齐 plant3d-web 对 PDMS API 404 的结构化诊断。

**Architecture:** 以 `plant-model-gen/src/web_api/mod.rs` 里已存在的 `assemble_stateless_web_api_routes()` 为现状基线，优先在 `plant-model-gen` 内增加“可回归脚本 + 路由可观测性”，再在 `plant3d-web` 的 PDMS API fetch 层收口错误提示。验证遵循仓库约束：不新增 Rust `#[cfg(test)]` 或 web_server 单测，统一使用真实启动的服务、HTTP 请求和 CLI/type-check 做验收。

**Tech Stack:** Rust / Axum / PowerShell / Vue 3 / TypeScript / Fetch API

---

## 当前基线（动手前确认）

- 已完成：`M1` 修复提交、`M2` 路由聚合提交，最近相关 commit 为 `21470a9 refactor(web_server): introduce assemble_stateless_web_api_routes to prevent silent route miss`
- 已落地代码：`src/web_api/mod.rs` 中已有 `assemble_stateless_web_api_routes()`
- 未落地缺口：
  - `scripts/verify_pdms_console_api.ps1` 尚不存在
  - `src/web_server/mod.rs` 尚无 `AIOS_PRINT_ROUTES` 或“registered routes”日志
  - `plant3d-web/src/api/genModelPdmsAttrApi.ts` 仍只抛出裸 `HTTP 404 Not Found`
- 推荐执行顺序：`M4 -> M5 -> M3`
  - `M4` 最快形成回归护栏，且只改 `plant-model-gen`
  - `M5` 依托已完成的 `M2`，能直接把“是否漏挂载”暴露到启动日志
  - `M3` 是跨仓体验增强，适合在后端护栏补齐后收尾

## Task 1: 实现 PDMS 控制台 API 冒烟脚本（M4）

**Files:**
- Create: `scripts/verify_pdms_console_api.ps1`
- Reference: `scripts/verify_fixing_position.ps1`
- Reference: `src/web_api/pdms_transform_api.rs`
- Reference: `src/web_api/pdms_attr_api.rs`
- Reference: `src/web_api/pdms_model_query_api.rs`

**Step 1: 固定脚本输入与覆盖范围**

- 参数：
  - `BaseUrl`，默认 `http://localhost:3100`
  - `Refno`，默认 `17496_152153`
  - 可选 `SkipDataCheck`，仅校验接口可达和 JSON 结构，不强制 `success=true`
- 第一批覆盖 6 个 case：
  - `GET /api/pdms/transform/{refno}`
  - `GET /api/pdms/transform/compute/{refno}`
  - `GET /api/pdms/ui-attr/{refno}`
  - `GET /api/pdms/ptset/{refno}`
  - `GET /api/pdms/type-info?refno={refno}`
  - `GET /api/pdms/children?refno={refno}`

**Step 2: 先写脚本骨架与请求循环**

```powershell
param(
  [string]$BaseUrl = "http://localhost:3100",
  [string]$Refno = "17496_152153",
  [switch]$SkipDataCheck
)

$cases = @(
  @{ Name = "q pos"; Path = "/api/pdms/transform/$Refno" },
  @{ Name = "q pos(compute)"; Path = "/api/pdms/transform/compute/$Refno" },
  @{ Name = "q ui-attr"; Path = "/api/pdms/ui-attr/$Refno" },
  @{ Name = "q ptset"; Path = "/api/pdms/ptset/$Refno" },
  @{ Name = "q type-info"; Path = "/api/pdms/type-info?refno=$Refno" },
  @{ Name = "q children"; Path = "/api/pdms/children?refno=$Refno" }
)
```

**Step 3: 为每个 case 写最小结构校验**

- `transform`：`world_transform` 为 16 长度数组，或 `SkipDataCheck` 时只要求返回 JSON
- `transform/compute`：`world_translation` 为 3 长度数组，或 `SkipDataCheck`
- `ui-attr`：存在 `attrs`
- `ptset`：存在 `ptset`
- `type-info`：存在 `noun` / `owner_refno` 字段之一即可
- `children`：存在 `children` 数组

**Step 4: 统一输出 PASS/FAIL 表与非零退出码**

- 成功时打印 `[PASS] <case name>`
- 失败时打印请求 URL、状态/异常文本、关键 JSON 片段
- 任一失败则 `exit 1`

**Step 5: 启动 web_server 并实际跑脚本**

Run: `cargo build --bin web_server --features web_server`

Expected: 编译通过

Run: `powershell -ExecutionPolicy Bypass -File .\scripts\verify_pdms_console_api.ps1 -BaseUrl "http://localhost:3100" -Refno "17496_152153"`

Expected: 6 个 case 全部 PASS；若本地数据不完整，使用 `-SkipDataCheck` 至少验证 6 条路由均可达且返回 JSON

---

## Task 2: 实现启动期路由清单打印（M5）

**Files:**
- Modify: `src/web_api/mod.rs`
- Modify: `src/web_server/mod.rs`

**Step 1: 在 `src/web_api/mod.rs` 抽出“静态路由路径清单”**

- 新增一个与 `assemble_stateless_web_api_routes()` 同步维护的函数，例如：

```rust
#[cfg(feature = "web_server")]
pub fn stateless_web_api_route_paths() -> Vec<&'static str> {
    vec![
        "/api/room-tree/...",
        "/api/pdms/ui-attr/{refno}",
        "/api/pdms/transform/{refno}",
        "/api/pdms/transform/compute/{refno}",
        "/api/pdms/type-info",
        "/api/pdms/children",
        // ... 其余 stateless route 与 nest 前缀
    ]
}
```

**Step 2: 在 `src/web_server/mod.rs` 拼出最终打印列表**

- 先打印当前文件里手写注册的核心 `/api/tasks`、`/api/model/*`、`/api/surreal/*`、`/api/database/*`、`/api/incremental/*`
- 再拼接：
  - `stateless_web_api_route_paths()`
  - `search` / `upload` / `collision` / `e3d_tree` / `noun_hierarchy` / `spatial_query` / `room_api` 等 stateful 路由的已知前缀

**Step 3: 增加受控开关**

- 默认在 debug/dev build 打印
- release 下仅当 `AIOS_PRINT_ROUTES=1` 时打印

建议逻辑：

```rust
let should_print_routes = cfg!(debug_assertions)
    || std::env::var("AIOS_PRINT_ROUTES").ok().as_deref() == Some("1");
```

**Step 4: 在服务监听前打印**

- 打印标题：`[web_server] registered routes`
- 每行一个路径，稳定排序或稳定分组，不要每次输出顺序抖动

**Step 5: 启动验证**

Run: `cargo build --bin web_server --features web_server`

Expected: 编译通过

Run: `$env:AIOS_PRINT_ROUTES=1; cargo run --bin web_server --features web_server`

Expected: 启动日志出现 `registered routes` 段落，且包含：
- `/api/pdms/transform/{refno}`
- `/api/pdms/transform/compute/{refno}`
- `/api/pdms/ui-attr/{refno}`
- `/api/pdms/type-info`
- `/api/pdms/children`

---

## Task 3: 增强 plant3d-web 的 PDMS API 404 诊断（M3）

**Files:**
- Modify: `../plant3d-web/src/api/genModelPdmsAttrApi.ts`
- Reference: `../plant3d-web/src/api/genModelE3dApi.ts`

**Step 1: 只改与 PDMS 控制台直接相关的 fetch 层**

- 第一轮只改 `genModelPdmsAttrApi.ts`
- 不扩大到所有 `src/api/*.ts`，避免把“本次 PDMS 路由漏挂载”议题变成全仓风格重构

**Step 2: 把 404 空 body 转成结构化提示**

```ts
if (!resp.ok) {
  const text = await resp.text().catch(() => '');

  if (resp.status === 404 && !text.trim()) {
    throw new Error(
      `HTTP 404 at ${url}\n` +
      '后端可能没有挂载这个 API（例如 web_server 路由装配遗漏），或请求路径拼写错误。'
    );
  }

  throw new Error(`HTTP ${resp.status} ${resp.statusText} at ${url}: ${text}`);
}
```

**Step 3: 保证普通 404 不误报**

- 只有 `404 + 空 body` 才提示“可能未挂载 API”
- 带正文的 404 继续原样透出，避免把“资源不存在”误判成“路由未注册”

**Step 4: 类型检查**

Run: `npm run type-check`

Working directory: `../plant3d-web`

Expected: type-check 通过

---

## Task 4: 组合验证与结果记录

**Files:**
- Modify: `docs/plans/2026-04-24-pdms-hardening-m3-m5-implementation-plan.md`

**Step 1: 记录本轮实际执行结果**

- 记录是否已完成：
  - `scripts/verify_pdms_console_api.ps1`
  - `registered routes` 日志打印
  - `genModelPdmsAttrApi.ts` 错误提示增强

**Step 2: 记录验证命令与样例输出**

- `cargo build --bin web_server --features web_server`
- `powershell -ExecutionPolicy Bypass -File .\scripts\verify_pdms_console_api.ps1 ...`
- `npm run type-check`

**Step 3: 标注剩余风险**

- 若本地 DB 数据不完整，脚本先以 `-SkipDataCheck` 保障“路由和 JSON 结构”层面的回归
- 若未来要扩展到 `plant3d-web` 其他 API 文件，应单开 follow-up，不在本轮顺手扩散

---

## 执行备注

- 相关规则：
  - `AGENTS.md`：不要为 web_server 新增 test；运行真实服务并用 HTTP/Post 验证
  - `@superpowers:verification-before-completion`：完成前必须给出实际验证证据
- 本轮不做的事：
  - 不补 Rust `#[cfg(test)]`
  - 不做跨仓 fetchJson 大重构
  - 不提交 commit / PR（除非用户另行要求）

## 完成定义

- `scripts/verify_pdms_console_api.ps1` 可在本地对 6 条 PDMS 控制台相关接口做 PASS/FAIL 汇总
- `web_server` 启动日志可打印已注册路由，至少能看到 PDMS 相关 5 条关键路径
- `plant3d-web` 对 `404 + 空 body` 的 PDMS 请求会给出带 URL 的“可能未挂载 API”提示
- 本轮验证命令有实际输出，且结果记录回本文档

## 本轮执行记录（2026-04-24）

### 已完成

- 新增 `scripts/verify_pdms_console_api.ps1`
  - 覆盖 6 条 PDMS 控制台相关接口：`transform`、`transform/compute`、`ui-attr`、`ptset`、`type-info`、`children`
  - 对 `ptset` 增加“路由可达但当前 refno 无数据”的提示，避免被样例数据偶然性卡死
- 修改 `../plant3d-web/src/api/genModelPdmsAttrApi.ts`
  - `404 + 空 body` 时抛出带 URL 的结构化错误
  - `5xx` 时额外输出 `console.error`

### 已执行验证

- `powershell -ExecutionPolicy Bypass -File .\scripts\verify_pdms_console_api.ps1 -BaseUrl "http://127.0.0.1:3100" -Refno "17496_152153"`
  - 结果：`6 passed, 0 failed`
  - 备注：`q ptset` 为“route reachable but ptset data is unavailable”，说明路由与 JSON 结构正常，但当前样例 refno 没有 ptset 数据
- `npm run type-check`（工作目录：`../plant3d-web`）
  - 结果：通过

### 下一步

- ~继续执行 `M5`：在 `src/web_api/mod.rs` / `src/web_server/mod.rs` 增加启动期路由清单打印~ → 已完成，见下

## M5 执行记录（2026-04-24，补录）

### 已完成

- `src/web_api/mod.rs` 新增 `pub fn stateless_web_api_route_paths() -> Vec<&'static str>`（`#[cfg(feature = "web_server")]`）
  - 与 `assemble_stateless_web_api_routes()` 同步维护，覆盖 room-tree / pdms_attr / pdms_transform / ptset / pdms_model_query / review_integration / platform_api / jwt_auth / review_api / scene_tree / mbd_pipe / pipeline_annotation（nested `/api/pipeline`）/ version（nested `/api`）
  - 格式 `METHOD  /path/{param}`（METHOD 左对齐 7 格，便于裸 `println!` 对齐）
  - 包含 PDMS 5 条关键路径：`/api/pdms/transform/{refno}` / `/api/pdms/transform/compute/{refno}` / `/api/pdms/ui-attr/{refno}` / `/api/pdms/type-info` / `/api/pdms/children`
- `src/web_server/mod.rs` 新增 `fn maybe_print_registered_routes()`
  - 开关：`cfg!(debug_assertions) || AIOS_PRINT_ROUTES == "1"`
  - 在 `admin_auth_handlers::start_session_cleanup_timer()` 之后、`axum::serve()` 之前打印
  - 分三段：stateless web_api / stateful web_api prefixes / main router manual prefixes
  - stateful 部分列出前缀与其所需 state（`spatial_query` / `noun_hierarchy` / `e3d_tree` / `room_api` / `collision` / `search` / `upload`）
  - main router 部分列出前缀（`/api/tasks*` / `/api/model/*` / `/api/surreal/*` / `/api/database/*` / `/api/incremental/*` / `/ws/*` / `/admin/*` / `/console/*`）

### 已执行验证

- `cargo check --bin web_server --features web_server`
  - 结果：`Finished dev profile [unoptimized + debuginfo] target(s) in 53.44s`（零错误；告警均为 `pdms_io` 子项目既有未使用项，与本轮无关）
- 未执行 `cargo build --bin web_server --features web_server`：本机 `target\debug\web_server.exe` 被一个运行中的 web_server 进程（PID 27112）持有文件句柄，`cargo build` 报 `failed to remove file ... 拒绝访问`。为避免意外打断用户的本地服务，使用 `cargo check` 先做编译验证。
- 未执行 `cargo run --bin web_server --features web_server`：同上；待用户停掉占用进程或切换到独立目标目录后，再执行并核对实际启动日志中的 `[web_server] registered routes` 段落。

### 剩余事项

- 运行时验证：需要在本机可 rebuild 后启动 web_server（`AIOS_PRINT_ROUTES=1` 或 debug build），核对启动日志包含：
  - `[web_server] registered routes (stateless web_api)`
  - `GET    /api/pdms/transform/{refno}` / `GET    /api/pdms/transform/compute/{refno}` / `GET    /api/pdms/ui-attr/{refno}` / `GET    /api/pdms/type-info` / `GET    /api/pdms/children`
- 维护约定：后续若修改 `assemble_stateless_web_api_routes()` 新增/删除路由，必须同步 `stateless_web_api_route_paths()`。
