# Implementation Plan: 主项目 CATA manifest 与真实解析数量日志

## Goal

修正 008 之后现场暴露出的第二层偏差：

- 008 已经让最终 parse plan 按 closure manifest 收窄 CATA 文件列表。
- 009 要保证 aios-database runtime 在遍历关联项目时仍读取**主项目 / 目标模型项目**的 `cata_closure.json`。
- 009 同时把日志和站点摘要从“整文件 refno 表数量”改为优先展示“实际解析数量 / 整文件总量”。

## Current Flow

```text
web_server spawn_parse_process()
  -> gen-cata-closure
       writes output/<main_project>/scene_tree/cata_closure.json
  -> align parse plan with manifest
       final DbOption-parse.toml includes only manifest-covered CATA files
  -> start aios-database parse job with AIOS_CATA_CLOSURE_MODE=manifest
       database.rs iterates included_projects
       load_sync_filter(project.as_str(), db_types)
       default_manifest_path(current_project)
```

Observed failure:

```text
main project: AvevaPlantSample
current traversed project: AvevaCatalogue
expected manifest: output/AvevaPlantSample/scene_tree/cata_closure.json
actual derived manifest: output/AvevaCatalogue/scene_tree/cata_closure.json
```

This makes the runtime filter disappear in the associated CATA project and
can turn a manifest-covered CATA file back into full parsing.

## Target Flow

```text
web_server spawn_parse_process()
  -> compute main_project_manifest_path once from site.project_name
  -> pass explicit manifest context to parse job env
       AIOS_CATA_CLOSURE_MODE=manifest
       AIOS_CATA_CLOSURE_MANIFEST_PATH=<main project cata_closure.json>
       AIOS_CATA_CLOSURE_MAIN_PROJECT=<site.project_name>
  -> aios-database load_sync_filter()
       prefer explicit manifest path if env is set
       fallback to default_manifest_path(current_project) for legacy CLI usage
  -> apply_sync_filter()
       logs dbnum actual/total using main-project manifest wording
  -> web_server log summary
       prioritizes actual partial/skipped/full parse mode lines over raw All refnos count
```

## Design

### 1. Introduce explicit runtime manifest context

Keep `AIOS_CATA_CLOSURE_MODE=manifest` as the on/off switch.

Add optional env vars:

```text
AIOS_CATA_CLOSURE_MANIFEST_PATH
AIOS_CATA_CLOSURE_MAIN_PROJECT
```

Resolution rule:

1. If `AIOS_CATA_CLOSURE_MANIFEST_PATH` is non-empty, use it.
2. Else fallback to `default_manifest_path(project_name)` as today.

This keeps existing CLI / non-site parse jobs compatible while giving
managed sites a stable manifest source across `included_projects`.

### 2. Extend filter metadata without disrupting call sites

Current `CataClosureFilter` is effectively:

```rust
dbnum -> HashSet<RefU64>
```

009 should wrap that map with context:

```rust
pub struct CataClosureFilter {
    pub by_dbnum: BTreeMap<u32, HashSet<RefU64>>,
    pub manifest_path: PathBuf,
    pub main_project: Option<String>,
}
```

If changing the alias causes too much churn, use an internal wrapper and keep
small helper methods:

```rust
filter.allowed(dbnum)
filter.len()
filter.manifest_path()
filter.main_project()
```

The public behavior remains the same: `None` means full-parse fallback.

### 3. Pass the main project manifest from web_server

In `src/web_server/managed_project_sites.rs::spawn_parse_process()`:

```rust
let mut parse_env = metrics_env;
if cata_partial_enabled {
    parse_env.insert("AIOS_CATA_CLOSURE_MODE".to_string(), "manifest".to_string());
    parse_env.insert(
        "AIOS_CATA_CLOSURE_MANIFEST_PATH".to_string(),
        cata_manifest_path_for_site(&site).display().to_string(),
    );
    parse_env.insert(
        "AIOS_CATA_CLOSURE_MAIN_PROJECT".to_string(),
        site.project_name.clone(),
    );
}
```

`cata_manifest_path_for_site(&site)` already points at the runtime output for
the managed site's target project, so this avoids deriving the manifest from
the currently traversed associated project.

### 4. Improve runtime diagnostics

Update `load_sync_filter()` diagnostics to include:

- resolved manifest path;
- current project name;
- optional main project name;
- whether the path was explicit env or fallback derived.

Expected missing-manifest warning:

```text
[cata_closure] AIOS_CATA_CLOSURE_MODE=manifest but manifest missing:
path=<.../AvevaPlantSample/.../cata_closure.json>
main_project=AvevaPlantSample current_project=AvevaCatalogue source=explicit
（CATA 整库回退）
```

### 5. Make partial parse logs user-facing accurate

`apply_sync_filter()` already has both numbers:

- `filtered.len()` = actual parsed refnos;
- `before` = total refnos in file.

Change the partial log from generic manifest wording to main-project wording
when context is available:

```text
[cata_closure] dbnum=7320 按主项目 manifest 部分解析: 41/1175904 refnos
```

For full fallback because filter is missing, add a one-time load warning via
`load_sync_filter()`. Avoid printing per-file fallback noise unless needed for
diagnostics.

For manifest-loaded-but-dbnum-missing skip, keep `info!` as in 008:

```text
[cata_closure] dbnum=250166 不在主项目 manifest 覆盖内，按需跳过该 CATA 库（3 refnos 不解析）
```

### 6. Fix web_server log summary priority

`summarize_log_line("parse", ...)` currently turns:

```text
All refnos count: 1175904
```

into:

```text
最近 refno 计数 1175904
```

009 should add specific parsing before that fallback:

```rust
if line contains "按主项目 manifest 部分解析:" {
    return Some("CATA 部分解析 41/1175904 refnos".to_string());
}
if line contains "不在主项目 manifest 覆盖内" {
    return Some("CATA manifest 跳过未引用库".to_string());
}
```

Then keep `All refnos count` as a low-priority raw-index summary:

```text
读取 refno 索引 1175904
```

This preserves diagnostic value without presenting the index total as the
actual parse workload.

### 7. Metrics semantics

Keep existing metrics contract:

- `mode=partial`, `elements=<actual>`, `total_in_file=<before>`.
- `mode=full`, `elements=<actual/full>`, `total_in_file=<full>`.
- `mode=skipped`, `elements=0`, `total_in_file=<full>`.

Add/adjust tests only if current assertions cannot prove the associated
project case. Do not redefine `elements`.

## Test Plan

Unit tests:

- `load_sync_filter` uses `AIOS_CATA_CLOSURE_MANIFEST_PATH` over
  `default_manifest_path(current_project)`.
- missing explicit manifest logs fallback context and returns `None`.
- `apply_sync_filter` partial mode still returns only manifest-covered refnos.
- `summarize_log_line` prefers partial actual/total lines over `All refnos count`.

Manual / scenario validation:

```powershell
cargo check -q --features web_server
scripts/guard/web_server_parse_boundary_guard.ps1
```

For `quicktest-250160-8080`:

```text
output/AvevaPlantSample/scene_tree/cata_closure.json exists
output/AvevaCatalogue/scene_tree/cata_closure.json may be absent

acp7320_0001 parse log:
  dbnum=7320 按主项目 manifest 部分解析: 41/1175904 refnos

metrics:
  7320 mode=partial elements=41 total_in_file=1175904
```

Fail-open validation:

```text
remove / rename main project cata_closure.json
parse does not silently drop CATA
log includes explicit manifest path + main/current project context
```

## Risks

### R1: Env var leakage between jobs

`parse_env` is per sidecar job invocation. Ensure the new env vars are only
added for `cata_partial_enabled`.

### R2: Path string portability

Pass absolute runtime path from web_server. Avoid current-directory relative
paths because aios-database process cwd may differ from web_server cwd.

### R3: Non-site CLI behavior

Do not require `AIOS_CATA_CLOSURE_MANIFEST_PATH`; fallback to
`default_manifest_path(project_name)` preserves legacy CLI behavior.

### R4: Log parser brittleness

`summarize_log_line()` is string-based. Add tests for exact expected strings
and keep parsing tolerant of the emoji / prefix being absent.

## Rollout

1. Implement explicit manifest path env support.
2. Pass main project manifest context from managed site parse job.
3. Update partial/skipped log wording.
4. Update log summary parsing.
5. Run `cargo check -q --features web_server` and web_server boundary guard.
6. Re-run `quicktest-250160-8080` and verify logs + metrics.
