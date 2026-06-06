# 开发计划 — 全库 ref0/dbnum 预扫描索引 + 精确按需解析关联库

> 配套：`facts.md`（共识事实）/ `interview-result.json`（决策来源）。
> 核实基准：`D:/work/plant-code/plant-model-gen`（aios-database）+ 本地 patch `../pdms-io-fork`。

## 1. 方案概述

用 `pdms-io` 的 **index-only** 能力（只读文件头 + 遍历 B+树索引页，不解析元素属性）扫描站点所有工程根下的全部 db 文件，得到每个文件的 `dbnum / db_type / owned ref0 集合`，写入**站点级独立 SQLite** `db_index.sqlite`，形成覆盖全库（含未导入 SurrealDB 的元件/字典/规格库）的全局 `ref0→dbnum`。

解析站点时：(1) 解析前自动增量预扫；(2) 对设计库收集其**外向 refno**，经全局 `ref0→dbnum` 反查得到精确的外部依赖 dbnum（递归闭包），替换现有按 `CATA/DICT` 类型全量纳入的粗粒度逻辑；(3) 不论开关，恒纳入 `SYST/DICT/GLOB`。

关键复用 API（已核实）：
- `parse_pdms_db::parse::parse_file_basic_info`（`pdms-io-fork/crates/parse_pdms_db/src/parse.rs:3395`）→ `dbnum/db_type`。
- `pdms_io::PdmsIO::new/open/get_latest_sesno/collect_refno_locs`（`pdms-io-fork/src/io.rs:191/209/450/3099`）→ owned `RefnoDataLoc`（`refno_0` 即 ref0），不读元素。
- 解析编排：`run_parse_pipeline`（`managed_project_sites.rs:6915`）→ `spawn_parse_process:6427`（跑 `aios-database -c DbOption-parse`）；`build_parse_config:1846` / `resolve_included_db_files:1304`（粗粒度块 `:1393`）。
- 现有正向归属：`db_meta_info.json` / `DbMetaManager`（`data_interface/db_meta_manager.rs`，`ref0_to_dbnum`）—— 保留，不破坏。

## 2. 有序步骤

### Stage 0 — 索引存储模块（schema 先行）
- 新增 `src/data_interface/db_index.rs`：基于 `rusqlite`（已是 web_server feature 依赖）封装 `DbIndexStore`：
  - 建表 `db_file_index(dbnum INTEGER PRIMARY KEY, db_type TEXT, file_name TEXT, file_path TEXT, project TEXT, latest_sesno INTEGER, fingerprint TEXT, scanned_at TEXT)`、`ref0_owner(ref0 INTEGER PRIMARY KEY, dbnum INTEGER)`，索引 `ref0_owner(dbnum)`。
  - 路径：`runtime/admin_sites/<site_id>/db_index.sqlite`（站点级一份，多工程共享）。
  - API：`upsert_db_file(...)`、`replace_ref0_owners(dbnum, &[u32])`、`dbnum_by_ref0(ref0)`、`file_by_dbnum(dbnum)`、`all_dbnums_by_type(db_type)`、`fingerprint_of(dbnum)`。
- **验证**：`cargo build -p aios-database --features web_server`；单测建库→写一条→读回（rusqlite `bundled`）。

### Stage 1 — index-only 预扫描器
- 在 `db_index.rs` 增 `prescan_roots(store, project_roots, force) -> ScanReport`：
  - 复用站点根目录派生 `site_existing_project_roots`（`managed_project_sites.rs:1274`）与目录遍历 `scan_db_file_name`（`:~1240`）枚举 db 文件。
  - 每文件：`parse_file_basic_info` 取 `dbnum/db_type`；计算指纹 `fingerprint = latest_sesno + mtime + size`；若与库内一致且非 force → 跳过（增量）。
  - 变更/新增：`PdmsIO::new(project, path, false).open()` → `get_latest_sesno()` → `collect_refno_locs(sesno)` → 收集 `loc.refno_0` 去重 → `replace_ref0_owners` + `upsert_db_file`。
- **验证**（fact-index-only / fact-global-coverage）：对 `D:\AVEVA\Projects\E3D2.1\AvevaPlantSample`(+`AvevaCatalogue`) 跑预扫；断言 `ref0_owner` 含元件库 dbnum 对应 ref0；用计时/日志确认未走元素解析路径（不调用 `parse_db_basic_data`）。

### Stage 2 — 全局 ref0→dbnum 查询能力
- `DbIndexStore::resolve_dbnums(ref0s) -> Vec<dbnum>`（去重）；并提供 `DbMetaManager` 兜底合并（索引未命中时回落 `db_meta_info.json`）。
- **验证**（fact-global-coverage）：单测/CLI 给定一组 ref0 → 返回正确 dbnum，含未解析库。

### Stage 3 — SYST/DICT/GLOB 恒解析（低风险，独立）
- `managed_project_sites.rs`：将常量语义调整为「恒纳入类型」`ALWAYS_PARSE_DB_TYPES = ["SYST","DICT","GLOB","GLB"]`；在 `resolve_included_db_files:1304` 中**无条件**并入这些类型存在的文件（与 `auto_parse_related_dbnums` 解耦）。
- **验证**（fact-syst-dict-always）：构造站点关闭 `auto_parse`，断言生成的 `DbOption-parse.toml` `included_db_files` 仍含 SYST/DICT/GLOB 文件。

### Stage 4 — 精确依赖推导（方案 a）替换粗粒度
- 新增 `derive_related_dbnums_precise(store, design_dbnums) -> Vec<dbnum>`：
  - 取设计库**外向 refno**：用 `pdms-io` 读设计库元素引用属性的 `RefU64`（候选：`engine_v2/db4/attrs.rs:get_ref_array` + 显式/隐式属性中的 RefU64 值）。【执行期首要落实点，见风险 R1】
  - 外向 ref0 → `store.resolve_dbnums` → 过滤掉 design 自身 → 得直接依赖；
  - **递归闭包**：对新依赖库重复，直到无新增；`HashSet` 去重 + 访问标记防环。
- `resolve_included_db_files:1393`：当 `auto_parse_related_dbnums` 开 → 用 `derive_related_dbnums_precise` 的结果（映射回文件名）替换 `RELATED_DEPENDENCY_DB_TYPES` 全量纳入。
- **验证**（fact-precise-deps / fact-transitive-closure / fact-replace-coarse）：AvevaPlantSample golden——设计库依赖集 = 实际被引用的元件/规格库，**不含**未引用库；合成依赖链单测验证闭包与环检测。

### Stage 5 — 触发与入口
- **自动预扫**：`run_parse_pipeline:6915` 起始处调用 `prescan_roots(force=false)`（解析前自动 + 增量）。
- **CLI**：`cli_args.rs` 加子命令（如 `db-index rebuild [--force] [--site <id>]`），`cli_modes.rs` 实现，`main.rs` 路由。
- **admin 按钮**：`admin_handlers.rs:27` 路由表加 `POST /api/admin/sites/{id}/db-index/rebuild` → 调 `prescan_roots(force=true)`；前端在站点详情加按钮。
- **验证**（fact-auto-trigger-incremental / fact-cli-rebuild / fact-admin-rebuild-button[手动]）：改动某 db 的 mtime/sesno → 重跑断言仅该 db 重扫；CLI `db-index rebuild --force` 全量重建；admin 按钮手测返回 ok。

### Stage 6 — 回归与收尾
- **验证**（fact-no-regression）：`db_meta_info.json` 仍正常产出；`db_meta_manager` 相关既有单测通过；`cargo build` + 相关 `cargo test` 全绿。

## 3. 验证命令（汇总）
- 构建：`cargo build -p aios-database --features web_server`
- 单测：`cargo test -p aios-database --features web_server db_index`（新模块）；既有 `db_meta` 相关测试不回归
- 真实 golden：CLI `db-index rebuild --force` 对 `AvevaPlantSample`，查询 `db_index.sqlite` 校验 ref0_owner 覆盖与设计库精确依赖集
- 解析联动：关闭/开启 `auto_parse_related_dbnums` 各跑一次，比对 `runtime/admin_sites/<site>/DbOption-parse.toml` 的 `included_db_files`

## 4. 风险与开放问题
- **R1（首要）外向 refno 采集成本/接口**：index-only 仅给 owned ref0；设计库外向引用在元素属性中。需在 `parse_pdms_db`/`pdms-io` 落实一个「只取引用 RefU64、不做完整属性反序列化」的轻量采集器（候选 `db4/attrs.rs:get_ref_array` 与隐式/显式 RefU64 字段）。反应式路径 `handlers.rs:8807` 证明引用数据可得，但需确认能在解析前低成本拿到；若成本偏高，退化为「先解析设计库→读已解析 PE 的引用→再解析依赖」的两趟方案。
- **R2 ref0 多值/共享**：一个 dbnum 可能含多个 ref0（`ref0s` 为集合）；`ref0_owner` 以 ref0 为主键，需保证跨库 ref0 不冲突（理论上 ref0 全局唯一，落地时加冲突日志）。
- **R3 大库扫描时延**：B+树索引遍历虽轻，超大库仍有成本；预扫并发 + 指纹增量缓解；首扫可后台化并提示。
- **R4 站点级 vs 全局索引**：本轮按站点存一份（已定）；跨站点共享同一元件库会重复扫描，列为后续优化（out of scope）。
- **R5 GLOB/GLB 命名**：恒解析类型集需与实际工程 db_type 命名核对（`DEFAULT_PARSE_DB_TYPES` 现含 `GLB/GLOB`）。
