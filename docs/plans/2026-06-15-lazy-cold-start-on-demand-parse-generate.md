# Lazy Cold Start + 按需解析/生成实施计划

> 日期：2026-06-15
> 目标仓库：`D:/work/plant-code/plant-model-gen-cata-closure`
> 关联计划：`docs/plans/2026-06-09-on-demand-cata-refno-closure-parsing.md`

## 0. 背景与目标

现有站点部署更接近 eager 模式：站点可用前通常需要提前解析设计库/元件库并生成模型。大工程下，尤其是 CATA 全量解析，会让启动和首次可用时间过长。

本计划新增 lazy runtime mode：

1. 站点 cold start 只做可定位、可调度的最小准备：SYST/MBD 摘要、MBD 下 DESI/CATA 成员发现、db_index/sesno 定位。
2. 用户进入模型树页面时，后台解析当前 MBD 下 DESI 成员到现有 SurrealDB；解析完成后前端再用现有 tree/query API 拉树。
3. 用户请求显示某个 root/refno 子树模型时，后台先检查是否已按当前版本生成；未生成或 stale 时，按 root/refno 子树触发 partial CATA 解析和模型生成。
4. SurrealDB 仍是唯一业务数据源；lazy API 只做任务编排和状态判断，不返回大体量树/模型实体数据。

## 1. 非目标

- 不重建一套 lazy tree storage。
- 不用本地表替代现有 SurrealDB 中的 PE/tree/attrs/model 数据。
- 不在 cold start 解析 DESI/CATA 或生成模型。
- 不在 tree-ready 阶段触发模型生成。
- 不默认 fallback 到整库 CATA。
- 第一版不做视锥自动预生成、复杂取消、子树内部差量拼接。

## 2. 三条红线

1. cold start 不解析 DESI/CATA，不生成模型。
2. tree-ready 只解析 MBD 下 DESI members，不触发模型生成。
3. model-ready 默认只做 partial CATA，不允许偷偷整库 CATA fallback。

## 3. 总体架构

```text
start site (lazy)
  -> parse SYST/MBD summary
  -> build/refresh db_index + latest_sesno
  -> web ready

open model tree page
  -> POST /api/lazy/ensure-tree-ready
  -> parse MBD DESI members into SurrealDB
  -> existing tree API reads SurrealDB

show model(root_refno)
  -> POST /api/lazy/ensure-model-ready
  -> check model_key cache
  -> resolve CATA closure for root/refno subtree
  -> partial parse CATA manifest into SurrealDB
  -> generate model for root/refno subtree
  -> existing model API reads SurrealDB/mesh outputs
```

## 4. Runtime Mode

新增显式模式，默认保持现有 eager 行为：

```toml
runtime_mode = "lazy" # eager | lazy

[lazy]
tree_parse_scope = "mbd_desi_members"
model_generate_scope = "root_subtree"
cata_parse_mode = "partial_manifest"
allow_full_cata_fallback = false
max_concurrent_parse_jobs = 2
max_concurrent_model_jobs = 1
```

lazy 模式下：

- `start_site` 不要求 full parse 已完成。
- `parse_site` 可保留为手动预热树数据。
- `generate_site` 可保留为手动预生成模型。
- 前端实际浏览走 `ensure-tree-ready` / `ensure-model-ready`。

## 5. Cold Start / Bootstrap

启动阶段做：

- 启动/连接 SurrealDB。
- 启动 web_server。
- 启动或准备 parse sidecar。
- 扫描工程根，读取 db 文件头。
- 解析 SYST，按 `mdb_name` 找 MDB。
- 从 MDB `CURD` 成员识别 DESI/CATA 成员库。
- 构建或刷新 db_index：`dbnum -> file_path/db_type/latest_sesno`、`ref0 -> dbnum`、依赖边。
- 写入 lazy bootstrap 状态。

启动阶段不做：

- 不解析 DESI 全库到 SurrealDB。
- 不解析 CATA 全库到 SurrealDB。
- 不跑 CATA closure。
- 不生成模型。
- 不导出 parquet/json/glb。

启动成功定义：

- SYST 可读。
- 目标 MDB 可找到。
- 目标 MDB 至少有一个可用 DESI 成员库。
- db_index 可建立或可部分建立。
- SurrealDB 可连接。

CATA 缺失在 cold start 阶段只产生 warning；模型显示阶段再阻塞并返回明确错误。

## 6. MBD / WORLD 来源

SYST 是 `mdb_name -> DESI/CATA members` 的权威来源。`db_index` 是定位器，不是 MDB 业务语义源。

推荐顺序：

```text
SYST/MDB summary -> db_index prescan -> lazy bootstrap ready
```

如果模型树查询需要 root/world refno：

- 优先在 DESI 解析完成后从 SurrealDB 查询真实 root/world。
- 不把 `dbnum/0` 当作长期契约；临时猜测必须带 `confidence = guessed`，并在 DESI 解析完成后校正。

## 7. ensure-tree-ready

触发时机：用户进入模型树页面后自动触发。

解析粒度：当前 MDB 下的 DESI 成员集合，按现有 SYST/MBD 语法：

```text
SYST -> MDB(name) -> CURD members -> STYP == DESI -> design_dbnums
```

流程：

```text
POST /api/lazy/ensure-tree-ready
  -> refresh MBD DESI member sesnos
  -> compute tree_scope_key
  -> if tree_scope ready: return ready/skipped
  -> else submit parse job for MBD DESI members
  -> parse into existing SurrealDB
  -> verify tree/query data exists
  -> mark ready
```

tree_scope_key：

```text
hash(site_id + mdb_name + sorted(design_dbnum:sesno))
```

M2 lazy tree parse config should set:

```text
manual_db_nums = MBD 下 DESI member dbnums
parse_db_types = ["DESI"]
gen_model = false
gen_mesh = false
auto_parse_related_dbnums = false
cata_partial_parse = false
```

如果现有 parse pipeline 强制 mandatory DICT/GLOB/GLB preparse，需要为 lazy tree mode 提供跳过能力；树浏览不应被 CATA/DICT/GLOB/GLB 阻塞。

## 8. ensure-model-ready

触发时机：前端请求显示某个 root/refno 子树模型。

生成粒度：root/refno 子树。

流程：

```text
POST /api/lazy/ensure-model-ready(root_refno)
  -> ensure-tree-ready if needed
  -> refresh DESI/CATA sesnos
  -> check recent model_state / model_key
  -> if ready and version-matched: return ready/skipped
  -> run CATA closure for root/refno
  -> partial parse CATA manifest into SurrealDB
  -> run model generation for root/refno subtree
  -> verify model data can be read by existing APIs
  -> mark model ready
```

model_key:

```text
hash(
  root_refno,
  include_descendants,
  desi_scope_key,
  cata_manifest_hash,
  dependency_sesnos,
  apply_boolean,
  lod,
  mesh_tol_ratio
)
```

Do not use only these signals as "already generated":

- mesh file exists
- `inst_relate` has rows
- `scene_node.generated = true`

They may belong to old sesno, old CATA manifest, or old generation options.

## 9. Partial CATA 策略

模型生成前必须按需要部分解析 CATA。默认禁止整库 CATA。

流程：

```text
root_refno
  -> collect model CATA seeds
  -> run_cata_closure_pass_for_refnos(root_refno)
  -> cata_closure manifest
  -> partial parse manifest refnos into SurrealDB
  -> generate model
```

复用 `docs/plans/2026-06-09-on-demand-cata-refno-closure-parsing.md` 中的 refno 级闭包设计。该计划解决闭包算法和 CATA 部分解析能力；本计划解决站点 lazy 编排。

partial CATA 写入语义：

- 写入同一个 SurrealDB。
- 使用 upsert/merge。
- 不清空整个 CATA dbnum 的旧数据。
- 不删除其它 root 已经部分解析出来的 CATA refnos。

默认：

```text
allow_full_cata_fallback = false
```

只有管理员显式允许时才能整库 fallback，并必须记录：

```text
fallback_reason
fallback_started_at
fallback_dbnums
fallback_duration_ms
```

partial CATA 失败时，模型生成不得启动；模型状态应为 blocked/failed dependency。

错误类型至少区分：

- `missing_db_mapping`
- `missing_db_file`
- `missing_refno_in_cata`
- `parse_error`

## 10. sesno 版本与缓存命中

缓存失效以 `latest_sesno` 为准。

`db_index` 已保存 `latest_sesno`，但 lazy 模式不能只信旧 SQLite 值。展开树/显示模型前应提供定点能力：

```text
refresh_db_sesno(dbnum)
```

它只定位 db 文件、打开 PdmsIO、读 `get_latest_sesno()`，更新 `db_index.db_file_index.latest_sesno`。不扫 ref0，不解析元素。

tree-ready 判断：

```text
MBD DESI dbnums + latest_sesnos -> tree_scope_key
tree_scope_key ready -> skip parse
```

model-ready 判断：

```text
root_refno
DESI dbnums/sesnos
CATA manifest hash
CATA dependency dbnums/sesnos
generation options
  -> model_key
model_key ready -> skip generation
```

## 11. 状态记录

状态记录用于编排，不替代业务数据。推荐写入现有 SurrealDB 服务。

### LazyBootstrapSummary

```rust
struct LazyBootstrapSummary {
    site_id: String,
    project_name: String,
    mdb_name: String,
    syst_dbnum: Option<u32>,
    syst_sesno: Option<u32>,
    design_members: Vec<LazyDbMember>,
    cata_members: Vec<LazyDbMember>,
    db_index_path: Option<String>,
    status: LazyBootstrapStatus,
    warnings: Vec<String>,
    updated_at: String,
}

struct LazyDbMember {
    dbnum: u32,
    db_type: String,
    file_name: String,
    file_path: String,
    latest_sesno: Option<u32>,
    available: bool,
}
```

### LazyTreeState

```rust
struct LazyTreeState {
    site_id: String,
    mdb_name: String,
    design_dbnums: Vec<u32>,
    design_sesnos: BTreeMap<u32, u32>,
    scope_key: String,
    status: LazyEnsureStatus,
    job_id: Option<String>,
    error: Option<String>,
    updated_at: String,
}
```

### LazyCataClosureState

```rust
struct LazyCataClosureState {
    site_id: String,
    root_refno: String,
    desi_scope_key: String,
    dependency_sesnos: BTreeMap<u32, u32>,
    manifest_hash: Option<String>,
    manifest_path: Option<String>,
    status: LazyEnsureStatus,
    error: Option<String>,
    updated_at: String,
}
```

### LazyModelState

```rust
struct LazyModelState {
    site_id: String,
    root_refno: String,
    model_key: String,
    desi_scope_key: String,
    cata_manifest_hash: Option<String>,
    dependency_sesnos: BTreeMap<u32, u32>,
    generation_options: LazyGenerationOptions,
    status: LazyEnsureStatus,
    phase: Option<LazyModelPhase>,
    job_id: Option<String>,
    error: Option<String>,
    updated_at: String,
}

struct LazyGenerationOptions {
    include_descendants: bool,
    apply_boolean: bool,
    lod: Option<String>,
}
```

Shared status:

```rust
enum LazyEnsureStatus {
    Missing,
    Queued,
    Running,
    Ready,
    Failed,
    Stale,
}

enum LazyModelPhase {
    CheckingCache,
    ResolvingCata,
    ParsingCataPartial,
    GeneratingModel,
    SyncingModelData,
}
```

## 12. API 草案

```text
POST /api/lazy/bootstrap
GET  /api/lazy/bootstrap-state
POST /api/lazy/ensure-tree-ready
GET  /api/lazy/tree-state
POST /api/lazy/ensure-model-ready
GET  /api/lazy/model-state?root_refno=...
GET  /api/lazy/jobs/{job_id}
```

`ensure-tree-ready` response:

```json
{
  "state": "running",
  "phase": "parsing_desi_members",
  "job_id": "...",
  "design_dbnums": [250160, 250161],
  "progress": null
}
```

Cache hit:

```json
{
  "state": "ready",
  "skipped": true,
  "reason": "tree_scope_cache_hit",
  "scope_key": "..."
}
```

`ensure-model-ready` running:

```json
{
  "state": "running",
  "job_id": "...",
  "phase": "parsing_cata_partial",
  "progress": null
}
```

Model cache hit:

```json
{
  "state": "ready",
  "skipped": true,
  "reason": "model_cache_hit",
  "model_key": "..."
}
```

## 13. 任务进度

`ensure-tree-ready` phases:

```text
refresh_sesno
parsing_desi_members
syncing_surrealdb
ready
failed
```

`ensure-model-ready` phases:

```text
checking_model_cache
resolving_cata
parsing_cata_partial
generating_model
syncing_model_data
ready
failed
```

SSE/event format:

```json
{
  "job_id": "...",
  "kind": "tree_ready",
  "root_refno": "24381/145018",
  "phase": "parsing_cata_partial",
  "status": "running",
  "message": "按需解析 CATA 依赖",
  "progress": null
}
```

无法精确进度时不要伪造百分比。

## 14. 并发、重试与取消

并发去重：

- tree 按 `site_id + mdb_name + tree_scope_key` single-flight。
- model 按 `root_refno + model_key` single-flight。
- 相同 key 正在 running 时，后续请求返回同一个 `job_id`。

重试：

- retry tree 先 refresh DESI member sesnos，再重算 tree_scope_key。
- retry model 先 refresh DESI/CATA sesnos，再重算 manifest/model_key。
- key 不变时可增加 attempt；key 变化时创建新 job。

取消：

- 第一版不对普通前端暴露复杂取消。
- 可保留管理员/调试 cancel。
- 取消后不承诺清理已写入 partial data；下次请求按当前 key 重新 ensure。

幂等要求：

- DESI 解析重复写同一 sesno 不应破坏数据。
- partial CATA 重复写同一 manifest 不应清库。
- 同一 model_key 重复生成最终状态一致。

## 15. 前端交互

进入模型树页：

```text
auto POST ensure-tree-ready
show "正在解析模型树"
ready -> call existing tree API
failed -> show retry
```

显示模型：

```text
click show model(root_refno)
POST ensure-model-ready
ready/skipped -> call existing model API
running -> show phases
failed -> show retry and error
```

树解析状态和模型生成状态分开；树可浏览期间，模型生成任务不得阻塞整个页面。

## 16. 复用的现有能力

M1:

- `mdb_candidates`：离线读 SYST，枚举 MDB 和成员 DB。
- `db_index/rebuild`：写 `dbnum/db_type/file_path/latest_sesno/ref0 map/dependency edges`。

M2:

- 复用现有 parse pipeline。
- 用 lazy tree 专用配置解析 MBD 下 DESI members。

M3:

- 复用 `run_cata_closure_pass_for_refnos` / manifest 部分解析能力。
- 复用现有 root/refno 子树模型生成逻辑。
- 建议把现有 stream-generate 内部生成逻辑抽为可调用函数，供旧接口和新 ensure-model-ready 共用。

## 17. 里程碑

### M1 Lazy Bootstrap

交付：

- `runtime_mode = lazy`
- lazy bootstrap API/status
- MBD -> DESI/CATA members summary
- dbnum -> file_path/latest_sesno

不做：

- 不解析 DESI/CATA。
- 不生成模型。

### M2 ensure-tree-ready

交付：

- `POST /api/lazy/ensure-tree-ready`
- MBD DESI members parse into SurrealDB
- tree_scope_key cache hit
- single-flight
- 完成后前端调用现有 tree API

### M3 ensure-model-ready

交付：

- `POST /api/lazy/ensure-model-ready`
- root/refno 子树粒度
- partial CATA closure/parse
- model_key cache hit
- 完成后前端调用现有模型 API

硬约束：

- 默认不整库 CATA fallback。
- tree 展开不触发模型生成。

### M4 前端接入 + HTTP 验收

交付：

- 进入树页自动 ensure-tree-ready。
- tree ready 后拉现有 tree API。
- 点击显示模型自动 ensure-model-ready。
- model ready 后拉现有模型数据。
- 失败可重试。

## 18. HTTP 验收清单

按仓库规则：不创建/运行 `cargo test`；web_server 运行后用 HTTP/POST 验证。

1. lazy 站点 cold start
   - 期望：站点启动成功，未解析 DESI/CATA，未生成模型。
   - 验证：bootstrap_status=ready，parse_status 不阻塞访问。

2. 进入模型树页触发 ensure-tree-ready
   - `POST /api/lazy/ensure-tree-ready`
   - 期望：解析 MBD 下 DESI 成员。
   - 完成后：现有 tree API 能拉到树。

3. 重复进入模型树页
   - 期望：sesno 未变时 `skipped=true`，不重复解析。

4. 显示某 root/refno 模型
   - `POST /api/lazy/ensure-model-ready`
   - 期望：先 partial CATA，再 root/refno 子树模型生成。
   - 完成后：现有模型接口能拉到模型。

5. 重复显示同一 root/refno
   - 期望：model_key 命中，`skipped=true`。

6. CATA 缺失或 partial 失败
   - 期望：不自动整库 CATA fallback，返回 failed + 可解释 error。

不允许行为：

- cold start 解析全部 CATA。
- tree-ready 触发模型生成。
- model-ready partial CATA 失败后默认整库 CATA。
- sesno 未变时重复解析/重复生成。
- ensure 接口直接返回大模型数据。

## 19. 风险与开放问题

| 风险 | 说明 | 缓解 |
|---|---|---|
| partial CATA 写入语义不清 | 若现有 parse pipeline 会清库，不能直接复用 | 必须新增 manifest 部分 upsert/merge 模式 |
| lazy tree parse 被 mandatory preparse 阻塞 | 现有 parse plan 可能强制 DICT/GLOB/GLB | lazy tree mode 下跳过非 DESI 前置 |
| model_key 与现有模型读取接口未强绑定 | 前端可能读到旧模型数据 | 第一版 ready 前后做 sesno 校验；后续给模型读取接口带 `model_key` |
| db_index 全量预扫仍慢 | cold start 可能被 ref0 map 拖慢 | 后续拆 bootstrap index 与 background index |
| 取消半写入 | 中途取消会留下 partial data | 第一版只做重试，取消仅管理员调试 |

## 20. 推荐执行顺序

1. 固化 M1 数据模型和 lazy runtime mode。
2. 接入 `mdb_candidates` 与 `db_index/rebuild`，写 bootstrap summary。
3. 实现 `ensure-tree-ready`，只解析 MBD 下 DESI members。
4. 抽取 root/refno 子树模型生成内部函数。
5. 接入 CATA closure manifest 与 partial parse。
6. 实现 `ensure-model-ready` 与 model_key cache hit。
7. 前端接入树页和模型显示。
8. 运行 web_server，通过 HTTP/POST 完成验收。
