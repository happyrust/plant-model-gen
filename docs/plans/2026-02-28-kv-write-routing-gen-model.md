# 模型写入路由统一：gen_model-dev 侧 SUL_DB 替换为 model_query_response

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 将 `save_instance_data_optimize` 及周边函数中 18 处硬编码 `SUL_DB` 调用替换为 rs-core 统一写入网关 `model_query_response` / `model_primary_db`，使模型数据写入支持 `surreal_only / dual / kv_only` 三种模式。

**Architecture:** rs-core 已提供完整基础设施（`ModelWriteMode` 枚举、`model_query_response()` 路由、`KV_DB` 连接、`init_model_tables()` 双写支持）。本次改造仅在 gen_model-dev 侧，将 `pdms_inst.rs` 和 `utils.rs` 中绕过路由的 `SUL_DB` 硬编码调用统一改为走路由。新增 `--model-write-mode` CLI 参数覆盖配置文件。

**Tech Stack:** Rust, aios_core (rs-core), SurrealDB (WS), clap (CLI), TOML (config)

**前置条件:** rs-core 侧 `ModelWriteMode`、`model_query_response`、`model_primary_db`、`connect_model_kv`、`set_model_write_mode` 均已实现（参见 `docs/plans/2026-02-25-surrealkv-model-write-separation.md`）。

---

## SUL_DB 调用完整盘点

### 写入调用（→ `model_query_response`）

| # | 行号 | 函数 | SQL 操作 |
|---|------|------|----------|
| W1 | 53 | `save_tubi_info_batch_with_replace` | INSERT IGNORE tubi_info |
| W2 | 73 | `delete_inst_relate_by_in` | DELETE inst_relate |
| W3 | 92 | `delete_geo_relate_by_inst_info_ids` | DELETE geo_relate |
| W4 | 116 | `delete_boolean_relations_by_carriers` | DELETE neg_relate |
| W5 | 120 | `delete_boolean_relations_by_carriers` | DELETE ngmr_relate |
| W6 | 137 | `delete_inst_relate_bool_records` | DELETE inst_relate_bool |
| W7 | 196 | `delete_inst_geo_by_hashes` | DELETE inst_geo |
| W8 | 980 | `TransactionBatcher::flush` | 核心批量写入（事务块） |
| W9 | 1006 | `TransactionBatcher::flush` | REMOVE/DEFINE INDEX（修复） |
| W10 | 1137 | `save_tubi_info_batch` | INSERT tubi_info |
| W11 | 1278 | `reconcile_missing_neg_relate` | INSERT RELATION neg_relate |

### 查询调用（→ `model_primary_db()`）

| # | 行号 | 函数 | SQL 操作 |
|---|------|------|----------|
| R1 | 731 | `save_instance_data_optimize` | SELECT COUNT inst_relate（校验） |
| R2 | 1170 | `query_existing_tubi_info_ids` | SELECT VALUE id FROM tubi_info |
| R3 | 1211 | `reconcile_missing_neg_relate` | SELECT FROM geo_relate |
| R4 | 1245 | `reconcile_missing_neg_relate` | SELECT FROM neg_relate |
| R5 | 1357 | `load_pe_spec_values` | SELECT FROM pe |
| R6 | 1410 | `load_pe_dbnum_sesno` | SELECT FROM pe |
| R7 | 1443 | `load_ses_date` | SELECT FROM ses |

### utils.rs 中的写入

| # | 行号 | 函数 | 当前模式 |
|---|------|------|----------|
| U1 | 183 | `save_inst_relate_bool` | `SUL_DB.query()` + `kv_dual_write()` |
| U2 | ~210 | `save_inst_relate_cata_bool` | `SUL_DB.query()` + `kv_dual_write()` |

---

## Task 1: 新增 `--model-write-mode` CLI 参数

**Files:**
- Modify: `src/main.rs:560` (CLI 定义区) 和 `src/main.rs:1297` (参数提取区)
- Modify: `db_options/DbOption.toml`

**Step 1: 在 CLI 定义中添加参数**

在 `--defer-db-write` 参数定义之后（约第 564 行后）添加：

```rust
.arg(
    Arg::new("model-write-mode")
        .long("model-write-mode")
        .help("Model write target: surreal_only (default), dual (SUL_DB+KV_DB), kv_only (KV_DB only)")
        .value_name("MODE")
        .value_parser(["surreal_only", "dual", "kv_only"]),
)
```

**Step 2: 在参数提取区处理该参数**

在 `defer_db_write` 参数提取之后（约第 1301 行后）添加：

```rust
if let Some(mode_str) = matches.get_one::<String>("model-write-mode") {
    println!("🔧 CLI 覆盖 model_write_mode = {}", mode_str);
    let mode = aios_core::ModelWriteMode::parse(mode_str)
        .unwrap_or_else(|| panic!("无效的 model-write-mode: {}", mode_str));
    aios_core::set_model_write_mode(mode);
}
```

**Step 3: 确认 DbOption.toml 中有 model_write_mode 字段**

在 `db_options/DbOption.toml` 中添加（如果不存在）：

```toml
# 模型写入模式: surreal_only / dual / kv_only
# model_write_mode = "surreal_only"
```

注意：默认注释掉，表示使用 rs-core 默认值 `surreal_only`。CLI 参数 `--model-write-mode` 可覆盖。

**Step 4: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

Expected: 编译通过，无警告。

**Step 5: Commit**

```bash
git add src/main.rs db_options/DbOption.toml
git commit -m "feat: add --model-write-mode CLI parameter for KV write routing"
```

---

## Task 2: 替换辅助删除函数中的 SUL_DB 写入（W2-W7）

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs:53-196`

**Step 1: 添加 import**

在文件头部 `use aios_core::{SUL_DB, ...}` 行中追加导入：

```rust
use aios_core::model_query_response;
```

如果 `model_query_response` 不在 `aios_core` 的 prelude 中，使用完整路径：

```rust
use aios_core::rs_surreal::model_query_response;
```

**Step 2: 替换 6 个辅助函数中的写入调用**

逐一替换以下函数内的 `SUL_DB.query_response(&sql)` 为 `model_query_response(&sql)`：

**W2 — 行 73: `delete_inst_relate_by_in`**
```rust
// 替换前:
SUL_DB.query_response(&sql).await?;
// 替换后:
model_query_response(&sql).await?;
```

**W3 — 行 92: `delete_geo_relate_by_inst_info_ids`**
```rust
// 替换前:
SUL_DB.query_response(&sql).await?;
// 替换后:
model_query_response(&sql).await?;
```

**W4 — 行 116: `delete_boolean_relations_by_carriers` (neg_sql)**
```rust
// 替换前:
SUL_DB.query_response(&neg_sql).await?;
// 替换后:
model_query_response(&neg_sql).await?;
```

**W5 — 行 120: `delete_boolean_relations_by_carriers` (ngmr_sql)**
```rust
// 替换前:
SUL_DB.query_response(&ngmr_sql).await?;
// 替换后:
model_query_response(&ngmr_sql).await?;
```

**W6 — 行 137: `delete_inst_relate_bool_records`**
```rust
// 替换前:
SUL_DB.query_response(&sql).await?;
// 替换后:
model_query_response(&sql).await?;
```

**W7 — 行 196: `delete_inst_geo_by_hashes`**
```rust
// 替换前:
SUL_DB.query_response(&sql).await?;
// 替换后:
model_query_response(&sql).await?;
```

**Step 3: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

注意：`model_query_response` 返回 `anyhow::Result<Response>` 而 `SUL_DB.query_response` 返回 `Result<Response, surrealdb::Error>`。由于原代码用 `?` 传播且上层返回 `anyhow::Result`，类型应当兼容。如果编译报错 `surrealdb::Error` 和 `anyhow::Error` 不兼容，需要在原来的 `?` 后面移除 `.map_err(...)` 转换（如果有的话）。

**Step 4: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs
git commit -m "refactor: route auxiliary delete functions through model_query_response"
```

---

## Task 3: 替换 `save_tubi_info_batch_with_replace` 写入（W1）

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs:53`

**Step 1: 替换写入调用**

```rust
// 替换前（行 53）:
SUL_DB.query_response(&sql)
    .await
    .with_context(|| format!("写入 tubi_info 失败 (insert ignore): {}", written))?;

// 替换后:
model_query_response(&sql)
    .await
    .with_context(|| format!("写入 tubi_info 失败 (insert ignore): {}", written))?;
```

**Step 2: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

**Step 3: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs
git commit -m "refactor: route save_tubi_info_batch_with_replace through model_query_response"
```

---

## Task 4: 替换 TransactionBatcher 核心写入（W8, W9）— 最关键

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs:980,1006`

这是模型数据最大流量的写入点。9 个 batcher（geo/neg/ngmr/inst_info/inst_relate/aabb/inst_aabb/transform/vec3）全部通过此处写入。

**Step 1: 替换主写入点（行 980）**

```rust
// 替换前（行 980）:
match SUL_DB.query_response(query.clone()).await {
    Ok(mut resp) => take_all_results_or_err!(resp),
    Err(err) => Err(anyhow::Error::from(err)),
}

// 替换后:
match model_query_response(&query).await {
    Ok(mut resp) => take_all_results_or_err!(resp),
    Err(err) => Err(err),  // model_query_response 已返回 anyhow::Error
}
```

关键变化：
- `query.clone()` → `&query`（`model_query_response` 接受 `&str`）
- `Err(anyhow::Error::from(err))` → `Err(err)`（已经是 `anyhow::Error`）

**Step 2: 替换索引修复点（行 1006）**

```rust
// 替换前（行 1006）:
let _ = SUL_DB.query_response(repair_sql).await;

// 替换后:
let _ = model_query_response(repair_sql).await;
```

注意：`repair_sql` 如果是 `String` 类型，需要 `&repair_sql`。

**Step 3: 验证 `is_tx_conflict` 函数的错误匹配**

行 960-964 的 `is_tx_conflict()` 函数接收 `&anyhow::Error`，检查错误消息中是否包含 `"transaction conflict"` 等字符串。确认 `model_query_response` 返回的错误信息保持不变（因为底层仍调用 `Surreal.query()`，错误消息来源相同）。

**Step 4: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

重点关注：
- `query` 变量的类型（`String` vs `&str`）— 可能需要 `&query` 或 `query.as_str()`
- `repair_sql` 变量的类型 — 可能需要 `&repair_sql`
- `take_all_results_or_err!` 宏对 `resp` 类型的要求

**Step 5: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs
git commit -m "refactor: route TransactionBatcher flush through model_query_response

This is the core write path for all 9 batchers (geo, neg, ngmr,
inst_info, inst_relate, aabb, inst_aabb, transform, vec3)."
```

---

## Task 5: 替换 `save_tubi_info_batch` 和 `reconcile_missing_neg_relate` 写入（W10, W11）

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs:1137,1278`

**Step 1: 替换 `save_tubi_info_batch` 写入（行 1137）**

```rust
// 替换前:
SUL_DB.query_response(&sql).await?;
// 替换后:
model_query_response(&sql).await?;
```

**Step 2: 替换 `reconcile_missing_neg_relate` 写入（行 1278）**

```rust
// 替换前:
SUL_DB.query_response(&sql).await?;
// 替换后:
model_query_response(&sql).await?;
```

**Step 3: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

**Step 4: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs
git commit -m "refactor: route tubi_info_batch and reconcile_neg_relate writes through model_query_response"
```

---

## Task 6: 替换查询调用为 `model_primary_db()`（R1-R7）

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs:731,1170,1211,1245,1357,1410,1443`

**Step 1: 添加 import**

```rust
use aios_core::rs_surreal::model_primary_db;
```

**Step 2: 替换 7 处查询调用**

`model_primary_db()` 返回 `&'static Surreal<Any>`，与 `SUL_DB` 类型完全相同，可直接调用 `.query_response()` 和 `.query_take()`。

**R1 — 行 731: verify_sql**
```rust
// 替换前:
match SUL_DB.query_response(&verify_sql).await {
// 替换后:
match model_primary_db().query_response(&verify_sql).await {
```

**R2 — 行 1170: query_take**
```rust
// 替换前:
let result: Vec<String> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
// 替换后:
let result: Vec<String> = model_primary_db().query_take(&sql, 0).await.unwrap_or_default();
```

**R3 — 行 1211: reconcile SELECT geo_relate**
```rust
// 替换前:
let mut response = SUL_DB.query_response(&sql).await?;
// 替换后:
let mut response = model_primary_db().query_response(&sql).await?;
```

**R4 — 行 1245: reconcile SELECT neg_relate**
```rust
// 替换前:
let mut check_resp = SUL_DB.query_response(&check_sql).await?;
// 替换后:
let mut check_resp = model_primary_db().query_response(&check_sql).await?;
```

**R5 — 行 1357: load_pe_spec_values**
```rust
// 替换前:
match SUL_DB.query_response(&sql).await {
// 替换后:
match model_primary_db().query_response(&sql).await {
```

**R6 — 行 1410: load_pe_dbnum_sesno**
```rust
// 替换前:
match SUL_DB.query_response(&sql).await {
// 替换后:
match model_primary_db().query_response(&sql).await {
```

**R7 — 行 1443: load_ses_date**
```rust
// 替换前:
match SUL_DB.query_response(&sql).await {
// 替换后:
match model_primary_db().query_response(&sql).await {
```

**关键说明：** R5/R6/R7 查询的是 `pe` 和 `ses` 表——这些是**输入数据**表，不是模型输出表。在 `kv_only` 模式下 KV_DB 可能没有这些表。因此这三处应该**始终使用 SUL_DB 而非 `model_primary_db()`**。

修正：
- R1-R4：使用 `model_primary_db()`（查询的是模型输出表 inst_relate/tubi_info/geo_relate/neg_relate）
- **R5/R6/R7：保持 `SUL_DB`**（查询的是输入数据表 pe/ses）

**Step 3: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

`model_primary_db()` 返回类型与 `SUL_DB` 完全相同，不应有编译问题。

**Step 4: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs
git commit -m "refactor: route model-table reads through model_primary_db, keep pe/ses on SUL_DB"
```

---

## Task 7: 统一 `utils.rs` 中的布尔结果写入（U1, U2）

**Files:**
- Modify: `src/fast_model/utils.rs:167-230`

**Step 1: 替换 `save_inst_relate_bool`（行 183/191）**

```rust
// 替换前:
if let Err(e) = SUL_DB.query(&sql).await {
    // ...error handling...
    anyhow::bail!("save_inst_relate_bool 失败: refno={refno} err={e}");
}
aios_core::kv_dual_write(&sql).await;  // 手动双写

// 替换后:
if let Err(e) = model_query_response(&sql).await {
    // ...error handling...
    anyhow::bail!("save_inst_relate_bool 失败: refno={refno} err={e}");
}
// kv_dual_write 不再需要 — model_query_response 已内置双写路由
```

**Step 2: 替换 `save_inst_relate_cata_bool`（类似模式）**

同样将 `SUL_DB.query(&sql)` 改为 `model_query_response(&sql)`，删除 `kv_dual_write` 调用。

**Step 3: 添加 import**

在 `utils.rs` 头部添加：

```rust
use aios_core::rs_surreal::model_query_response;
```

如果原来已有 `use aios_core::SUL_DB;`，且本文件中再无其他 `SUL_DB` 使用，则移除该 import。

**Step 4: 编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

注意：`SUL_DB.query()` 返回 `Result<Response, surrealdb::Error>`，而 `model_query_response` 返回 `anyhow::Result<Response>`。错误处理从 `if let Err(e)` 模式不受影响（`e` 的类型变了但 `bail!` 只要 `Display` 即可）。

**Step 5: Commit**

```bash
git add src/fast_model/utils.rs
git commit -m "refactor: unify bool result writes through model_query_response, remove manual kv_dual_write"
```

---

## Task 8: 清理残留 — 确认 pdms_inst.rs 中无剩余 SUL_DB 引用

**Files:**
- Modify: `src/fast_model/gen_model/pdms_inst.rs` (import 清理)

**Step 1: 搜索残留引用**

```bash
grep -n "SUL_DB" src/fast_model/gen_model/pdms_inst.rs
```

Expected: 仅剩下 R5/R6/R7 的 3 处（pe/ses 查询，有意保留），以及可能的 import 行。

**Step 2: 精简 import**

如果 `SUL_DB` 在 pdms_inst.rs 中仍被 R5/R6/R7 使用，则保留 import。否则从 `use aios_core::{SUL_DB, ...}` 中移除 `SUL_DB`。

**Step 3: 搜索 utils.rs 残留**

```bash
grep -n "SUL_DB\|kv_dual_write" src/fast_model/utils.rs
```

Expected: 无残留（除非该文件中有其他非模型写入的 `SUL_DB` 用途）。

**Step 4: 完整编译检查**

```powershell
$env:CARGO_TARGET_DIR="target-check"; cargo check --bin web_server --features web_server
```

**Step 5: Commit**

```bash
git add src/fast_model/gen_model/pdms_inst.rs src/fast_model/utils.rs
git commit -m "chore: clean up unused SUL_DB imports after model write routing migration"
```

---

## Task 9: 集成验证

**Step 1: 回归测试 — `surreal_only` 模式**

这是默认模式，行为应与改造前完全一致：

```powershell
# debug 编译运行
cargo run -- --dbnum 7997 --export-obj --regen-model --verbose
```

验证：
- 模型生成成功，无报错
- inst_geo/inst_relate/geo_relate 数据写入 SUL_DB 正常
- 无 KV_DB 相关日志输出

**Step 2: 双写测试 — `dual` 模式**

前置条件：启动 SurrealKV 实例（`kv_ip:kv_port`）。

```powershell
cargo run -- --dbnum 7997 --export-obj --regen-model --model-write-mode dual --verbose
```

验证：
- 模型生成成功
- 日志中出现 `[MODEL_WRITE_MIRROR]` 相关输出（如果 KV 未启动则是警告，不影响主库写入）
- SUL_DB 数据完整

**Step 3: KV-only 测试 — `kv_only` 模式**

前置条件：启动 SurrealKV 实例。

```powershell
cargo run -- --dbnum 7997 --export-obj --regen-model --model-write-mode kv_only --verbose
```

验证：
- 模型生成成功
- 数据写入 KV_DB
- pe/ses 查询仍走 SUL_DB（不报错）

---

## 风险与注意事项

### 1. TransactionBatcher 的事务重试在双写时的行为

`model_query_response` 在 `Dual` 模式下：先写主库获取响应，再异步写镜像库（失败仅打印警告）。事务冲突检测 `is_tx_conflict()` 基于主库响应的错误消息，不受镜像写入影响。**风险：低。**

### 2. `model_query_response` 与 `SUL_DB.query_response` 的返回类型差异

- `SUL_DB.query_response(&str)` → `Result<Response, surrealdb::Error>`
- `model_query_response(&str)` → `anyhow::Result<Response>`

所有调用点已通过 `?` 或 `match` 处理，`anyhow::Error` 实现了 `From<surrealdb::Error>`，兼容性无问题。仅需注意 TransactionBatcher 中 `Err(anyhow::Error::from(err))` 可简化为 `Err(err)`。

### 3. pe/ses 表查询必须保持在 SUL_DB

R5/R6/R7 查询的 `pe` 和 `ses` 表是**输入数据表**（由 PDMS 数据导入工具填充），不属于模型输出数据。在 `kv_only` 模式下 KV_DB 不会有这些表。因此这三处必须始终使用 `SUL_DB`，**不能**替换为 `model_primary_db()`。

### 4. defer_db_write 路径不受影响

`defer_db_write=true` 时走 `save_instance_data_to_sql_file()` 路径，完全不进入 `save_instance_data_optimize()`，因此本次改造对 defer 模式零影响。
