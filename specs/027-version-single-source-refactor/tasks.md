# Tasks

- [ ] T001 全仓删除清单盘点（产出 `docs/plans/2026-07-22-ducklake-retirement-deletion-manifest.md`，分 A 整文件/B 局部/C 配置脚本/D 文档规格四组 + 编译验证门）
- [ ] T002 版本控制兜底：`git init` + 基线提交（或目录快照）——M2 动工硬前置
- [ ] T003 删除面：`version_store/`（除 model_unit_commit）、`generation_read/{ducklake,compare}` 与 factory/catalog 分支、`bootstrap-generation-read` CLI、feature/`dep:duckdb`、perf gate 脚本、打包 DuckDB 资产、watermark/authority feature fork
- [ ] T004 `options.rs` 字段清理 + `get_db_option_ext` 启动分级检测（行为键硬错误+指引 / 惰性键警告）
- [ ] T005 解析零痕迹：删 `full` 锚点固化与解析期 DuckLake 链调用；新站首增量衔接手测（US1/US2）
- [ ] T006 生成读路径：主表直读适配器、增量 `VERSION AT` 钉住、全量活读、五张副本表删除、输入版本清单降级观测记录
- [ ] T007 台账搬家：Surreal `model_unit_commit` 表 + cli/model_runtime 调用方改造 + 外置救济脚本
- [ ] T008 验证门：三组构建（默认 / `--features full` / build-sync-cli）、SC-001 新站剧本冒烟、SC-002 配置检测手测、SC-004 并发切面验证、既有 e2e/审计冒烟回归
- [ ] T009 文档收尾：specs/024、025 状态注记、ops-notes/AGENTS.md 核对、CHANGELOG（硬切 + 配置检测 + 解析零痕迹）
