# 24381_145019 无数据诊断指南

## 问题描述

生成 dbnum=7997 的模型时，指定 `index_tree_enabled_target_types = ["BRAN"]`（DbOption.toml:53），
导出的 Parquet 中 `24381_145019`（ELBO 弯头）没有数据。

## 数据流与过滤点

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ 1. 实例收集                                                                   │
├─────────────────────────────────────────────────────────────────────────────┤
│ • 有 --root-refno: query_deep_visible_inst_refnos(root) → sub_refnos          │
│   → query_inst_relate_by_refnos → inst_rows                                  │
│ • 无 root-refno: query_inst_relate_by_dbnum → inst_rows（按 ref0 前缀扫描）   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────────┐
│ 2. 几何体查询                                                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│ query_insts_for_export(in_refnos) → 查 geo_relate / inst_relate_bool         │
│ → export_inst_map                                                            │
└─────────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────────┐
│ 3. 行数据构建                                                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│ 仅当 export_inst_map 有该 refno 且 export_inst.insts 非空时，写入 instance_rows│
└─────────────────────────────────────────────────────────────────────────────┘
```

## 可能原因

### 原因 A：root-refno 模式 + TreeIndex 未返回 24381_145019

- **表现**：`query_deep_visible_inst_refnos(24381_145018)` 的结果中不包含 24381_145019
- **条件**：使用了 `--root-refno 24381_145018`
- **说明**：24381_145019 是 BRAN 24381_145018 的子节点（ELBO），按逻辑应被返回。
  若未返回，可能是：
  - `output/<project>/scene_tree/7997.tree` 缺失或与 SurrealDB 不一致
  - TreeIndex 需从 parse-db 阶段生成

### 原因 B：无几何体数据（geo_relate / inst_relate_bool）

- **表现**：24381_145019 在 inst_rows 中，但不在 export_inst_map 中
- **说明**：ELBO 的几何体来自 CATE 生成。当 `index_tree_enabled_target_types = ["BRAN"]` 时，
  BRAN 流程会执行 gen_cata_instances，将 ELBO 等 CATE 写入 inst_relate_bool。
  若出现缺失，可能：
  1. 模型生成未完成或 CATE 步骤失败
  2. 该 ELBO 的 catalogue 数据异常
  3. geo_type 过滤（Pos/DesiPos/CatePos）排除了该记录

### 原因 C：有结构无几何（insts 为空）

- **表现**：export_inst_map 中有 24381_145019，但 `export_inst.insts` 为空
- **说明**：inst_relate_bool 有记录，但关联的 geo 无效或 geo_type 被过滤

## 诊断步骤

### 1. 带 verbose 运行导出

```bash
cargo run -- --export-dbnum-instances-parquet --dbnum 7997 -v
```

或（若使用 root-refno）：

```bash
cargo run -- --export-dbnum-instances-parquet --dbnum 7997 --root-refno 24381_145018 -v
```

查看输出中的：

- `子树 refno 数量` 以及 `24381_145019 不在 query_deep_visible_inst_refnos 结果中` 提示
- `24381_145019 不在 inst_rows 中` 或 `✓ 24381_145019 在 inst_rows 中`
- `以下 refno 在 inst_relate 但无几何体` 列表
- `[debug] 24381_145019 跳过: 不在 export_inst_map` 或 `export_inst.insts 为空`

### 2. 对比有无 root-refno

**不指定 root-refno**（全 dbnum 扫描）：

```bash
cargo run -- --export-dbnum-instances-parquet --dbnum 7997 -v
```

- 若此时 24381_145019 出现在 Parquet 中：问题在 root-refno 路径（TreeIndex / query_deep_visible_inst_refnos）
- 若仍无：问题在几何体（geo_relate / inst_relate_bool）

### 3. 直接查 SurrealDB

在 Surrealist 或 SurrealQL 中执行：

```sql
-- 检查 inst_relate
SELECT * FROM inst_relate WHERE in = pe:⟨24381_145019⟩;

-- 检查 geo_relate（通过 inst_relate 的 out）
SELECT * FROM geo_relate WHERE in IN (
  SELECT out FROM inst_relate WHERE in = pe:⟨24381_145019⟩
) AND geo_type IN ['Pos','DesiPos','CatePos'];

-- 检查 inst_relate_bool
SELECT * FROM inst_relate_bool WHERE in = pe:⟨24381_145019⟩;
```

若 geo_relate 和 inst_relate_bool 均无记录，则需先完成模型生成（含 CATE 步骤）。

### 4. 确认 TreeIndex 与 scene_tree

```bash
# 检查 .tree 文件
ls output/AvevaMarineSample/scene_tree/7997.tree

# 如缺失，需先解析/生成：
cargo run --bin aios-database -- --parse-db
```

## 建议修复

1. **几何体缺失**：重新跑 gen_model，确保 BRAN 流程完成且 CATE 生成无报错。
2. **TreeIndex 缺失**：执行 `--parse-db` 生成 scene_tree。
3. **需要完整导出**：临时将 `index_tree_enabled_target_types` 设为 `[]` 以启用全部类型，或使用 `--gen-nouns BRAN,ELBO,VALV,BEND,...` 扩大导出范围。
