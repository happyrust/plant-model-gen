# 代码审查与分析文档集 — 2026-06-08

本文件夹归集了 2026-06-08 一次代码审查 / 架构分析会话的全部产出。

| 主题 | 审查对象 | 分支 |
|------|---------|------|
| 多工程站点分支增量审查 + 数据解析保存流程分析 | `aios-database`（plant-model-gen） | `feat/multi-project-site` |

---

## 文档清单

### 1. `MULTI_PROJECT_SITE_CODE_REVIEW.md` — 代码校核意见书

`feat/multi-project-site` 相对 `main` 的**增量代码审查**（架构 + 正确性），遵循 Iron Law（Symptom → Source → Consequence → Remedy）。

- **范围**：217 文件 / +22,107 / −6,088 / 12 commits（抽样审查，聚焦 `src/web_server` 核心）
- **健康评分**：57 / 100（1 Critical + 5 Warning + 3 Suggestion）
- **核心结论**：方向正确（sidecar 控制面/数据面解耦、PID 防误杀、并发与安全加固）但工程纪律退化（继续向 12,102 行上帝模块 `managed_project_sites.rs` 堆 3,000 行，并带入进程泄漏与校验缺口）
- **最高优先级**：① sidecar 孤儿进程泄漏 ② `precheck_dbnum_conflicts` 空函数（dbnum 冲突校验缺失）③ 上帝模块拆分

### 2. `data-parse-save-flow.drawio` — 数据解析保存流程图

数据从 PDMS 原始库到多后端持久化的完整流程图（draw.io 原生 mxGraph 格式，24 节点 / 22 边）。

- **打开方式**：[draw.io / diagrams.net](https://app.diagrams.net) 在线打开，或 VS Code 安装 **Draw.io Integration** 扩展后直接预览/编辑
- **四条泳道**：
  1. **控制面**（web_server）：Admin UI → `admin_handlers` / `managed_project_sites` → `parse_sidecar_client`
  2. **Sidecar 进程**（`aios-database serve` · `src/parse_sidecar.rs`）：`/projects/scan`、`/parse/preview-plan`、`/db-index/rebuild`、`/jobs/submit-cli`（+ WS 事件流）
  3. **主管线 CLI**（`aios-database -c <config>`，`DbOption.toml` 驱动）：加载配置 → `sync_pdms` 解析 PDMS `.db` → `save_db?` 存 SurrealDB → 构建索引树（rkyv）+ `db_meta_info.json` → `gen_all_geos_data` 生成几何/网格/布尔 → 写 inst/geo/trans/aabb → 导出 Parquet
  4. **存储后端**：SurrealDB（主）/ 索引树文件 / RocksDB KV（可选）/ SQLite RTree 空间索引 / Parquet 文件组（前端查询）
- **关键事实**：真正的「解析 + 保存」发生在第 ③ 层（一次性 CLI 子进程）；`save_db` 决定是否落 SurrealDB，`gen_model` / `export_parquet` 决定是否生成几何与导出列存。

### 3. `review-findings-analysis.md` — 审核结论分析

对审查 9 条发现的**根因归纳**（收敛到 4 个主题）+ **对照最新合并代码（HEAD `c7b9900`）的重新验证**。

- **根因主题**：A 上帝模块（放大器）/ B 进程生命周期不完整 / C 校验时机错位 / D 概念一致性缺失
- **再验证结论**：9 条经最新代码核对**全部成立或仅部分缓解，无一被证伪**
  - W-2 被上游 `c7b9900`（sidecar 关闭竞态）**部分缓解**（加宽限期），但孤儿泄漏 / TOCTOU / 非 job 自关闭三点未修
  - W-1 **仍成立**；新增 `site_data_validation.rs` 是正交的「输出产物校验」，未覆盖创建期输入冲突预检
  - C-1 上帝模块本次又 +332 行，**更严重**
- **重排优先级**：先趁热收口 W-2 剩余 + 补 W-1 输入校验，再分批拆 C-1

---

## 方法论说明

- 代码审查基于 Brooks-Lint 的六类衰退风险（Cognitive Overload / Change Propagation / Knowledge Duplication / Accidental Complexity / Dependency Disorder / Domain Model Distortion）。
- 所有发现含具体文件行号，行号基于审查时 `feat/multi-project-site` HEAD。
- 流程图基于对 `src/parse_sidecar.rs`、`src/web_server/parse_sidecar_client.rs`、`src/versioned_db/database.rs`、`src/fast_model/gen_model/`、`src/fast_model/export_model/` 的源码追踪。

---

*生成日期：2026-06-08*
