# GenPipeline 清理与改名设计（2026-07-21）

> 承接：`docs/plans/2026-07-20-pe-owner-tree-query-migration-dev-plan.md` 的 **M4/M5**。  
> 背景：模型生成主路径已迁到 `GenerationRead` + pe_owner 层级快照；`.tree` / TreeIndex 仅剩双源回退与命名歧义。  
> 决策（会话确认）：配置 **硬切换**（无旧键兼容）；命名体系 **GenPipeline**；落地 **两阶段**（先停用 tree 回退并改名，再删生产侧）。

## 1. 目标与非目标

### 目标

1. 消除「IndexTree = `.tree` 文件」的歧义：代码、配置、日志统一称 **GenPipeline**。
2. 运行时层级数据源只保留 pe_owner / GenerationRead；删除 `AIOS_TREE_QUERY_SOURCE=tree` 回退。
3. Phase2 停产 `.tree` 写出与 `TreeIndexManager` 消费，完成原迁移计划 M4。

### 非目标

- 不改 BRAN → LOOP → CATE → PRIM 算法顺序与布尔/mesh 下游管线。
- 不改 specs/023 pe_owner 版本化语义、specs/024/025 DuckLake / generation-read 权威语义。
- Phase1 不删除 `--gen-indextree` / `tree_export` 生产命令（留给 Phase2）。

## 2. 命名对照（硬切换）

| 旧 | 新 |
|---|---|
| `index_tree_mode.rs` | `gen_pipeline.rs` |
| `process_index_tree_generation` | `process_gen_pipeline` |
| `gen_index_tree_geos_optimized` | `gen_pipeline_geos` |
| `gen_index_tree_geos_for_incremental_log` | `gen_pipeline_geos_for_incremental_log` |
| `IndexTreeConfig` | `GenPipelineConfig` |
| `IndexTreeError` | `GenPipelineError`（同模块改名，不保留旧类型别名） |
| `index_tree_batch_size` | `gen_pipeline_batch_size` |
| `index_tree_max_concurrent_targets` | `gen_pipeline_max_concurrent` |
| `index_tree_enabled_target_types` | `gen_pipeline_enabled_target_types` |
| `index_tree_excluded_target_types` | `gen_pipeline_excluded_target_types` |
| `index_tree_debug_limit_per_target_type` | `gen_pipeline_debug_limit_per_target_type` |
| 日志 `[gen_model] IndexTree…` | `[gen_pipeline]…` |
| `AIOS_TREE_QUERY_SOURCE=tree\|pe_owner` | **删除**（Phase1 起仅 pe_owner） |
| `TreeIndexManager`（运行时查询） | Phase1 调用面清零；Phase2 删模块 |

`get_index_tree_*` 访问器同步改为 `get_gen_pipeline_*`；**不保留 deprecated 别名**。

## 3. 落地路径

采用 **语义先行、改名跟进**（非机械大改名单 PR、非长期门面适配）：

1. Phase1：砍运行时 tree 回退 + 配置硬切 + GenPipeline 改名。
2. Phase2：停产 `.tree` 与删除 `TreeIndexManager` 等生产侧残留。

## 4. Phase 1 — 消歧义 + 停用 tree 回退

### P1-A 配置硬切换

- `src/options.rs` 与所有 `db_options/*.toml`：旧键删除，只认新键。
- 加载时若检测到任一旧键（`index_tree_*`）：**失败退出**，错误信息列出旧→新映射表（禁止静默忽略）。
- 同步文档示例、smoke、fixture 中的 toml。

### P1-B 生成管线改名

- `git mv` `index_tree_mode.rs` → `gen_pipeline.rs`；更新 `mod.rs` 与全部引用。
- 重命名 §2 表中的类型/函数/配置访问器与日志前缀。
- `orchestrator.rs` 中「IndexTree 统一管线」注释改为 GenPipeline。

### P1-C 砍掉运行时双源

- 删除 `AIOS_TREE_QUERY_SOURCE` 与 `latest_tree_source_is_pe_owner()`；调用点内联为 pe_owner 路径。
- 删除 `HierView::Tree`、`TreeIndexQueryProvider`、query_compat / utilities / neg_query 等 tree 回退分支。
- `precheck_coordinator`：移除 `.tree` 检查/生成提示分支；保留 pe_transform / db_meta。
- 生成主路径只经 `GenerationReadContext.hierarchy` 与 pe 快照 provider。
- 若仍设置 `AIOS_TREE_QUERY_SOURCE`：**拒绝启动或明确错误**（避免运维误以为仍可回退）。

### P1-D 文档与验收

- CHANGELOG：硬切换说明 + 配置迁移表。
- 原 pe_owner 迁移计划标注：M4/M5 由本文档承接。
- Smoke：`pe_owner_latest_tree_smoke` 改为单源断言（不再双源 diff）。
- 验收：
  - `cargo check --lib` / `--features web_server` / sync-cli 瘦构建绿；
  - `rg 'index_tree_|IndexTree|AIOS_TREE_QUERY_SOURCE'` 在生成热路径（`gen_model/`、`options.rs`、web API latest）清零；允许 Phase2 待删文件（`tree_export.rs`、`tree_index_manager.rs`、CLI gen-indextree）暂时残留。

## 5. Phase 2 — 生产侧退役

### P2-A 停产 `.tree`

- 删除 `versioned_db/tree_export.rs` 与 `export_tree_file` 写出点。
- `--gen-indextree` / `--gen-all-desi-indextree` / `gen_tree_only`：删除或改为 `gen_db_meta_only`（只维护 `db_meta_info.json`）；更新 `init_project` 第一步。
- 删除 `tree_index_manager.rs`；`resolve_dbnum_for_refno` 迁至 `db_meta_manager`（本就 db_meta 驱动）。
- 删除 `TREE_INDEX_CACHE`、`load_index_with_large_stack`。

### P2-B 清理与文档

- `rg 'TreeIndex|indextree|\.tree'` 在 `src/` 清零（若 rs-core 仍暴露类型，仅允许类型 import 且无加载 `.tree` 的调用）。
- ops-notes：`scene_tree/` 仅保留 `db_meta_info.json`；旧 `.tree` 可删。
- 修订 specs/023 FR-005 等「不传 sesno 走 TreeIndex」历史表述为 pe_owner。

### P2 验收

- 无 `.tree` 站点：`init-project` + 全量/增量生成 + latest 树 API 全绿。
- 增量「BRAN 下新增管件」仍被生成（回归原迁移计划 §0-3）。
- t012 生成耗时相对 Phase1 基线无异常回退（预算沿用 ≤10%，若无可比基线则记录绝对墙钟）。

## 6. PR 切分

| PR | 内容 | 合入门槛 |
|---|---|---|
| **PR-1** | P1-A + P1-C（行为：硬切配置、砍双源） | check 全绿；旧 toml 键启动失败含迁移提示；无 tree 回退开关 |
| **PR-2** | P1-B + P1-D（改名 + 文档/smoke） | 生成路径无 IndexTree 命名；CHANGELOG 含迁移表 |
| **PR-3** | Phase2 全量 | 无 `.tree` 站点 e2e；src 无 TreeIndex 生产/消费 |

PR-1 与 PR-2 可合并；**PR-3 必须独立**。

建议顺序：

```
P1-A 配置硬切 ──┐
P1-C 砍双源   ──┼──→ P1-B 改名 ──→ 站点 toml 迁移
                └──→ P1-D 文档/smoke
                         │
                         ▼ 站点稳定后
                       P2-A/B 停产删除
```

## 7. 站点迁移清单

升级前必须改 toml（无兼容读）：

```toml
# 删除旧键
# index_tree_max_concurrent_targets = 6
# index_tree_batch_size = 200
# index_tree_enabled_target_types = [...]
# index_tree_excluded_target_types = [...]
# index_tree_debug_limit_per_target_type = 0

# 使用新键
gen_pipeline_max_concurrent = 6
gen_pipeline_batch_size = 200
# gen_pipeline_enabled_target_types = [...]
# gen_pipeline_excluded_target_types = [...]
gen_pipeline_debug_limit_per_target_type = 0
```

环境变量：移除 `AIOS_TREE_QUERY_SOURCE`；Phase1 后设置该变量应报错。

## 8. 成功标准

1. **语义**：讨论「生成管线」不再联想到 `.tree` 文件。
2. **运行时**：层级只来自 GenerationRead / pe_owner 快照。
3. **配置**：仓库内零 `index_tree_*`；旧键加载失败信息含新键名。
4. **代码**：Phase1 后生成路径无 Tree 回退；Phase2 后无 TreeIndex 生产/消费。
5. **验证**：三面 `cargo check` + pe_owner smoke；（Phase2）无 tree 站点增量生成含新管件。

## 9. 风险与回退

| 风险 | 缓解 |
|---|---|
| 存量站点未改 toml | 加载失败 + 迁移表；CHANGELOG/ops 显著标注 |
| 边不完整站点 pe_owner 查询截断 | 既有 `audit_pe_owner_vs_children` + `rebuild-pe-owner`；不绿站点先修边 |
| 性能相对 `.tree` 反序列化回退 | Phase1 不改变快照加载策略；Phase2 前后记 t012 墙钟 |
| 误删仍被旁路引用的 TreeIndex API | PR-1 先清调用面，PR-3 再删文件；`rg` 门禁 |

回退：Phase1/2 均以 **git revert** 回退；**不再**保留 `AIOS_TREE_QUERY_SOURCE=tree` 作为运行时逃生舱。

## 10. 与既有计划关系

| 文档 | 关系 |
|---|---|
| `docs/plans/2026-07-20-pe-owner-tree-query-migration-dev-plan.md` | M0–M3 已落地；**M4/M5 由本文档承接** |
| `specs/025-versioned-generation-read-session` | 生成读会话不变；本设计只改命名与去掉 tree 回退 |
| `specs/023` | latest 层级源表述随 Phase2 文档修订对齐 pe_owner |

## 11. 实施后下一步

用户批准本文档后，使用 writing-plans 产出可执行任务清单（按 PR-1 → PR-2 → PR-3），再开始改代码。
