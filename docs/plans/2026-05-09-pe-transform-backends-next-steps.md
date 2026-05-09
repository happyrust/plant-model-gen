# pe-transform-backends 下一步开发计划

> 文档日期：2026-05-09
> 当前分支：`feat/pe-transform-backends`（commit `512663f`）
> 关联分支：`feat/collab-api-consolidation`（主工作树，ModelWriter trait 重构未提交）

---

## 0. 当前状态

### 已完成

| 项 | 状态 | 说明 |
|---|---|---|
| `pe_transform_store.rs` 模块 | ✅ 已提交 | save/load/clear/compare 四个公开接口 |
| SurrealDB 读写后端 | ✅ 运行验证中 | dbnum=1112，42.4 万节点刷新链路畅通 |
| Parquet 读写后端 | ✅ 编译通过 | 需 `transform-store-parquet` feature 运行验证 |
| Rkyv/Memory 读取后端 | ✅ 编译通过 | cfg(gen_model) 门控 |
| compare_backends_for_dbnums | ✅ 编译通过 | delta 含 translation/rotation/scale 三维度 |
| DuckLake 后端 | ⚠️ 空桩 | register_ducklake 仅 Ok(()) |
| CLI 参数 (7 个) | ✅ --help 已验证 | transform-write/read-backend, parquet-dir 等 |
| transform_cache 预热 | ✅ 已集成 | prime_global_transform_cache_from_pe_entries |
| 构建工具链 | ✅ cmake + NASM 已安装 | aws-lc-sys 编译通过 |

### 主工作树未提交改动（`feat/collab-api-consolidation`）

| 文件 | 改动 |
|---|---|
| `model_writer.rs` | ModelWriter trait + SurrealModelWriter + DrainOnlyWriter |
| `orchestrator.rs` | base_writer / mesh_stage 重构为 worker pool |
| `transform_cache.rs` | prime_global_transform_cache_from_pe_entries（与 pe-transform 分支重叠） |
| `pe_transform_refresh.rs` | compat 函数签名调整（与 pe-transform 分支重叠） |
| 多处 web_api / web_server | review/annotation/e3d/mbd 微调 |

---

## 1. 冲刺 A — Parquet 后端运行验证（~0.5 天）

### 目标
用 `--transform-write-backend dual` 实际写出 Parquet 文件，验证读回一致性。

### 任务

| # | 任务 | 详情 |
|---|---|---|
| A1 | 用 `transform-store-parquet` feature 重编译 | `cargo build --features "review,transform-store-parquet"` |
| A2 | dual 模式刷新 | `--refresh-transform 1112 --transform-write-backend dual --transform-parquet-dir output/pe_transform` |
| A3 | 验证 Parquet 文件 | 检查 `output/pe_transform/pe_transform.parquet` 行数、字段完整性 |
| A4 | compare 对比 | `--transform-compare-backends parquet` 对比 surreal vs parquet，max_delta 应为 0 |
| A5 | 纯 parquet 读取验证 | `--transform-read-backend parquet --refresh-transform 1112`，验证与 surreal 结果一致 |

### 验收标准
- Parquet 文件行数 ≥ 40 万（与 surreal 一致）
- compare max_delta = 0，missing = 0
- 无 panic / 无数据丢失

---

## 2. 冲刺 B — 两分支合并（~0.5 天）

### 目标
将 `feat/pe-transform-backends` 和 `feat/collab-api-consolidation` 的改动合入统一分支。

### 冲突分析

| 文件 | 冲突风险 | 处理策略 |
|---|---|---|
| `transform_cache.rs` | 高 | 两分支都新增 `prime_global_transform_cache_from_pe_entries`，但 pe-transform 分支多了 `pub(crate)` + `query_from_configured_store`；取 pe-transform 版本为主，合入 ModelWriter 的 `Copy` derive 改动 |
| `pe_transform_refresh.rs` | 中 | pe-transform 分支改了 `flush_entries`，主分支改了 compat 签名；需手动合并 |
| `orchestrator.rs` | 低 | 仅主分支改动，无冲突 |
| `model_writer.rs` | 低 | 仅主分支改动，无冲突 |
| `options.rs` | 低 | 仅 pe-transform 分支改动 |
| `main.rs` | 中 | 两分支都改了参数解析；pe-transform 加了 7 个 CLI 参数，主分支改了 `--model-writer` 逻辑 |

### 任务

| # | 任务 |
|---|---|
| B1 | 在 pe-transform-backends 分支上 `git merge feat/collab-api-consolidation`（或反向） |
| B2 | 解决 transform_cache.rs / pe_transform_refresh.rs / main.rs 冲突 |
| B3 | `cargo check --features review` 验证合并后编译 |
| B4 | `cargo check --features "review,transform-store-parquet"` 验证 parquet 路径 |

---

## 3. 冲刺 C — ModelWriter + Transform 后端集成（~1 天）

### 目标
让模型生成管线同时使用 ModelWriter trait 和 pe_transform 多后端。

### 任务

| # | 任务 | 详情 |
|---|---|---|
| C1 | orchestrator 集成 | `gen_all_geos_data` 根据 `model_writer_mode` 创建 ModelWriter，根据 `transform_write_backend` 控制刷新路径 |
| C2 | 端到端验证 | 完整模型生成（gen_model）+ Parquet 双写，确认几何输出不变 |
| C3 | drain-only + parquet 组合 | 验证 `--model-writer drain-only --transform-write-backend parquet` 压测路径 |

---

## 4. 冲刺 D — DuckLake 实现（~1 天，可选）

### 目标
让 Parquet 文件通过 DuckLake 注册到统一数据湖目录。

### 前置条件
- DuckDB 可用（winget / cargo feature）
- DuckLake metadata schema 确定

### 任务

| # | 任务 |
|---|---|
| D1 | `register_ducklake` 实现：创建/更新 DuckLake metadata.ducklake |
| D2 | `load_entries_from_ducklake` 实现：通过 DuckDB 查询 DuckLake 注册的 Parquet 文件 |
| D3 | 验证 `--transform-write-backend ducklake` 端到端流程 |

---

## 5. 冲刺 E — 性能基线与优化（~0.5 天）

### 目标
建立各后端的性能基线，识别优化机会。

### 任务

| # | 任务 |
|---|---|
| E1 | 基线测量：surreal / parquet / rkyv 各后端 load 时间（用 compare 的 elapsed_ms） |
| E2 | Parquet 列式优化：评估 binary 编码 Transform（替代 JSON string）的收益 |
| E3 | 分 dbnum Parquet 文件：当前单文件合并，大规模时可能需要按 dbnum 分片 |

---

## 风险登记

| 风险 | 等级 | 缓解 |
|---|---|---|
| 合并冲突导致逻辑回退 | P1 | B2 后立即做完整 cargo check + 端到端验证 |
| Parquet JSON 序列化精度丢失 | P2 | compare 对比已覆盖，max_delta > 0 即告警 |
| DuckLake 依赖引入新编译重量 | P2 | 独立 feature gate，不影响默认编译 |
| transform_cache 预热与旧 rkyv snapshot 冲突 | P3 | prime 函数标记 loaded_dbnums，后续 miss 走 DB fallback 而非旧 snapshot |

---

## 推荐执行顺序

**冲刺 A → B → C → E → D**

- A（Parquet 运行验证）是 B（合并）的前提 — 确认新模块独立正确后再合
- C（集成）依赖 B — 需要两分支代码统一后才能做端到端
- E（性能基线）可与 C 并行 — compare 功能已就绪
- D（DuckLake）优先级最低，仅在有数据湖需求时推进
