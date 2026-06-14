# Tasks

## T001 现场证据与当前实现确认

- [x] 记录现场站点：
  - `quicktest-250160-8080`
  - 主项目：`AvevaPlantSample`
  - 关联项目：`AvevaCatalogue`
- [x] 记录被推翻的 008 假设：
  - runtime 继续按当前遍历 project 推导 manifest path。
- [x] 确认当前实现落点：
  - `src/versioned_db/database.rs` 三处调用 `load_sync_filter(project.as_str(), ...)` / `load_sync_filter(project_name, ...)`。
  - `src/data_interface/cata_closure.rs::load_sync_filter()` 仍调用 `default_manifest_path(project_name)`。
  - `src/data_interface/cata_closure.rs::apply_sync_filter()` 已能拿到 actual/total，但日志没有主项目 manifest context。
  - `src/web_server/managed_project_sites.rs::summarize_log_line()` 仍把 `All refnos count` 摘要成“最近 refno 计数”。

## T002 规格文件落地

- [x] 新增 `specs/009-main-project-cata-manifest-logging/spec.md`。
- [x] 新增 `specs/009-main-project-cata-manifest-logging/plan.md`。
- [x] 新增 `specs/009-main-project-cata-manifest-logging/tasks.md`。
- [x] 检查 009 与 008 的边界：
  - 008 = parse plan 与 manifest 覆盖对齐。
  - 009 = runtime manifest 权威路径 + 日志/metrics 真实数量口径。

## T003 显式 manifest path runtime context

- [x] 在 `src/data_interface/cata_closure.rs` 增加 manifest path 解析：
  - [x] 新增 env key `AIOS_CATA_CLOSURE_MANIFEST_PATH`。
  - [x] 新增 env key `AIOS_CATA_CLOSURE_MAIN_PROJECT`。
  - [x] `AIOS_CATA_CLOSURE_MANIFEST_PATH` 非空时优先使用该路径。
  - [x] 未设置显式路径时保留 `default_manifest_path(project_name)` 旧行为。
- [x] 为 filter 增加 context：
  - [x] manifest path。
  - [x] main project name（可选）。
  - [x] current project name / path source 仅用于日志。
- [x] 保持 `None` = 整库回退语义不变。

## T004 web_server 传递主项目 manifest context

- [x] 修改 `src/web_server/managed_project_sites.rs::spawn_parse_process()`：
  - [x] 在 `cata_partial_enabled` 时继续设置 `AIOS_CATA_CLOSURE_MODE=manifest`。
  - [x] 追加 `AIOS_CATA_CLOSURE_MANIFEST_PATH=cata_manifest_path_for_site(&site)`。
  - [x] 追加 `AIOS_CATA_CLOSURE_MAIN_PROJECT=site.project_name`。
- [x] 确保 env 只传给 parse job，不影响 closure job。
- [x] 确保 `cata_partial_enabled=false` 时不设置新 env。

## T005 runtime 日志诊断增强

- [x] 更新 `load_sync_filter()` 成功日志：
  - [x] 输出 manifest path。
  - [x] 输出 main project（如有）。
  - [x] 输出 current project。
  - [x] 输出 path source：`explicit` / `derived`。
- [x] 更新 missing manifest warning：
  - [x] 包含 manifest path。
  - [x] 包含 main project。
  - [x] 包含 current project。
  - [x] 明确“CATA 整库回退”。
- [x] 更新 manifest parse error warning：
  - [x] 同样包含上述 context。

## T006 actual/total 解析数量日志

- [x] 更新 `apply_sync_filter()` partial hit 日志：
  - [x] 从 `按 manifest 部分解析` 改成 `按主项目 manifest 部分解析`（context 可用时）。
  - [x] 保持 `actual/total refnos` 格式，例如 `41/1175904 refnos`。
- [x] 更新 manifest-loaded-but-dbnum-missing skip 日志：
  - [x] 从 `不在 manifest 覆盖内` 改成 `不在主项目 manifest 覆盖内`（context 可用时）。
  - [x] 继续 `info!`，不恢复 warn。
- [x] 不删除底层 `All refnos count` 日志。

## T007 站点日志摘要口径

- [x] 修改 `src/web_server/managed_project_sites.rs::summarize_log_line()`：
  - [x] 优先识别 `按主项目 manifest 部分解析: actual/total refnos`。
  - [x] 摘要输出类似 `CATA 部分解析 41/1175904 refnos`。
  - [x] 识别 `不在主项目 manifest 覆盖内` skip 记录。
  - [x] 将 `All refnos count: N` 摘要降级为 `读取 refno 索引 N` 或等价文案。
- [x] 增加/更新单元测试：
  - [x] partial line 优先于 raw count line。
  - [x] raw count 仍有可读摘要但不冒充解析数量。
- [x] 验证：
  - `cargo test -q summarize_log_line_prefers_cata_actual_count --features web_server`

## T008 metrics 语义回归

- [x] 确认 `perf_metrics::note_parse_db_mode()` 与 `record_parse_db()` 后的 metrics：
  - [x] partial：`elements = actual`。
  - [x] partial：`total_in_file = full refno table size`。
  - [x] skipped：`elements = 0`。
  - [x] full：`elements = total_in_file`。
- [x] 测试策略：
  - `apply_sync_filter()` 已在 partial/skipped 分支写入 `note_parse_db_mode(dbnum, mode, full_count)`；
    `record_parse_db()` 由解析循环传入过滤后的 actual 元素数，并在无 note 时默认 full。
  - 未新增 `perf_metrics` 单测：当前 collector 通过全局 `OnceCell` 初始化，单测重置会要求额外重构；
    本轮用 focused code review + 既有 targeted tests 覆盖行为入口，避免为测试扩大变更面。

## T009 quicktest-250160-8080 验证

- [x] 重跑站点 parse。
- [x] 检查 manifest：
  - [x] `output/AvevaPlantSample/scene_tree/cata_closure.json` 存在。
  - [x] 不要求存在 `output/AvevaCatalogue/scene_tree/cata_closure.json`。
- [x] 检查日志：
  - [x] `acp7320_0001` 出现 `dbnum=7320 按主项目 manifest 部分解析: 41/1175904 refnos` 或等价 actual/total 口径。
  - [x] manifest 缺失 warning 不应出现在正常路径。
- [x] 检查 metrics：
  - [x] `7320 mode=partial elements=41 total_in_file=1175904`。
  - [x] manifest 覆盖的关联项目 CATA 不再回退 `mode=full`。

### T009 现场验证记录（2026-06-13）

- [x] 主项目 manifest 路径验证通过：
  - `dist/package/Plant3D-AIOS-win-x64/release/runtime/admin_sites/quicktest-250160-8080/output/AvevaPlantSample/scene_tree/cata_closure.json` 可读取。
  - `dist/package/Plant3D-AIOS-win-x64/release/runtime/admin_sites/quicktest-250160-8080/output/AvevaCatalogue/scene_tree/cata_closure.json` 不存在，符合“主项目 manifest 为权威来源”的预期。
- [x] 当前源码重跑 parse 已完成：
  - Admin API reconcile/parse 路径本轮超时，无法通过站点 API 触发可信重跑。
  - 第一次 `cargo run` 由于 PowerShell 外层 `$env:` 被展开，遗留了错误启动的 `cargo` / `target/debug/aios-database.exe` 子进程；已按进程路径确认并终止本轮遗留进程。
  - 使用隔离 `CARGO_TARGET_DIR=target/009-validation` 可避开 exe 锁，但触发 RocksDB 重新链接，失败于 `fatal error LNK1180: 没有足够的磁盘空间完成链接`；日志保存在 `specs/009-main-project-cata-manifest-logging/evidence/parse-current-source-isolated.log`。
  - 已有 dist 运行日志 `runtime/admin_sites/quicktest-250160-8080/logs/parse.log` 是旧行为证据：最新 `parse-20260612-232352` 运行已覆盖 `covered_dbnums=[7000, 7001, 7014, 7320, 250193, 250700, 250701]`，但 parse 阶段仍显示旧口径，例如 `7014 mode=full`，且 `acp7320_0001` 只记录到 `All refnos count: 1175904`，不能作为 009 当前源码验收通过证据。
  - 清理本轮失败的 `target/009-validation` 后，用默认 target 成功重跑当前源码：
    - `specs/009-main-project-cata-manifest-logging/evidence/parse-current-source-after-clean.log`
    - `specs/009-main-project-cata-manifest-logging/evidence/parse-current-source-after-clean-metrics.json`
  - 为捕获 `log::info!` 诊断行，使用 `-v` + `AIOS_LOG_TO_CONSOLE=1` 成功重跑：
    - `specs/009-main-project-cata-manifest-logging/evidence/parse-current-source-verbose.log`
    - `specs/009-main-project-cata-manifest-logging/evidence/parse-current-source-verbose-metrics.json`
  - 验收关键证据：
    - 日志：`dbnum=7320 按主项目 manifest 部分解析: 41/1175904 refnos`。
    - metrics：`dbnum=7320, db_type=CATA, elements=41, total_in_file=1175904, mode=partial`。
    - metrics 中 manifest 覆盖的关联项目 CATA（如 `7000/7001/7014/7320/250700/250701`）均为 `mode=partial`。

## T010 fail-open 验证

- [x] 临时移除 / 重命名主项目 manifest。
  - 本轮未改动真实 manifest 文件；通过显式设置不存在路径 `cata_closure.missing-for-t010.json` 模拟同等缺失场景，避免污染站点输出。
- [x] 重跑 parse。
  - 命令使用当前源码、`-v`、`AIOS_LOG_TO_CONSOLE=1`、`AIOS_CATA_CLOSURE_MODE=manifest`、不存在的 `AIOS_CATA_CLOSURE_MANIFEST_PATH`。
  - evidence：
    - `specs/009-main-project-cata-manifest-logging/evidence/fail-open-missing-manifest.log`
    - `specs/009-main-project-cata-manifest-logging/evidence/fail-open-missing-manifest-metrics.json`
- [x] 确认不会静默少解析 CATA。
  - warning 明确写出缺失 manifest 后 `CATA 整库回退`。
  - metrics 已记录缺失 manifest 下 `250193/7000/7014` 等 CATA 为 `mode=full`，例如 `7000 elements=547720 total_in_file=547720 mode=full`、`7014 elements=1714 total_in_file=1714 mode=full`。
  - `7320` 全量回退已进入 `refnos=1175904` 解析；因完整 full run 超过 30 分钟仍在运行，本轮为保护机器资源手动停止。该中止不影响 fail-open 语义验证：缺失 manifest 没有裁剪/跳过，而是进入 full 解析路径。
- [x] 确认 warning 包含：
  - [x] explicit manifest path。
  - [x] main project。
  - [x] current project。
  - [x] fallback reason。
  - 关键行：
    - `AIOS_CATA_CLOSURE_MODE=manifest 但 manifest 不存在: path=.../cata_closure.missing-for-t010.json main_project=AvevaPlantSample current_project=AvevaPlantSample source=explicit（CATA 整库回退）`
    - `AIOS_CATA_CLOSURE_MODE=manifest 但 manifest 不存在: path=.../cata_closure.missing-for-t010.json main_project=AvevaPlantSample current_project=AvevaCatalogue source=explicit（CATA 整库回退）`

## T011 回归门禁

- [x] `cargo check -q --features web_server`
- [x] `scripts/guard/web_server_parse_boundary_guard.ps1`
- [x] targeted Rust format check：
  - `rustfmt --edition 2024 --check src/data_interface/cata_closure.rs src/web_server/managed_project_sites.rs`
- [ ] 如改动触及 UI 摘要展示，重建 admin UI / static assets，并确认只提交预期产物。
- [x] 如存在现有 dirty code changes，提交前按文件范围复核，避免混入无关修改。

## T012 文档收口

- [x] 若实现完成，更新 `specs/009-main-project-cata-manifest-logging/tasks.md` 任务状态。
- [x] 若现场验证通过，记录新的 parse log / metrics 关键行。
- [x] 如发现 008 文档中 runtime manifest 假设仍会误导，追加 note 指向 009。
