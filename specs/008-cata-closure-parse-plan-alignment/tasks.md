# Tasks

## T001 现场复现与回归样本

- [x] 归档当前现场证据：
  - [x] `evidence/baseline-quicktest-250160-8080/parse.log`
  - [x] `evidence/baseline-quicktest-250160-8080/DbOption-parse.toml`
  - [x] `evidence/baseline-quicktest-250160-8080/parse-plan-manifest.json`
  - [x] `evidence/baseline-quicktest-250160-8080/cata_closure.json`
  - [x] `evidence/baseline-quicktest-250160-8080/parse-20260612-215036.json`
- [x] 记录 baseline：
  - [x] manifest only covers `250193`
  - [x] parse plan includes `250166`
  - [x] parse metrics has `250166 mode=skipped`
  - [x] parse succeeds with `error_count=0`

## T002 修复站点级 db_index manual target 依赖边

- [x] 修改 `src/parse_sidecar.rs::rebuild_db_index_request()`：
  - [x] 不再在 `manual_db_nums` 非空时跳过 `collect_design_outbound()`。
  - [x] 收集全部 DESI outbound 后，只保留 `src_dbnum in manual_db_nums` 的 source。
  - [x] 继续用 `DbIndexStore::resolve_dbnums(ref0s)` + `record_dependencies(src, dsts)` 写边。
- [ ] 增加/更新测试覆盖：
  - manual target 非空时仍记录目标库依赖。
  - manual target 为空时保持全 DESI 依赖收集。
- [ ] 验证站点根 `db_index.sqlite`：
  - `db_dependency where src_dbnum=250160` 至少包含 `250193`。

## T003 新增 manifest 驱动的 parse plan CATA 对齐 helper

- [x] 在 `src/web_server/managed_project_sites.rs` 新增 helper：
  - [x] 输入：`ManagedProjectSite`、`ManagedSiteParsePlan`、`CataClosureManifest`。
  - [x] 输出：与 manifest 覆盖范围对齐后的 `ManagedSiteParsePlan`。
- [x] 行为要求：
  - [x] 保留 manual DESI。
  - [x] 保留 DICT / mandatory preparse / system reuse / 非 CATA 文件。
  - [x] CATA 文件仅保留 `dbnum in manifest.by_dbnum.keys()`。
  - [x] 如果 manifest 覆盖的 CATA dbnum 不在原 plan 中，通过同目录 `db_index.sqlite` 补入文件名；不扫描 E3D 文件头。
  - [x] 同步更新 `included_db_files` 与 `auto_related_db_files`。
  - [x] 保持原解析计划顺序，缺失的 manifest 覆盖项追加在末尾，避免 UI diff 大幅抖动。
- [ ] 单元测试：
  - `250166` 被移除。
  - `250193` 被保留。
  - 原 plan 缺 `250193` 但 manifest 覆盖时，`250193` 可被补入。
  - 非 CATA 文件全部保留。
  - manifest 为空时移除全部 CATA。

## T004 在 spawn_parse_process 中接入 closure 后对齐

- [x] 修改 `spawn_parse_process()` 流程：
  - [x] 初始 parse plan 和 config 写入保持不变。
  - [x] `gen-cata-closure` 成功后读取站点 runtime 输出下的 `cata_closure.json`。
  - [x] 调用 T003 helper 对齐 parse plan。
  - [x] 再次调用 `write_site_files_with_parse_plan(..., Some(&aligned_plan))` 写回最终 `DbOption-parse.toml` 与 `parse-plan-manifest.json`。
- [x] 追加日志：
  - [x] `CATA manifest 对齐 parse plan: before=N after=M covered_dbnums=[...]`
- [x] fail-open：
  - [x] manifest 缺失/解析失败时只告警，不对齐。
  - [x] closure job 失败时保持现有失败路径。

## T005 收紧 sidecar preview plan 的 CATA 策略

- [x] 修改 `src/parse_sidecar.rs::resolve_included_db_files()`：
  - [x] 最终进入 parse 的配置不再因 `auto_parse_related_dbnums=true` 无条件收集所有 CATA。
- [x] 短期实现：
  - [x] 在 T003/T004 已能补入 manifest-covered CATA 后，`cata_partial_parse=true` 的 preview 不再加入全量 CATA；`cata_partial_parse=false` 仍保留全量 CATA 预览。
- [ ] 中期增强：
  - 扩展 preview 请求，携带可选 `db_index_path`。
  - 如果 index 可用，使用 `DbIndexStore::resolve_related_closure(manual_db_nums)` 生成 preview 的 CATA 列表。
- [ ] UI 文案：
  - 对 CATA partial 模式显示“CATA 目标由 closure manifest 决定”，避免用户误以为未纳入依赖。

## T006 调整 CATA skip 日志等级

- [x] 修改 `src/data_interface/cata_closure.rs::apply_sync_filter()`：
  - [x] manifest 外 CATA skip 从 `warn!` 降为 `info!`。
  - [x] 移除重复 `println!`。
- [x] 保留 metrics：
  - [x] `note_parse_db_mode(dbnum, "skipped", total_in_file)` 不变。
- [x] 确保真正异常仍告警：
  - [x] manifest 缺失。
  - [x] manifest 解析失败。
  - [x] closure job 失败。

## T007 验证 quicktest-250160-8080

- [ ] 重建二进制/发布包。
- [ ] 使用相同站点配置重跑 parse/generate。
- [ ] 检查最终 `DbOption-parse.toml`：
  - 包含 `aps250160_0001`。
  - 包含 `aps250193_0001`。
  - 不包含 `aps250166_0001`、`aps7351_0001`、`aps7355_0001` 等 manifest 外 CATA。
- [ ] 检查 parse log：
  - 不再出现 `dbnum=250166 不在 manifest 覆盖内`。
  - closure summary 仍为 `cata_dbs=1 visited=16` 或有明确解释的变化。
- [ ] 检查 metrics：
  - `250193 mode=partial elements=16`。
  - 无 `250166 mode=skipped`。
  - `success=true`、`error_count=0`。

## T008 回归验证

- [x] 跑基础编译：
  - `cargo check -q --features web_server`
- [x] 跑边界 guard：
  - `scripts/guard/web_server_parse_boundary_guard.ps1`
- [ ] 对已有 spec 002 的验证入口补一轮：
  - `verify-cata-closure` 对按需站点与整库基准 diff 不回归。
- [ ] 若生成阶段仍失败：
  - 单独记录到新问题；`The table 'ses' does not exist` 不归入本 spec。
