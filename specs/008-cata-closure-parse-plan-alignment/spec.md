# Feature Specification: CATA closure 与解析计划口径对齐

## User Need

管理站点开启 `auto_parse_related_dbnums` 与 CATA 按需部分解析后，解析日志不应反复出现大量类似：

```text
[cata_closure] dbnum=250166 不在 manifest 覆盖内，按需跳过该 CATA 库（3 refnos 不解析）
```

这类日志本身不是解析失败，但它暴露出两个阶段口径不一致：

- parse plan 在文件级把不需要的 CATA 库加入 `included_db_files`。
- `gen-cata-closure` 在 refno 级 manifest 中只覆盖真正被目标 DESI 引用到的 CATA refno。

用户需要解析计划、db_index、CATA closure manifest 三者使用一致的精确依赖口径，减少误导日志和不必要的大库 refno 表读取，同时保持按需解析零漏边。

## 现场证据

现场站点目录：

```text
D:/work/plant-code/plant-model-gen-cata-closure/dist/package/Plant3D-AIOS-win-x64/release/runtime/admin_sites/quicktest-250160-8080
```

关键事实：

- `DbOption-parse.toml` 中 `manual_db_nums = [250160]`，且 `included_db_files` 包含 `aps250166_0001`。
- `parse-plan-manifest.json` 中 `aps250166_0001` 的来源是 `source = "auto_related"`、`priority = 40`。
- `output/AvevaPlantSample/scene_tree/cata_closure.json` 只包含 `by_dbnum = { 250193: 16 refs }`，没有 `250166`。
- `parse.log` 显示 closure 结果为 `cata_dbs=1 visited=16 missing=44`，随后 `250166` 被 skip。
- `metrics/parse-20260612-215036.json` 显示 `250166` 为 `mode = "skipped"`、`elements = 0`、`total_in_file = 3`，整个 parse `success = true`、`error_count = 0`。
- 站点根 `runtime/admin_sites/.../db_index.sqlite` 当前 `db_dependency` 为空；而 `output/.../scene_tree/db_index.sqlite` 有 103 条依赖边，且 `250160` 的依赖中 CATA 只有 `250193`。

## Root Cause

### RC1: parse plan 粗粒度纳入所有 CATA

`src/parse_sidecar.rs::resolve_included_db_files()` 当前在 `auto_parse_related_dbnums=true` 时直接收集工程下所有 CATA 文件：

```rust
if auto_parse_related_dbnums {
    for root in project_roots {
        collect_db_file_names_for_types(root, &["CATA"], &mut file_names)?;
    }
}
```

这会把 `250166`、`7351`、`7355` 等并非当前目标 DESI 真正需要的 CATA 库写入 `included_db_files`。

### RC2: 站点级 db_index 在 manual target 场景下不记录依赖边

`src/parse_sidecar.rs::rebuild_db_index_request()` 当前在 `manual_db_nums` 非空时跳过 outbound 依赖收集：

```rust
let outbound = if payload.manual_db_nums.is_empty() {
    collect_design_outbound(&roots).await
} else {
    Vec::new()
};
```

因此 quick deploy / 单库目标站点的站点级 `db_index.sqlite` 可能只有 ref0 所属库映射，却没有 `db_dependency` 边，导致后续无法用它精确解释 auto-related 文件。

### RC3: closure manifest 没有反向收敛最终 parse config

`spawn_parse_process()` 已经按顺序执行：

1. 生成 parse plan 并写 `DbOption-parse.toml`
2. 跑 `gen-cata-closure`
3. 设置 `AIOS_CATA_CLOSURE_MODE=manifest`
4. 启动真正 parse job

但第 2 步生成的 manifest 没有回写到第 1 步的 `included_db_files`。因此即使 manifest 已经证明 CATA 只需要 `250193`，parse job 仍会遍历早先宽泛列表里的 `250166`。

## Grill-Me 决策记录

| 问题 | 推荐答案 | 决策理由 |
|---|---|---|
| Q1：`250166` skip 是错误还是裁剪行为？ | 裁剪行为，不应视为 parse 失败。 | parse metrics 成功，`mode=skipped` 表示 manifest 过滤生效。 |
| Q2：是否应继续在 preview plan 中加入所有 CATA？ | 否。 | 这会放大日志与 I/O，违背按需解析目标。 |
| Q3：最终权威来源应是谁？ | `cata_closure.json` 的 `by_dbnum` 是 CATA refno 级权威来源。 | 它由目标 DESI 出向引用闭包得到，粒度比库级 `auto_related` 更精确。 |
| Q4：如果 manifest 缺失或生成失败，是否仍整库回退？ | 是，保留现有 fail-open 回退。 | 缺 manifest 时不能静默少解析 CATA。 |
| Q5：manifest 为空时是否删除全部 CATA included files？ | 是，但仅在 closure job 成功且 manifest 可读时。 | 成功的空 manifest 表示目标 DESI 没有 CATA 覆盖；失败/缺失不能收窄。 |
| Q6：修复应放在 preview plan 还是 parse runtime？ | 两处都修，但 runtime manifest 收敛优先；preview 精确化延后到能读同一份 db_index 后。 | runtime 能用刚生成的 manifest 做最终真实配置；preview 若过早停止加入 CATA，又没有 manifest/db_index 补入能力，反而可能漏掉 `250193`。 |
| Q7：skip 日志应保留 warn 吗？ | 不应默认 warn。 | 它是正常裁剪，建议降为 info 或只进 metrics。 |
| Q8：是否依赖旧的全 CATA 列表保证零漏边？ | 不依赖。 | 零漏边由 closure + T007 惰性兜底 + verify-cata-closure 保障。 |

## Requirements

1. 最终进入 parse job 的配置不得再无条件包含所有 CATA 库；preview 阶段在没有精确 db_index 输入前允许保守偏宽。
2. `/db-index/rebuild` 在 `manual_db_nums` 非空时仍必须记录目标 DESI 的外部依赖边，只过滤 source，不跳过依赖构建。
3. `spawn_parse_process()` 在 `gen-cata-closure` 成功后，必须用 manifest 覆盖范围重写 parse config 中的 CATA `included_db_files`：保留/补入 manifest 覆盖的 CATA，移除 manifest 外 CATA。
4. manifest 读取失败、缺失或 closure job 失败时，不得收窄 CATA 文件列表；保持现有回退语义。
5. 非 CATA 预解析库（DICT 等）、manual DESI 目标、系统库补齐逻辑不得受影响。
6. parse metrics 中仍应记录每库 `mode = full | partial | skipped`；正常裁剪日志不得以 warn 误导用户。
7. 修复后 `quicktest-250160-8080` 这类单 DESI 快速部署只应解析 manifest 覆盖的 CATA 库（现场期望：`250193`），不应再遍历 `250166`。

## Non-Goals

- 不改变 CATA closure BFS 语义。
- 不取消 `AIOS_CATA_CLOSURE_MODE=manifest` 的 manifest 缺失回退。
- 不把 web_server 变成 E3D 数据解析方；web_server 只编排 sidecar、读 manifest/metrics 产物。
- 不在本 spec 修复 `generate.log` 中的 `The table 'ses' does not exist` 问题。
- 不做历史站点自动迁移；修复后通过重新解析/重新生成 parse config 生效。

## Acceptance Criteria

- 对 `quicktest-250160-8080` / `manual_db_nums=[250160]` 重跑 parse：
  - closure metrics 仍显示 `covered_dbnums = [{ dbnum: 250193, refnos: 16 }]`。
  - `DbOption-parse.toml` 在 closure 后的最终版本不再包含 `aps250166_0001` 这类 manifest 外 CATA。
  - parse metrics 中不再出现 `dbnum=250166 mode=skipped`。
  - parse job `success=true`、`error_count=0`。
- 站点级 `db_index.sqlite` 对 `250160` 能记录至少 `250193` 这条 CATA 依赖边。
- manifest 缺失/closure 失败场景仍按现有路径失败或回退，不因收窄逻辑产生静默少解析。
- `scripts/guard/web_server_parse_boundary_guard.ps1` 继续通过，确保 web_server 未新增 E3D 文件解析职责。
