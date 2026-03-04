# BRAN 24381/145018 Debug 模式生成与分析指南

## ✅ 验证结果摘要（2026-03-04 运行）

| 验证项 | 状态 | 说明 |
|--------|------|------|
| BRAN 模型生成 | ✅ | 18 个节点收集，几何体正常生成 |
| ELBO 包含 | ✅ | 24381_145019、24381_145021、24381_145023 等 6+ 个 ELBO |
| tubi 连接 ELBO | ✅ | 11 条 tubi_relate 已写入，直段链正确连接 ELBO |
| OBJ 导出 | ✅ | output/AvevaMarineSample/screenshots/obj-cache/ 含 24381_145018.obj 及子件 |

**tubi 直段链**（leave → arrive）：
```
24381_145018(BRAN) → 24381_145019(ELBO) → 24381_145021(ELBO) → 24381_145023(ELBO) 
→ 24381_145025(ELBO) → 24381_145026 → 24381_145028 → 24381_145029 → 24381_145031(ELBO) 
→ 24381_145032 → 24381_145033(ELBO) → 24381_145035
```

---

## 1. 执行命令

```bash
cd d:\work\plant-code\plant-model-gen
cargo run --bin aios-database -- --debug-model 24381/145018 --regen-model --export-obj -v
```

### 参数说明

| 参数 | 作用 |
|------|------|
| `--debug-model 24381/145018` | 启用调试模式，指定目标 BRAN 的 refno；会调用 `set_debug_model_enabled(true)`，触发 `debug_model!` / `debug_model_debug!` 等宏的输出 |
| `--regen-model` | 强制重建几何模型（replace_mesh + apply_boolean_operation），确保 CatePos 等正确生成 |
| `--export-obj` | 生成后导出 OBJ |
| `-v` | 详细日志输出 |

### refno 与 dbnum 说明

- `24381/145018`：BRAN 的 refno（可写作 `24381_145018`）
- `24381` 是 ref0，**不是** dbnum
- 对应 dbnum 需从 `output/<project>/scene_tree/db_meta_info.json` 的 `ref0_to_dbnum` 查（如 24381→7997）

---

## 2. 数据流与验证点

### 2.1 BRAN → 子件（含 ELBO）

```
BRAN (24381_145018)
 ├─ ELBO (弯头)
 ├─ BEND / TUBI / VALV 等
 └─ ...
```

- BRAN 流程会执行 `gen_cata_instances`，将 ELBO/BEND 等 CATE 写入 `inst_relate` 和 `inst_relate_bool`
- 几何体来自 catalogue，写入 `geo_relate`（`geo_type IN ['Pos','DesiPos','CatePos']`）

### 2.2 tubi_relate 与 ELBO 连接

`tubi_relate` 表结构：

- **ID**：`tubi_relate:[pe:⟨bran_refno⟩, index]`
- **关系**：`leave -> tubi_relate:[bran, idx] -> arrive`
- `leave`：起点（可为 ELBO/BEND/TUBI）
- `arrive`：终点（下一构件）
- 管道直段表示从 leave 到 arrive 的线段

ELBO 作为管道连接点，会出现在 tubi_relate 的 leave/arrive 中，形成：`TUBI->ELBO->TUBI` 之类的连通。

### 2.3 调试输出关键字

启用 `--debug-model` 后，关注以下日志：

| 关键字 | 含义 |
|--------|------|
| `[BRAN_TUBI]` | tubi 生成与 tubi_relate 写入 |
| `准备写入 N 条 tubi_relate` | 写入 tubi_relate 数量 |
| `子件 ... -> arrive_type=ELBO` | 子件中包含 ELBO |
| `直段 ... -> ...` | 管道直段（leave->arrive） |
| `axis_map` | 轴线信息，用于 tubi 几何 |

---

## 3. 验证步骤

### 3.1 检查生成是否成功

- 控制台无 panic、无严重错误
- 有 `✅ 模型重新生成完成`、OBJ 导出成功等提示

### 3.2 检查 ELBO 是否在子树中

```bash
# 导出 Parquet 并检查 24381_145019（ELBO）是否存在
cargo run -- --export-dbnum-instances-parquet --dbnum 7997 --root-refno 24381_145018 -v
```

- 输出中应有 `24381_145019` 或类似 ELBO refno
- 若缺失，参考 `docs/DEBUG_24381_145019_PARQUET.md` 排查

### 3.3 检查 tubi_relate

在 Surrealist 或 SurrealQL 中：

```sql
-- 查询 BRAN 24381_145018 的 tubi_relate
SELECT * FROM tubi_relate:[pe:⟨24381_145018⟩, 0]..[pe:⟨24381_145018⟩, ..];

-- 检查 leave 中是否有 ELBO
SELECT in.noun, in, out FROM tubi_relate WHERE id()[0] = pe:⟨24381_145018⟩;
```

- `leave`（in）或 `arrive`（out）中应出现 ELBO 的 pe id

### 3.4 检查 cache / 导出目录

- `output/<project>/model_cache/`：inst_tubi_map 等
- `output/<project>/meshes/`：mesh 文件
- OBJ 导出目录：应包含对应模型文件

---

## 4. 常见问题

### TreeIndex 缺失

```bash
# 若 output/.../scene_tree/7997.tree 缺失
cargo run -- --parse-db
```

### index_tree_enabled_target_types

- `DbOption.toml` 中 `index_tree_enabled_target_types = ["BRAN"]` 表示只为 BRAN 生成 TreeIndex
- 子件（ELBO 等）应随 BRAN 子树一起被收集；若未收集到，参考 DEBUG_24381_145019_PARQUET.md

### tubi 为空

- 检查 axis_map 是否从 DB/cache 正确加载
- 检查 `[BRAN_TUBI]` 日志中是否有 `跳过直段`、`无 axis_map` 等

---

## 5. 日志输出位置

- 控制台：`debug_model!` 等直接 `println!`
- 日志文件：`logs/24381_145018_YYYY-MM-DD_HH-MM-SS.log`（由 `AIOS_LOG_FILE` 环境变量决定）
