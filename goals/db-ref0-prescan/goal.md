# Goal — 全库 ref0/dbnum 预扫描索引 + 精确按需解析关联库

## 目标

为站点部署的「按需解析关联库」建立一个 **index-only 全库预扫描**：用 `pdms-io` 的索引块快速抽取每个 dbfile 的 `dbnum` 与 owned `ref0`，存入站点级独立 SQLite（`db_index.sqlite`）形成覆盖全部库的全局 `ref0→dbnum`。据此在解析设计库时**精确推导其外部依赖库**（递归闭包，方案 a）做自动解析，替换现有按 `CATA/DICT` 类型的粗粒度纳入；并让 `SYST/DICT/GLOB` 恒提前解析。

## 共识与执行

- 共识事实（验收口径）：见 [`facts.md`](./facts.md)
- 执行计划（有序步骤 + 验证 + 风险）：见 [`plan.md`](./plan.md)（已通过 Plannotator 门禁 `approved`）
- 决策来源：[`interview-result.json`](./interview-result.json) / [`facts-result.json`](./facts-result.json)

## 完成条件（Done）

1. 站点级 `db_index.sqlite`（`db_file_index` + `ref0_owner`）由 index-only 预扫生成，覆盖站点内全部 db（含未导入 SurrealDB 的库）。
2. 给定设计库可输出**精确**外部依赖 dbnum（递归闭包、不含未引用库），并驱动 `auto_parse_related_dbnums` 的 `included_db_files`，取代 `managed_project_sites.rs:1393` 的粗粒度块。
3. 不论开关，`SYST/DICT/GLOB` 恒纳入解析。
4. 预扫解析前自动触发并按指纹增量重扫；提供 CLI 重建子命令与 admin『重建索引』按钮。
5. `db_meta_info.json` 与既有 `ref0_to_dbnum` 使用方无回归；`cargo build` 与相关 `cargo test` 全绿；AvevaPlantSample 真实 golden 通过。
