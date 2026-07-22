# GenPipeline 清理与改名 Implementation Plan

> **For agentic workers:** Use subagent-driven-development or executing-plans. Checkbox tasks track progress.
> 依据：[`docs/superpowers/specs/2026-07-21-gen-pipeline-cleanup-rename-design.md`](../specs/2026-07-21-gen-pipeline-cleanup-rename-design.md)

**Goal:** 去掉 IndexTree/`.tree` 歧义与运行时双源，统一为 GenPipeline + pe_owner/GenerationRead。

**Architecture:** PR-1 配置硬切 + 砍双源 → PR-2 改名 → PR-3 停产 `.tree` / 删 TreeIndexManager。

## Tasks

- [x] PR-1: `gen_pipeline_*` 硬切 + 旧键/`AIOS_TREE_QUERY_SOURCE` 失败
- [x] PR-1: 删除 HierView/Provider/API/导出/precheck 的 tree 回退
- [x] PR-1: pe_owner smoke 单源化 + CHANGELOG
- [x] PR-2: `index_tree_mode` → `gen_pipeline` 与核心符号/日志改名
- [x] PR-2: 访问器收尾 + CHANGELOG + 计划回指 + MIGRATION_GUIDE
- [x] PR-3: 停写 `.tree`；CLI `--gen-db-meta` / `gen_db_meta_only`
- [x] PR-3: 删 TreeIndexManager；`resolve_dbnum` 迁 db_meta
- [x] PR-3: 文档/ops + 验收
