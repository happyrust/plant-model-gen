# 迁移指南：GenPipeline 配置硬切换（2026-07-21）

> 完整设计见 [`docs/superpowers/specs/2026-07-21-gen-pipeline-cleanup-rename-design.md`](../superpowers/specs/2026-07-21-gen-pipeline-cleanup-rename-design.md)。

## 概述

模型生成统一管线已从「IndexTree / `.tree`」命名与双源回退迁移为 **GenPipeline + pe_owner / GenerationRead**。配置键硬切换，**无兼容读**。

## DbOption.toml 迁移

删除旧键，改用新键：

```toml
# 删除
# index_tree_max_concurrent_targets = 6
# index_tree_batch_size = 200
# index_tree_enabled_target_types = ["BRAN"]
# index_tree_excluded_target_types = ["BOX"]
# index_tree_debug_limit_per_target_type = 0

# 使用
gen_pipeline_max_concurrent = 6
gen_pipeline_batch_size = 200
# gen_pipeline_enabled_target_types = ["BRAN"]
# gen_pipeline_excluded_target_types = ["BOX"]
gen_pipeline_debug_limit_per_target_type = 0
```

若仍保留任一旧 `index_tree_*` 键，配置加载会失败并打印 `旧键 -> 新键` 对照。

## 环境变量

删除 `AIOS_TREE_QUERY_SOURCE`（含 `=tree` / `=pe_owner`）。若仍设置，配置加载失败。层级查询仅 pe_owner / GenerationRead。

## 代码符号（开发者）

| 旧 | 新 |
|---|---|
| `index_tree_mode.rs` | `gen_pipeline.rs` |
| `process_index_tree_generation` | `process_gen_pipeline` |
| `gen_index_tree_geos_optimized` | `gen_pipeline_geos` |
| `IndexTreeConfig` / `IndexTreeError` | `GenPipelineConfig` / `GenPipelineError` |
| `get_index_tree_concurrency` | `get_gen_pipeline_concurrency` |

入口仍为 `gen_all_geos_data` / `gen_all_geos_data_with_session`。

## 故障排查

1. **旧键加载失败**：按上方对照改 toml。
2. **AIOS_TREE_QUERY_SOURCE 报错**：从环境/脚本中删除该变量。
3. **层级缺边**：跑 `scripts/smoke/pe_owner_children_audit.ps1`，必要时 `model-version rebuild-pe-owner`。
