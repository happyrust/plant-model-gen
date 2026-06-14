# Feature Specification: 主项目 CATA manifest 与真实解析数量日志

## User Need

管理站点在快速部署 / 快速重解析时，CATA closure manifest 应始终以“主项目 / 目标模型项目”为权威来源。解析流程遍历到关联项目（例如 `AvevaCatalogue`）时，不应改用关联项目自己的 `output/<project>/scene_tree/cata_closure.json` 路径，否则会因为 manifest 不存在而把已收窄的 CATA 库重新整库解析。

同时，站点日志和日志摘要应展示“实际需要解析的 refno 数量”，不能只显示整文件 refno 表数量。`All refnos count: 1175904` 这类底层索引日志会让用户误以为正在解析 1175904 个元素；在 manifest 部分解析模式下，用户更需要看到 `41/1175904` 这类“实际解析 / 文件总量”的口径。

## Field Evidence

现场站点目录：

```text
D:/work/plant-code/plant-model-gen-cata-closure/dist/package/Plant3D-AIOS-win-x64/release/runtime/admin_sites/quicktest-250160-8080
```

关键事实：

- `parse-plan-manifest.json` 已是 `FastReparse`，并包含 CATA closure 补入的 `acp7320_0001`。
- 最新 closure metrics 显示 `covered_dbnums` 中 `7320` 只需要 `41` 个 refno。
- 站点实际只有 `output/AvevaPlantSample/scene_tree/cata_closure.json`，没有 `output/AvevaCatalogue/scene_tree/cata_closure.json`。
- `DbOption-parse.toml` 的 `included_projects` 同时包含 `AvevaPlantSample` 和 `AvevaCatalogue`，解析会先处理主项目，再处理关联项目。
- 最新日志中 `acp7320_0001` 打印 `All refnos count: 1175904`，但没有出现期望的 `dbnum=7320 按 manifest 部分解析: 41/1175904 refnos`。
- 最新 metrics 中 `7014` 已记录为 `mode=full, elements=1714, total_in_file=1714`，说明至少部分 `AvevaCatalogue` CATA 文件没有被主项目 manifest 裁剪。

## Root Cause

### RC1: runtime filter 按当前遍历项目推导 manifest

`src/versioned_db/database.rs` 在每个 project 的解析循环中调用：

```rust
load_sync_filter(project.as_str(), &db_types_clone)
```

`load_sync_filter()` 进一步调用：

```rust
default_manifest_path(project_name)
```

这会把 `AvevaCatalogue` 下的 CATA 文件导向 `output/AvevaCatalogue/scene_tree/cata_closure.json`。但 closure pass 的语义是“主项目 DESI 出向引用闭包”，manifest 实际落在主项目 `AvevaPlantSample` 下。因此跨项目解析时 filter 丢失，触发整库回退。

### RC2: 008 规格中的 manifest path 假设被现场推翻

`specs/008-cata-closure-parse-plan-alignment/plan.md` 曾记录：

```text
Use cata_closure::default_manifest_path(&site.project_name) after gen-cata-closure
```

这个假设只覆盖 web_server 对齐 parse config 的阶段，不覆盖 aios-database runtime 遍历 `included_projects` 时的 per-project filter 加载。现场证明：最终 parse config 已包含 `acp7320_0001`，但 runtime 仍可能在 `AvevaCatalogue` 阶段找错 manifest。

### RC3: 日志摘要把索引总量当成解析进度

`src/web_server/managed_project_sites.rs::summarize_log_line()` 当前将：

```text
All refnos count: N
```

总结为：

```text
最近 refno 计数 N
```

该值来自 `parse_file_db_basic_data()` 读取出的整文件 refno 表，不等于 manifest 裁剪后的实际解析 refno 数量。对大 CATA 库，这个摘要会直接误导用户判断为“又全量解析了”。

## Grill-Me 决策记录

| 问题 | 推荐答案 | 决策理由 |
|---|---|---|
| Q1：manifest 的权威路径应该按当前 project 还是主项目？ | 按主项目 / 目标模型项目。 | closure seed 来自主项目 DESI，关联 CATA 项目只是被消费的数据源；按当前 project 会导致跨项目 filter 丢失。 |
| Q2：`All refnos count` 是否应该从日志里删除？ | 不删除，但降级为底层索引事实；必须追加更清晰的实际解析数量日志。 | 它对诊断文件索引仍有价值，但不能作为用户可见的解析数量口径。 |
| Q3：部分解析日志应显示什么？ | 显示 `actual/total`，例如 `dbnum=7320 按主项目 manifest 部分解析: 41/1175904 refnos`。 | 同时说明实际工作量和文件总规模，避免误读。 |
| Q4：metrics 中 `elements` 应该表示实际解析还是整文件总量？ | `elements` 表示实际解析数量；`total_in_file` 表示整文件 refno 总量。 | 当前 metrics 设计已经如此，应强化并用于 UI 展示。 |
| Q5：manifest 缺失时是否仍 fail-open 整库回退？ | 是，但必须在日志中带上正在查找的 manifest path 和主项目名。 | 不能因为路径错误静默少解析；但回退原因必须可诊断。 |
| Q6：是否需要复制 manifest 到每个关联项目目录？ | 不推荐作为正式方案。 | 复制容易产生陈旧副本；正式方案应传递显式主项目 manifest 路径或主项目名。 |
| Q7：若多个主项目一起解析怎么办？ | 本 feature 只覆盖单站点单主项目语义；多主项目需每个 parse job 带自己的 manifest context。 | 当前管理站点以一个 `site.project_name` 作为主项目，先修复已知边界。 |

## Requirements

1. 解析进程在 CATA partial 模式下必须使用主项目 / 目标模型项目的 `cata_closure.json` 作为唯一 manifest 来源，即使当前正在遍历关联项目目录。
2. web_server 启动 parse job 时必须把主项目 manifest context 显式传给 aios-database runtime；不得依赖 runtime 当前 project 名称隐式推导。
3. `load_sync_filter` 或其替代入口必须支持显式 manifest path；未传显式 path 时可保留旧的按 project 推导行为，以兼容非站点 CLI 场景。
4. manifest 缺失、解析失败或路径不可访问时，必须保留现有整库回退语义，并输出包含 manifest path、主项目名、当前 project、db type 的诊断日志。
5. CATA 部分解析命中时，日志必须输出实际解析数量与整文件总量，例如 `41/1175904 refnos`；该日志应出现在底层 `All refnos count` 之后，方便用户判断裁剪是否生效。
6. 站点日志摘要应优先展示实际解析数量；当同一文件后续出现 partial/skipped/full mode 记录时，不应继续用 `All refnos count` 作为“最近 refno 计数”的主要摘要。
7. parse metrics 必须保持 `elements = 实际解析数量`、`total_in_file = 整文件 refno 总量`、`mode = full | partial | skipped` 的语义，并覆盖跨项目 CATA 文件。
8. 修复不得影响非 CATA 的 DICT / DESI / SYST 解析范围，不得取消 CATA closure 缺失时的 fail-open 回退。

## Non-Goals

- 不改变 CATA closure BFS / refno 闭包算法。
- 不改变 `included_db_files` 的文件级 parse plan 对齐策略，除非为了传递主项目 manifest context 必须补充字段。
- 不移除 `All refnos count` 的底层日志。
- 不把临时复制 manifest 到关联项目目录作为正式方案。
- 不处理多主项目同一 parse job 的新语义。

## Acceptance Criteria

- 对 `quicktest-250160-8080` 重跑 parse：
  - `output/AvevaPlantSample/scene_tree/cata_closure.json` 仍是唯一主项目 manifest。
  - `AvevaCatalogue/acp7320_0001` 解析时出现类似 `dbnum=7320 按主项目 manifest 部分解析: 41/1175904 refnos` 的日志。
  - parse metrics 中 `7320` 为 `mode=partial`，`elements=41`，`total_in_file=1175904`。
  - `7014` 等 manifest 覆盖的关联项目 CATA 库不再因为 project path 切换而退回 `mode=full`。
- 站点 UI / 日志摘要不再仅展示 `最近 refno 计数 1175904` 作为用户可见的解析状态；有 partial 记录时优先展示实际解析数量。
- 人为移除 manifest 后重跑 parse：
  - parse 不静默少解析；
  - 日志明确说明主项目 manifest path 缺失并执行整库回退。
- `cargo check -q --features web_server` 通过。
- `scripts/guard/web_server_parse_boundary_guard.ps1` 继续通过，确保 web_server 未新增 E3D 文件解析职责。
