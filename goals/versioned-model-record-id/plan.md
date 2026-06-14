# Versioned Model Record ID Development Plan

## Goal

将模型生成产物表重构为 SurrealDB 3.1 range-friendly array record id，并把模型数据版本维度纳入架构。新架构不仅用于快速 cleanup，还要支持同一 `pe` 在不同 `sesno` 下拥有可查询、可并存、可按 range 删除的模型数据。

## Scope

In scope:

- 新增模型产物 record id helper，统一生成 `[ref0, ref1, sesno, ...extra]` ID。
- 将 `inst_relate` 建模为版本化模型关系：`inst_relate:[ref0, ref1, sesno]`。
- 保持 latest/current 的 `pe -> inst_relate` 查询体验，同时增加按 `sesno` 查询历史模型数据的路径。
- 将 `geo_relate`、`inst_relate_aabb`、`inst_relate_bool`、`inst_relate_cata_bool`、`refno_relations` 纳入版本化 ID。
- 将 `tubi_relate` 按 BRAN 的 `sesno` 纳入版本化 ID。
- 替换在线写库、布尔写入、refno relation 写入、导出查询和 cleanup 主路径。
- 使用 CLI、JSON 和 Surreal 查询完成验证。

Out of scope:

- 旧数据迁移。
- dual write 或 dual cleanup。
- 修改 `pe` 主表 ID 结构。
- `save_instance_data_to_sql_file` 的删除 SQL file 方案。
- 新增 Rust test 或运行 `cargo test`。

## Key Findings From Code

- `src/fast_model/gen_model/pdms_inst.rs` 是模型数据在线写入核心，当前 `inst_relate`、`geo_relate`、`neg_relate`、`ngmr_relate`、`inst_relate_aabb` 都使用旧 ID 假设。
- `geo_relate` 当前使用 `gen_string_hash(relate_json)` 作为 relation ID，无法表达 `[ref0, ref1, sesno, geo_index]`。
- `neg_relate/ngmr_relate` 当前保存裸 `geo_relate` hash 并用它拼 ID，迁移后必须保存完整 `geo_relate` record id。
- `src/fast_model/gen_model/pdms_inst_surreal.rs` 的 `refno_relations` 仍使用旧 `refno_relations:⟨pe_key⟩`。
- `src/fast_model/gen_model/manifold_bool.rs`、`src/fast_model/utils.rs` 写入 `inst_relate_bool`、`inst_relate_cata_bool` 时仍构造旧 ID。
- `src/fast_model/gen_model/inst_query.rs` 和多个 export module 仍假设 `record::id(in)` 可拼出同 ID 的 AABB/bool 记录。
- `src/fast_model/gen_model/refno_assoc_index.rs` 当前是清理快路径，但新架构要求 range id cleanup 成为主路径。

## Implementation Steps

### 1. Worktree And Baseline

- 在独立 worktree `D:\work\plant-code\plant-model-gen-range-id` 实施，不在当前脏 worktree 直接改代码。
- 将本 goal 包和 `plans/refno-range-id-cleanup.md` 的有效内容同步到实施 worktree，并以本 plan 为准。
- 记录当前可用的 CLI 启动方式、DB option 文件和最小验证 refno/dbnum/sesno。

Verification:

- `git status --short` 确认实施 worktree 干净或仅包含本任务改动。
- 不运行任何 test。

### 2. Add Versioned Model Record ID Helper

- 新增 `src/fast_model/gen_model/model_record_id.rs`。
- 提供 `refno_id_parts(refno) -> (ref0, ref1, sesno)`，明确处理 `RefnoEnum::Refno` 和 `RefnoEnum::SesRef`。
- 提供一对一 ID helper：`model_refno_id(table, refno)`。
- 提供 version-aware helper：`model_refno_id_with_sesno(table, base_refno, sesno)`，供从 `pe + sesno` 查询模型数据时使用。
- 提供一对多 ID helper：`geo_relate_id(carrier, geo_index)`、`neg_relate_id(...)`、`ngmr_relate_id(...)`、`tubi_relate_id(branch_refno, tubi_index)`。
- 提供 range helper：`model_ref0_range(table, ref0)`、`model_refno_range(table, refno)`、`model_refno_sesno_range(table, refno)`。
- 在 `gen_model/mod.rs` 导出 helper，调用方只依赖这一层。

Verification:

- 用 CLI/日志或 Surreal 查询手工确认 helper 生成示例：
  - `inst_relate:[24381,145569,0]`
  - `inst_relate:[24381,145569,12]`
  - `geo_relate:[24381,145569,12,0]`
  - `tubi_relate:[bran_ref0,bran_ref1,bran_sesno,0]`
  - `inst_relate:[24381,NONE]..=[24381,..]`

### 3. Define Versioned Query Contract

- 明确 latest/current 查询：默认使用 `sesno = 0`，保持现有接口行为不破坏。
- 明确历史版本查询：调用方传入 `RefnoEnum::SesRef` 或显式 `sesno` 时，查询 `inst_relate:[ref0, ref1, sesno]`。
- 在导出和实例查询入口中区分 base `pe` record id 与 model relation id。
- 避免在 SurrealQL 内临时拼 array id；优先在 Rust 侧用 helper 构造 record id 列表。

Verification:

- 同一 `pe` 构造 `sesno=0` 和 `sesno>0` 两组模型关系，确认 current 查询只读 `0`，历史查询只读目标 `sesno`。

### 4. Resolve Relation ID Ownership

- 实施前必须锁定 `neg_relate/ngmr_relate` 的 ID 前缀所有权。
- 推荐原则：ID 前缀必须服务于实际 regen/cleanup 驱动对象。如果按 target refno/sesno 重生成，就必须能按 target range 删除旧关系。
- 如果保留 carrier-first ID，则 cleanup 必须有确定的 carrier range 推导，不能退回 `WHERE target IN ...` 作为主删除策略。
- 将决策编码到 helper API，禁止调用方手拼 relation ID。

Verification:

- 构造 carrier 与 target 的 `ref0/sesno` 不同的样例，确认 regen target sesno 时旧 `neg_relate/ngmr_relate` 会被 range cleanup 删除。

### 5. Migrate Online Model Writes

- 修改 `save_instance_data_with_report`。
- `inst_relate` 使用 `inst_relate:[ref0, ref1, sesno]`，`in` 仍指向对应 `pe`，`out` 仍指向 `inst_info`。
- `geo_relate` 以 `[carrier_ref0, carrier_ref1, carrier_sesno, geo_index]` 生成 ID，不再用 `gen_string_hash(relate_json)` 作为主 ID。
- 将 `neg_geo_by_carrier` 和 `cata_cross_neg_geo_map` 的值从 `u64/hash` 改为完整 `geo_relate` record id 字符串或专用类型。
- `neg_relate/ngmr_relate` 写入时引用完整 `geo_relate` record id，并通过 helper 生成自身 ID。
- `inst_relate_aabb`、`inst_relate_bool`、`inst_relate_cata_bool` 写入都改用版本化 helper。
- 保留 `inst_info`、`inst_geo`、`aabb`、`trans`、`vec3` 原有 ID 语义，除非它们明确属于模型产物 range 清理表。

Verification:

- 用 CLI 生成一个包含多 geometry 的 refno。
- Surreal 查询确认同一 refno/sesno 下存在连续 `geo_relate:[ref0,ref1,sesno,index]`，且无 ID 冲突。
- 查询确认 `pe:...->inst_relate` current 路径仍可遍历，历史版本可按 `inst_relate:[ref0,ref1,sesno]` 查询。

### 6. Add Versioning To TUBI Relations

- 梳理当前 `tubi_relate` ID 和 BRAN 写入路径。
- 将 `tubi_relate` ID 改为包含 BRAN 的 `ref0/ref1/sesno`，例如 `[bran_ref0, bran_ref1, bran_sesno, tubi_index]`。
- BRAN 为 latest/current 时使用 `sesno=0`；历史 BRAN 使用真实 `sesno`。
- 更新 tubi 查询、导出和 cleanup range helper。

Verification:

- 对同一 BRAN 构造 current 和历史 sesno 的 tubi 关系，确认可分别查询。
- 对 BRAN ref0 或具体 BRAN/sesno 执行 range cleanup，确认只删除目标版本 tubi 数据。

### 7. Migrate Boolean And Refno Relation Writes

- 修改 `manifold_bool.rs` 的 `build_inst_relate_bool_upsert_sql`。
- 修改 `build_cata_status_sql` 中 `inst_relate_cata_bool` 的删除和插入策略，避免旧 ID 或 `WHERE in = $inst_info` 成为主清理路径。
- 修改 `fast_model/utils.rs` 中保存 bool/aabb 关系的旧 ID 构造。
- 修改 `pdms_inst_surreal.rs` 的 `refno_relations` 写入、读取和 cleanup。

Verification:

- 对存在布尔结果的 refno/sesno 执行 CLI 生成，确认 `inst_relate_bool:[ref0,ref1,sesno]` 状态可被 `query_insts` 读到。
- 对 `refno_relations` 执行写入和读取查询，确认 array id 生效。

### 8. Migrate Read And Export Queries

- 替换所有直接假设 `record::id(in)` 能拼出模型产物 ID 的查询。
- 对 `inst_relate_aabb`、`inst_relate_bool` 优先使用 Rust helper 预构造 ID 列表。
- 修改 `inst_query.rs` 的 bool key 和 inst_relate key 生成，使其支持 current 和历史 sesno。
- 修改 `export_dbnum_instances_parquet.rs`、`export_dbnum_instances_v3.rs` 的 bool/AABB/world export 查询。
- 修改 `export_glb.rs` 的 `filter_refnos_with_inst_relate_aabb`。
- 修改 prepack、instanced bundle、room instances 中 `record::exists(type::record('inst_relate_aabb', record::id(in)))` 类查询。

Verification:

- 对同一 refno 分别运行 current 和历史版本导出查询。
- JSON 输出中 `world_aabb_hash`、`world_trans_hash`、`insts.geo_hash` 不回退为空。
- prepack/GLB 过滤不因 AABB 查询迁移漏掉已生成实体。

### 9. Replace Cleanup Main Path With Range Cleanup

- 在 helper 或新 cleanup module 中集中生成 range delete SQL。
- `--dbnum`、`manual_db_nums`、全库路径先解析 dbnum 覆盖的 ref0 集合，再对每个 ref0 执行 range cleanup。
- 显式 refno/sesno regen 对目标 refno 使用 refno/sesno 前缀 range，不再依赖 refno 子树连续性。
- `inst_geo` 删除流程保持“先从将删除的 `geo_relate.out` 收集 hash，再删除 hash >= 10”。
- `refno_assoc_index` 保留为可选诊断/过渡数据结构，但不再作为 `pre_cleanup_for_regen` 的模型产物主删除路径。
- 不实施 `save_instance_data_to_sql_file` 删除 SQL file 路线。

Verification:

- 手工创建：
  - `inst_relate:[24381,1,0]`
  - `inst_relate:[24381,1,12]`
  - `inst_relate:[24381,2,0]`
  - `inst_relate:[24382,1,0]`
- 执行 ref0 range cleanup，确认只删除 `24381`。
- 执行具体 refno/sesno cleanup，确认只删除目标版本。

### 10. Remove Legacy Compatibility

- 删除或停止调用旧模型产物 ID 路径：
  - `key.to_inst_relate_key()` 用于模型产物表的调用。
  - `key.to_table_key("inst_relate_aabb")` 等模型产物表调用。
  - `format!("inst_relate_bool:⟨{}⟩", refno)`。
  - `format!("geo_relate:⟨{}⟩", relate_id)`。
- 不新增兼容旧 ID 的 fallback。
- 对非模型产物表的 `to_pe_key()`、`to_table_key("pe_transform")` 保持不变。

Verification:

- 全仓搜索确认旧模型产物 ID 模式只剩文档或明确的历史说明，不在执行路径中出现。

### 11. End-To-End Validation

- 启动 SurrealDB 和 `aios-database` 所需环境。
- 使用 CLI 对最小 refno/current 和至少一个 `sesno>0` 版本生成模型数据。
- 使用 Surreal 查询验证：
  - 新生成模型产物数据只使用 array record id。
  - current 和历史版本模型数据可并存、可分别查询。
  - `pe` 图关系仍可遍历到 current 实例和几何。
  - 多 geometry refno 不冲突。
  - `tubi_relate` 可按 BRAN sesno 查询。
  - `neg_relate/ngmr_relate` 引用完整 `geo_relate` id。
  - range cleanup 能删除目标 ref0/refno/sesno 数据且不误删相邻 ref0 或其他 sesno。
- 如涉及 `web_server`，启动服务后通过 HTTP/POST 验证，不写 Rust test。

## Risks And Open Questions

- `neg_relate/ngmr_relate` 的 ID 前缀若与 regen 驱动对象不一致，会留下旧关系；这是最高优先级设计风险。
- `pe -> inst_relate` 的历史版本查询需要清晰 API，避免 current 查询误读历史版本或历史查询默认落回 current。
- `tubi_relate` 需要以 BRAN 的 `sesno` 为准，不能混用 TUBI/owner 的版本来源。
- SurrealQL 内构造 array record id 不如 Rust 侧稳定，读路径应优先在 Rust 中构造 record id 或改用字段查询。
- 当前仓库已有大量未提交改动，实施必须在独立 worktree 中完成并单独验证。
