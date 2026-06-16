# Contract: Custom Project Output Namespace

## Purpose

Define externally verifiable behavior for managed-site identity handling, generated DbOption files, active output paths, CATA partial parse alignment, and generation precheck source resolution. The managed project name is the platform namespace for a collection of one or more E3D source projects, not a replacement for source E3D project names.

## Managed Site Configuration Contract

### Inputs

A managed site may contain:

- Custom deployment `project_name`, for example `9002`, used as database/output/external access namespace.
- One or more source E3D project names/paths, for example `AvevaPlantSample`.
- `manual_db_nums`, for example `[250160]`.
- Parse flags such as `parse_db_types`, `auto_parse_related_dbnums`, and `cata_partial_parse`.

### Required generated configuration behavior

Generated DbOption files MUST satisfy:

```toml
project_name = "<custom deployment project name>"
included_projects = ["<source E3D project name>", ...]
project_dirs = ["<source E3D project path>", ...]
```

For the observed scenario:

```toml
project_name = "9002"
included_projects = ["AvevaPlantSample"]
```

The exact `project_dirs` value depends on local filesystem layout but MUST point to the real E3D source project directory, not a synthetic `9002` directory unless that is also the real E3D source path.

The custom deployment `project_name` MUST remain valid even when it matches none of the source E3D project names.

## Active Output Path Contract

For site `quicktest-250160-3-8082` with project code `9002` and custom project name `9002`, active output readers and writers MUST use:

```text
runtime/admin_sites/9002/quicktest-250160-3-8082/output/9002/
```

The following active artifacts MUST resolve under that namespace after re-parse/redeploy:

- `scene_tree/cata_closure.json`
- `scene_tree/db_index.sqlite`
- `parquet/`
- generated scene-tree files used by the viewer

Historical artifacts under:

```text
output/AvevaPlantSample/
```

MAY remain on disk but MUST NOT be selected as the active generated-output namespace for the redeployed custom-name site.

## CATA Manifest Alignment Contract

### Success path

When:

- `cata_partial_parse = true`
- closure generation succeeds
- active manifest is readable from `output/<custom_project_name>/scene_tree/cata_closure.json`

Then the final parse configuration MUST:

- keep non-CATA entries that were otherwise selected
- keep target DESI/manual entries
- keep only CATA DB files whose DB numbers are present in `manifest.by_dbnum`
- write a log line with the manifest path and CATA before/after counts

### Failure/fallback path

When the manifest is missing or unreadable:

- The system MUST NOT claim CATA narrowing succeeded.
- The system MUST keep existing conservative/fail-open parse behavior.
- The system MUST log the attempted manifest path and the reason for fallback.

## Parse Type Selection Contract

Persisted `parse_db_types` MUST represent operator intent:

- Quick/scoped deploy stores its scoped/default preset.
- Full system parsing stores all supported types only when the full system preset or equivalent explicit selection is chosen.
- Edit/clone flows MUST NOT turn an older empty or scoped value into full system parsing without explicit admin action.

## Generation Precheck Source Resolution Contract

### Success path

When:

- `project_name = "9001"` is the managed deployment/output identity
- `included_projects = ["AvevaPlantSample"]`
- `project_dirs` contains a resolvable path for `AvevaPlantSample`
- model generation detects missing `indextree`/`db_meta_info` prerequisites

Then precheck repair MUST:

- use `AvevaPlantSample` as the source project for PDMS parse/tree generation
- not use `9001` as the source project unless `9001` is explicitly present in `included_projects` with a resolvable source path
- continue to write/read generated artifacts under the active custom output namespace for `9001`
- log the output project and source projects separately

### Failure path

When no source project from `included_projects` or legacy fallbacks can be resolved:

- The system MUST return a configuration error that names the unresolved output project and available `included_projects`.
- The system MUST NOT panic through `Option::unwrap()` or equivalent unchecked project path access.
- The job failure should preserve the normal sidecar failed status/event path.

## Observability Contract

For every CATA partial parse attempt, logs MUST include:

- active custom project output namespace
- manifest path attempted
- whether manifest was loaded
- CATA file count before alignment
- CATA file count after alignment, when loaded
- fallback reason, when not loaded
- generation precheck output project name
- generation precheck source project list
- missing source-project mapping errors, when repair cannot start
