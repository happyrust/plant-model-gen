# PDMS 变换接口修复的后续硬化与防御计划

**日期**: 2026-04-23
**状态**: 草案、待评审
**上游**: [`2026-04-23-pdms-transform-route-missing-registration-fix.md`](./2026-04-23-pdms-transform-route-missing-registration-fix.md)
**涉及仓库**:
- `plant-model-gen`
- `plant3d-web`

---

## 0. 背景

`GET /api/pdms/transform/{refno}` 返回 404 的即时修复已经完成并验证：

- `scripts/verify_fixing_position.ps1` 通过（世界坐标误差 0.006mm）
- `q pos` / `q ori` / `q pos wrt owner` / `q ori wrt owner` 在本地完整走通

但这次事故暴露了几条系统性问题：

1. 路由在 `src/web_api/*.rs` 中定义完整却可以被 `web_server/mod.rs` **静默遗漏**挂载
2. 前端 `fetchJson` 看到 404 时只透出一句 `HTTP 404 Not Found`，无法区分"接口不存在"还是"资源不存在"
3. 没有任何启动期/CI 期守卫能阻止这类漏挂载
4. `q pos wrt owner` 等复合命令的端到端覆盖极弱（仓库里几乎没有前端控制台命令的冒烟测试资产）

本计划给出后续的 M1→M5 五个里程碑，目标是把这次单点 bug 折射出的共性风险一次性填平。

---

## 1. 目标

| # | 目标 | 成功判定 |
| --- | --- | --- |
| G1 | 把变换 API 的修复稳稳落盘 | 提交合入主干，部署机端到端回归通过 |
| G2 | 彻底消除"新增路由忘记 merge"这类遗漏 | 新增 `web_api/*_routes` 必须自动生效，不需要再改 `web_server/mod.rs` |
| G3 | 前端对后端 404 有可诊断的错误 | 控制台/开发者工具能看到"后端未部署此 API"这类指向性提示 |
| G4 | PDMS 控制台关键命令的端到端可回归 | 有脚本或自动化能复现 `q pos/q ori/q pos wrt owner/q ori wrt owner` |
| G5 | web_server 启动期能看到完整路由清单 | dev 构建下首次监听时打印所有已注册路径 |

---

## 2. 里程碑拆分

### M1 · 落盘当前修复（0.5 天）

**动作**：

- 本地 commit：
  ```
  fix(web_server): register pdms_transform_routes so q pos/ori no longer 404
  ```
  内容包括：
  - `src/web_server/mod.rs` 的三处改动
  - `docs/plans/2026-04-23-pdms-transform-route-missing-registration-fix.md`
  - 本计划本身
- 推送到当前开发分支
- 部署机 `123.57.182.243` 拉最新分支并重启 web_server（使用 `root / Happytest123_`）
- 用户端 `plant3d-web` 控制台在生产环境跑一遍 `q pos`/`q ori`

**完成判定**：部署机上 `curl http://127.0.0.1:3100/api/pdms/transform/<refno>` 返回 200，且 plant3d-web 控制台不再出现该 404。

**风险**：部署机上可能有历史 surreal 进程残留占用 8020，需要先用 `systemctl` 或 `pgrep | xargs kill` 清理后再起 web_server。

---

### M2 · 路由聚合装配 `assemble_all_web_api_routes`（1 天）

**问题**：

目前 `src/web_server/mod.rs` 把 19 个 `create_*_routes()` 手动：
- import 三次（use 列表）
- 实例化若干 `let xxx_routes = ...`
- merge 若干次（部分带状态、部分 nest）

写法高度重复、易漏、顺序难读。本次 404 就是证据。

**方案**：

在 `src/web_api/mod.rs` 新增一个聚合装配函数，吃掉所有无状态 `create_*_routes()`：

```rust
// plant-model-gen/src/web_api/mod.rs

pub struct WebApiAssembly {
    pub stateless: Router,
    pub collision: Router,
    pub e3d_tree: Router,
    pub noun_hierarchy: Router,
    pub spatial_query: Router,
    pub search: Router,
    pub upload: Router,
}

pub fn assemble_all_web_api_routes(
    collision_state: CollisionApiState,
    e3d_tree_state: E3dTreeApiState,
    noun_hierarchy_state: NounHierarchyApiState,
    spatial_query_state: SpatialQueryApiState,
    search_state: SearchApiState,
    upload_state: UploadApiState,
) -> WebApiAssembly {
    let stateless = Router::new()
        .merge(create_pdms_attr_routes())
        .merge(create_pdms_transform_routes())   // 本次遗漏点，未来不再可能
        .merge(create_pdms_model_query_routes())
        .merge(create_ptset_routes())
        .merge(create_room_tree_routes())
        .merge(create_scene_tree_routes())
        .merge(create_mbd_pipe_routes())
        .merge(create_review_integration_routes())
        .merge(create_platform_api_routes())
        .merge(create_jwt_auth_routes())
        .merge(create_review_api_routes())
        .nest("/api/pipeline", create_pipeline_annotation_routes())
        .nest("/api", create_version_routes());

    WebApiAssembly {
        stateless,
        collision: create_collision_routes(collision_state),
        e3d_tree: create_e3d_tree_routes(e3d_tree_state),
        noun_hierarchy: create_noun_hierarchy_routes(noun_hierarchy_state),
        spatial_query: create_spatial_query_routes(spatial_query_state),
        search: create_search_routes(search_state),
        upload: create_upload_routes(upload_state),
    }
}
```

`src/web_server/mod.rs` 侧变为：

```rust
let api = assemble_all_web_api_routes(
    collision_state,
    e3d_tree_state,
    noun_hierarchy_state,
    spatial_query_state,
    search_state,
    upload_state,
);

app = app
    .merge(api.stateless)
    .merge(api.collision)
    .merge(api.e3d_tree)
    .merge(api.noun_hierarchy)
    .merge(api.spatial_query)
    .merge(api.search)
    .merge(api.upload);
```

**好处**：

- 新增 `create_xxx_routes()`，只需要在 `assemble_all_web_api_routes()` 里加一行；漏加 = 单元审阅一眼能看出
- `web_server/mod.rs` 从"一百多行重复装配"瘦身到十几行
- 为 M5 的路由清单打印提供单一数据源

**风险**：

- Router 的 merge 顺序对路径冲突敏感；现有顺序要原样保留，不要抖动
- `admin_*`、`room_api::*`、`create_pipeline_annotation_routes`（有 nest）等特殊情形先保留在 `web_server` 侧，不纳入聚合，避免一次改得太大

**完成判定**：

- 修改后 `cargo build --bin web_server --features web_server` 通过
- 所有现有 HTTP 接口（抽样至少 10 个）仍然返回原有状态码
- `scripts/verify_fixing_position.ps1` 再过一次

---

### M3 · 前端 `fetchJson` 对 404/5xx 诊断性增强（0.5 天）

**问题**：

`plant3d-web/src/api/genModelPdmsAttrApi.ts:59-77` 的 `fetchJson` 里：

```ts
if (!resp.ok) {
  const text = await resp.text().catch(() => '');
  throw new Error(`HTTP ${resp.status} ${resp.statusText}: ${text}`);
}
```

用户侧只看到 `Failed to query transform: Error: HTTP 404 Not Found`，既无法判断是后端没这个接口还是 refno 不存在，也无法在终端/控制台一眼看到 URL。

**方案**：

改为：

```ts
if (!resp.ok) {
  const text = await resp.text().catch(() => '');

  if (resp.status === 404 && !text.trim()) {
    const msg =
      `HTTP 404 at ${url}\n` +
      `后端可能没有挂载这个 API（web_server 装配遗漏），或路径拼写错误。\n` +
      `请检查 plant-model-gen/src/web_server/mod.rs 的路由装配。`;
    console.warn('[pdms-api]', msg);
    throw new Error(msg);
  }

  if (resp.status >= 500) {
    console.error('[pdms-api]', `HTTP ${resp.status} at ${url}: ${text}`);
  }

  throw new Error(`HTTP ${resp.status} ${resp.statusText} at ${url}: ${text}`);
}
```

要点：
- 空 body + 404 才是"接口没挂"的强信号，不要把 refno 合法 404 也打上"路由遗漏"标签
- 所有错误都带上 URL，便于从日志反推后端路径
- 控制台命令失败时 `store.addLog('error', ...)` 的文案继承改进

**涉及文件**：

- `plant3d-web/src/api/genModelPdmsAttrApi.ts`
- 建议同时改 `plant3d-web/src/api/genModelE3dApi.ts`、`plant3d-web/src/api/genModelE3dParquetApi.ts` 等同源 `fetchJson`（若有）

**完成判定**：

- 手动在浏览器里把后端请求拦截为 404 空 body，控制台能看到"web_server 装配遗漏"的提示
- 正常 refno 不存在的 404 仍然只打常规错误，不会误报

---

### M4 · PDMS 控制台命令端到端冒烟脚本（1 天）

**问题**：

`q pos`/`q ori`/`q pos wrt owner`/`q ori wrt owner`/`q wpos`/`q wori`/`q att xxx`/`q dbnum`/`q refno` 八条命令目前没有任何自动化覆盖。本次故障正是"四条命令同时没人发现 404 已存在"的直接反映。

**方案**：

新增 `plant-model-gen/scripts/verify_pdms_console_api.ps1`（不依赖前端，直接打 HTTP）：

```powershell
# 按真实控制台命令映射的 API，批量跑一遍，产出 PASS/FAIL 表
param(
    [string]$BaseUrl = "http://localhost:3100",
    [string]$Refno = "17496_152153"
)

$cases = @(
    @{ Name = "q pos";              Path = "/api/pdms/transform/$Refno";           Expect = { param($r) $r.success -and $r.world_transform -and $r.world_transform.Length -eq 16 } }
    @{ Name = "q pos(compute)";     Path = "/api/pdms/transform/compute/$Refno";   Expect = { param($r) $r.success -and $r.world_translation -and $r.world_translation.Length -eq 3 } }
    @{ Name = "q ui-attr";          Path = "/api/pdms/ui-attr/$Refno";             Expect = { param($r) $r.success -and $r.attrs } }
    @{ Name = "q ptset";            Path = "/api/pdms/ptset/$Refno";               Expect = { param($r) $r.success } }
    @{ Name = "q type-info";        Path = "/api/pdms/type-info?refno=$Refno";     Expect = { param($r) $r.success } }
    @{ Name = "q children";         Path = "/api/pdms/children?refno=$Refno";      Expect = { param($r) $r.success } }
)

# ... 循环、PASS/FAIL 汇总、非零退出码 ...
```

**好处**：

- 任何人改动 `web_api/` 后，一条命令即可复核；CI 也可挂上
- 未来新增控制台命令 → 加一个 case 就行
- 与前端耦合低，不依赖 plant3d-web 启动

**扩展（可选）**：

- 补 `plant3d-web` 侧 Playwright 脚本：真打开 Console 发 `= <refno>` + `q pos`，验证日志内容。这条在 `plant3d-web/docs/plans/` 下另起一个方案，范围更大。

**完成判定**：

- 脚本在本地和部署机都能跑通，6 条 case 全 PASS
- 挑一个已知坏 refno，脚本能忠实报告 FAIL

---

### M5 · 启动期路由清单打印（0.5 天）

**问题**：

web_server 启动时 stdout 里没有"已注册路由"这类信息。本次调试里，要区分"404 是路由未注册 vs 处理器返回 404"只能靠 grep 源码。

**方案**：

在 `web_server/mod.rs` 的 `start_web_server_with_config()` 里，构造完最终 `app: Router` 后（`.layer(CorsLayer::...)` 之前或之后），利用 Axum 的 `Router::nest_service` / `RouterIntoMakeService` 不直接暴露路由表的情况，用两种做法之一：

1. **在装配聚合函数里记录元信息**（M2 完成后最自然）：
   - `WebApiAssembly` 增加 `known_paths: Vec<String>` 字段，装配时把每个 `.route(path, ..)` 的 path 同步写入
   - 启动时 `println!("🧭 registered API routes:\n  - {}", known_paths.join("\n  - "));`

2. **独立 debug 钩子**：加一个 `--print-routes` 参数，启动时打印并退出

本轮优先做法 1，零运行时成本，自动随代码演进。

**完成判定**：

- dev build 启动时 stdout 能看到完整路由清单（约 30~60 条）
- release build 可通过环境变量 `AIOS_PRINT_ROUTES=1` 触发，默认关闭，避免噪音

---

## 3. 执行顺序与依赖

```
M1 (commit+deploy) ──→ M2 (aggregator) ──→ M5 (route listing)
                   └─→ M4 (pdms smoke script)
                   └─→ M3 (fetchJson hardening)
```

- M1 必须最先
- M2 与 M3/M4 并行安全
- M5 依赖 M2 的 `WebApiAssembly` 最干净

建议整体排期：1 + 1 + 0.5 + 1 + 0.5 ≈ **4 个工作日**。

---

## 4. 风险 & 规避

| 风险 | 等级 | 规避 |
| --- | --- | --- |
| M2 聚合改动动到了顺序敏感的 merge 顺序，引发路径冲突 | 中 | 对照旧代码逐行补齐；先保持完全同序，只收拢写法 |
| M2 把状态 route 也一并拽进聚合函数，签名爆炸 | 中 | 只聚合无状态 route；状态 route 保留在 `web_server/mod.rs` |
| M3 误判：refno 合法但 404（已被清理）被当作"路由遗漏"提示 | 低 | 靠 `response body 是否空` + `status==404` 联合判断；并附 URL 让用户自行排查 |
| M4 脚本在无 DB 或无 refno 环境下大面积 FAIL | 中 | 脚本支持 `-SkipDataCheck` 模式：只要求路由挂在，不校验 success=true |
| M5 路由清单泄漏给生产访问者 | 低 | 只打到 stdout，不经 HTTP 暴露 |

---

## 5. 验收（所有里程碑完成后）

- [ ] M1：本次修复已合入并上线
- [ ] M2：`src/web_api/mod.rs` 暴露 `assemble_all_web_api_routes`；`web_server/mod.rs` 对应段落压缩 >70%
- [ ] M3：`plant3d-web/src/api/genModelPdmsAttrApi.ts` 对 404 空 body 给出结构化提示
- [ ] M4：`plant-model-gen/scripts/verify_pdms_console_api.ps1` 跑通 6/6 case
- [ ] M5：dev build 启动日志包含"🧭 registered API routes"段落
- [ ] 总体回归：`scripts/verify_fixing_position.ps1` + `scripts/verify_pdms_console_api.ps1` + 前端控制台 `q pos/q ori/q pos wrt owner` 全绿

---

## 6. 与其它规划的关系

- 与 `docs/plans/2026-04-22-异地协同前端独立与API汇总计划.md` 互补：那边关注 API 汇总与跨站同步，这边关注**API 装配机制本身的健壮性**
- 与 `docs/架构文档/世界矩阵计算函数迁移指南.md` 互补：那边关注变换计算路径，这边保证那条路径对外暴露不再丢失
