# Implementation Plan

## Architecture

1. **删除面先行（M2）**：以盘点清单（`docs/plans/2026-07-22-ducklake-retirement-deletion-manifest.md`，T001 产出）为准绳，一次性移除 `version_store/`（除 `model_unit_commit.rs` 待搬家）、`generation_read/{ducklake,compare}.rs` 与 factory/catalog 的 ducklake 分支、`bootstrap-generation-read` CLI、`generation-read-ducklake` feature、`dep:duckdb`、perf gate 脚本、打包 DuckDB 资产；编译器暴露的悬挂引用即 M1/M3 的改造点清单。
2. **配置检测（随 M2）**：`options.rs` 删 `GenerationReadBackendMode::{Ducklake,Compare}`、`ParseStorageBackend::Ducklake`、`ducklake_*`/`duckdb_*` 字段及默认值函数；`get_db_option_ext` 校验链（`validate_data_source_mode` 旁）新增 raw TOML 分级检测：行为键硬错误 + 修复指引，惰性键警告。
3. **解析零痕迹（M1）**：删全量解析成功路径的 `full` 锚点固化（specs/022 T010 点位）与解析期 DuckLake 链调用（`versioned_db/database.rs` 等）；`committed_watermark` 回退链不动，新站首增量靠 dbnum_info 基线衔接。
4. **生成读路径（M3）**：保留 trait（`generation_read/traits.rs`）与共享领域逻辑（hierarchy 等）；`surreal.rs` 改造为主表直读适配器（复用长期生产验证的 query 层）；增量生成会话打开时取本次提交锚点 `anchored_at`，全部读加 `VERSION AT`；全量生成活读。删五张 `generation_replica_*` 表 schema 与复制链；输入版本清单改为 summary 观测字段，删 fail-closed 覆盖校验。
5. **起点锚点（M1/M3 交界）**：`publish_model_gen_anchors_after_generation` 现行为固定为原则（lib.rs 主路径 + cli_modes generate/regen 收尾），零新增代码，验收进 SC-001。
6. **台账搬家（M4）**：模型同库新建 Surreal `model_unit_commit` 表（`(dbnum, unit_refno, sesno)` + impact kind + 导出物指针，对齐 ADR-0005）；改造 `version_management/cli.rs`（6 处）与 `web_server/model_runtime.rs`（7 处）调用方；新增外置救济脚本 `scripts/migrate_unit_ledger_from_ducklake.ps1`（duckdb CLI 导 JSON → CLI 导入，仅文档标注可选）。
7. **文档收尾（M5）**：specs/024、025 加状态注记指向 ADR-0007/0008；ops-notes / AGENTS.md 增量段落核对（Committed Watermark 语义未变，删除 DuckLake 提及）；CHANGELOG 标注硬切与配置检测行为。

## Rollout

- **硬前置：版本控制兜底**——当前工作区未检测到 git 仓库；M2 删除面动工前先 `git init` + 首提交（或至少目录快照），这是唯一回滚保障。
- 行为变化集中两点，CHANGELOG/ops-notes 标注：① 带 ducklake 行为键的站点启动失败（修复=删两行配置）；② 解析不再产 full 锚点（新站历史查询自首个增量起可用）。
- 存量站点无需数据动作：`.ducklake` 遗物只读保留；台账空表起步；确需旧账走救济脚本。
- 验证遵循 AGENTS.md 约束：CLI `--json` 断言 + `scripts/smoke/*.ps1` + web_server 起服务 HTTP 实测，不使用 `cargo test`。

## Dependencies

- T001 盘点清单先于 M2 动工（已定义任务书，待工作组恢复后派发；产出与本 plan 第 1 条互相核对）。
- 与 `sesno_increment.rs` 的伴生加固（brooks-review M6 轨道）同文件改动需串行合入，避免冲突。
- spec-026 欠账闭环若尚在实施中，其 T003（追赶核心提炼）与本 spec M3 的生成入口改造存在交叠——先合谁都行，后合方 rebase 生成入口处的 VERSION AT 注入点。
