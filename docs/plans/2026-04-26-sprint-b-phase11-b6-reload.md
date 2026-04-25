# Sprint B · Phase 11 · B6 site-config reload 最小可用版（2026-04-26）

> 上游：
> - Sprint B 主计划：`docs/plans/2026-04-26-sprint-b-plan.md`（§1.B6）
> - 跨仓 PRD：`../plant-collab-monitor/docs/prd/2026-04-26-remote-site-prd.md`
> - 涉及 stub 位置：`src/web_server/site_config_handlers.rs:388-406`

---

## 0. 背景

Sprint B 主计划 §1.B6 把 reload 描述为「重新加载 `aios_core::set_db_option_from_file()`」。本会话核查 `rs-core/src/lib.rs:166` 后发现：

- `get_db_option()` 实现为 `OnceCell::get_or_init`，**全局静态、不可变**
- `aios_core::set_db_option_from_file` **不存在**
- 想真正"热替换" `DbOption` 必须改 `OnceCell` → `RwLock<Arc<DbOption>>`，**属于 rs-core 跨仓改动**

因此本 Phase 11 落实"最小可用 / 0.5d"档：**字段差异检测 + 热/静态分类 + 用户告知**，不动 rs-core。

真热重载（方案 C）留待独立 rs-core 改动会话。

---

## 1. 改造范围

### 1.1 单文件改动

`src/web_server/site_config_handlers.rs::reload_site_config`（行 388-406）

### 1.2 不改动

- ❌ `rs-core/src/lib.rs::get_db_option`（OnceCell 不动）
- ❌ `rs-core/src/options.rs::DbOption`（结构体不动）
- ❌ `main.rs` / `AppState`（不引入 shutdown_tx，留 Phase 10）
- ❌ 其他 handler（mqtt / sync / remote-sync 全部不动）

---

## 2. 实现方案（最小可用 · A 方案）

### 2.1 流程

```
POST /api/site-config/reload
  │
  ├─ 1. 读 ${DB_OPTION_FILE:-db_options/DbOption}.toml 文件内容
  │     失败 → 返回 error
  │
  ├─ 2. 解析为 DbOption 结构体
  │     失败 → 返回 error，提示具体语法错误
  │
  ├─ 3. 取当前内存中 get_db_option() 引用
  │
  ├─ 4. 字段级 diff（serde_json::to_value 对比 key-by-key）
  │     ├─ hot_changed_keys：列入 HOT_RELOADABLE_KEYS 白名单且值变化
  │     ├─ static_changed_keys：未列入白名单且值变化
  │     └─ 无变化 → no_change
  │
  └─ 5. 返回 JSON
        {
          status: "success",
          hot_changed_keys: [...],
          static_changed_keys: [...],
          requires_restart: bool,        // = !static_changed_keys.is_empty()
          actions: ["log_only" | "manual_restart_required" | "no_change"],
          message: "<用户友好提示>"
        }
```

### 2.2 HOT_RELOADABLE_KEYS 白名单

参考 `DbOption` 字段语义，**理论上可热改的**（不影响已建立的连接 / 已加载的索引 / 已订阅的 MQTT）：

```rust
const HOT_RELOADABLE_KEYS: &[&str] = &[
    "enable_log",          // 日志开关
    "mesh_tol_ratio",      // mesh 精度
    "gen_model",           // gen 模式开关
    "gen_spatial_tree",
    "load_spatial_tree",
    "apply_boolean_operation",
    "build_cate_relate",
    "sync_chunk_size",     // 下次同步任务读
    "parse_channel_capacity",
    "parse_mode",
    "incr_sync",
    "total_sync",
];
```

> 注：白名单标记为 "理论上可热改"。**本 Phase 不真正应用这些字段**（OnceCell 不可变），仅用于在响应中分类提示用户「这些不一定要重启」vs「这些一定要重启」。后续 rs-core 改动后即可一键升级为真热加载。

### 2.3 字段比对实现

```rust
fn diff_db_option(current: &DbOption, new: &DbOption) -> (Vec<String>, Vec<String>) {
    let cur_v = serde_json::to_value(current).unwrap_or(json!({}));
    let new_v = serde_json::to_value(new).unwrap_or(json!({}));
    let cur_obj = cur_v.as_object().cloned().unwrap_or_default();
    let new_obj = new_v.as_object().cloned().unwrap_or_default();

    let mut hot = Vec::new();
    let mut stat = Vec::new();
    let all_keys: std::collections::BTreeSet<_> =
        cur_obj.keys().chain(new_obj.keys()).cloned().collect();

    for k in all_keys {
        if cur_obj.get(&k) != new_obj.get(&k) {
            if HOT_RELOADABLE_KEYS.contains(&k.as_str()) {
                hot.push(k);
            } else {
                stat.push(k);
            }
        }
    }
    (hot, stat)
}
```

---

## 3. 验收

### 3.1 编译

- `cargo check --features web_server` 0 errors

### 3.2 行为

| 场景 | 期望响应 |
|------|---------|
| 文件不存在 | `{ status: "error", message: "DbOption.toml 不存在: <path>" }` |
| 文件存在但 toml 解析失败 | `{ status: "error", message: "DbOption.toml 解析失败: <serde_err>" }` |
| 解析成功但与当前完全一致 | `{ status: "success", hot_changed_keys: [], static_changed_keys: [], requires_restart: false, actions: ["no_change"] }` |
| 改了 `enable_log` | `hot_changed_keys: ["enable_log"], requires_restart: false, actions: ["log_only"]` |
| 改了 `mqtt_host` | `static_changed_keys: ["mqtt_host"], requires_restart: true, actions: ["manual_restart_required"]` |
| 同时改 hot+static | 两个数组都填，`requires_restart: true` |

### 3.3 smoke 脚本验证（可选，不阻塞 commit）

`shells/smoke-collab-api.sh` 已有 `[1/5]` 站点配置块，无需新增 endpoint。手动验证用：

```bash
curl -X POST http://127.0.0.1:3100/api/site-config/reload | jq
```

---

## 4. 风险

| 风险 | 等级 | 缓解 |
|------|------|------|
| `serde_json::to_value(DbOption)` 在某些字段上失败（如 PathBuf） | 🟢 低 | unwrap_or(json!({})) 兜底，diff 退化为"全部静态变更" |
| HOT_RELOADABLE_KEYS 白名单与未来 DbOption 字段失同步 | 🟡 中 | 在常量旁加注释 "需与 DbOption 字段同步审查"；后续 rs-core 重构时统一修订 |
| 用户误以为返回 `hot_changed_keys` = 已生效 | 🟡 中 | message 字段明确写"配置变更已检测，但当前版本仍需手动重启才能生效" |
| 文件路径与 `get_config_file_name()` 不一致 | 🟢 低 | 复用 rs-core 私有函数逻辑：读 `DB_OPTION_FILE` 环境变量，默认 `db_options/DbOption` |

---

## 5. 时间线

| 步骤 | 估时 |
|------|------|
| 11.1 编辑 site_config_handlers.rs reload_site_config | 30 min |
| 11.2 cargo check --features web_server（增量） | 1 min |
| 11.3 git commit Phase 11 | 5 min |
| 11.4 sprint-b-plan 标记 Phase 11 完成 | 5 min |

**Phase 11 总估时**：~45 min

---

## 6. 后续迁移路径（独立会话）

当用户有空做 rs-core 改动时，本 Phase 11 可一键升级为真热重载：

```rust
// rs-core/src/lib.rs
pub static DB_OPTION: Lazy<RwLock<Arc<DbOption>>> = Lazy::new(|| {
    RwLock::new(Arc::new(load_from_config_file()))
});
pub fn get_db_option() -> Arc<DbOption> {
    DB_OPTION.read().unwrap().clone()
}
pub fn set_db_option_from_file() -> Result<()> {
    let new = load_from_config_file()?;
    *DB_OPTION.write().unwrap() = Arc::new(new);
    Ok(())
}
```

然后 `reload_site_config` 在 hot_changed_keys 非空时调 `aios_core::set_db_option_from_file()`，把 `actions` 升级为 `["hot_reloaded"]`。所有业务方读 `get_db_option()` 时拿到 `Arc<DbOption>`，并发安全。

**风险评估**：跨仓改动，影响所有依赖 rs-core 的 binary（plant3d-web / pdms / mes 等），需要全仓回归。

---

## 7. 不做

- ❌ 真正热应用 `mesh_tol_ratio` 等字段到运行时 mesh 计算
- ❌ 触发 mqtt 订阅 / sync runtime 重启
- ❌ 任何 main.rs 改动（属 Phase 10 = B5）
- ❌ rs-core 改动
