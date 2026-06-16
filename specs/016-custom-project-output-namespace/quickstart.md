# Quickstart: Validate Custom Project Output Namespace

## Goal

Validate that a managed site uses the admin-provided custom project name as its active output namespace while preserving E3D source project names in `included_projects`, and that CATA closure manifest alignment narrows the final CATA parse plan.

## Prerequisites

- Admin service is runnable from the source tree or packaged release.
- A site can be created or reused with:
  - custom project name: `9002`
  - source E3D project: `AvevaPlantSample`
  - `manual_db_nums=[250160]`
  - `auto_parse_related_dbnums=true`
  - `cata_partial_parse=true`
- Admin authentication token is available when calling protected endpoints.

Do not use `cargo test` or Rust test-target compilation for `web_server` validation.

## Scenario 1: Generated DbOption identity split

1. Create or edit a managed site so the deployment project name is `9002` and the source E3D project remains `AvevaPlantSample`.
2. Trigger parse preview or redeploy so `DbOption-parse.toml` is regenerated.
3. Inspect the generated TOML.

Expected:

```toml
project_name = "9002"
included_projects = ["AvevaPlantSample"]
```

`project_dirs` should point at the real source E3D project path.

## Scenario 2: Active output namespace

1. Re-run parse/redeploy for the site.
2. Inspect the site runtime output folder.

Expected active files:

```text
runtime/admin_sites/9002/quicktest-250160-3-8082/output/9002/scene_tree/cata_closure.json
runtime/admin_sites/9002/quicktest-250160-3-8082/output/9002/scene_tree/db_index.sqlite
```

If stale files exist under `output/AvevaPlantSample`, they may remain, but active readers should use `output/9002`.

## Scenario 3: CATA closure alignment

1. Re-run parse with `manual_db_nums=[250160]` and `cata_partial_parse=true`.
2. Inspect `logs/parse.log`.
3. Inspect final `DbOption-parse.toml` or parse-plan artifact after closure alignment.

Expected:

- Log shows manifest path under `output/9002/scene_tree/cata_closure.json`.
- Log shows CATA count before and after alignment.
- Final CATA plan contains only manifest-covered CATA DB numbers.
- For the known observed closure, expected covered CATA DB numbers are:

```text
7015, 250124, 250162, 250193
```

Manifest-outside CATA DB files should not remain in the final parse config after successful manifest alignment.

## Scenario 4: No historical migration

1. Ensure a stale folder such as `output/AvevaPlantSample` exists.
2. Re-run parse/redeploy.
3. Compare old and active folders.

Expected:

- Old folder is not migrated or deleted automatically.
- New active generated files appear under `output/9002`.
- Service reads active files from `output/9002`.

## Scenario 5: Parse DB type persistence

1. Create a quick/scoped deploy site.
2. Save and reload the site in admin UI/API.
3. Clone or edit the site without choosing full system parsing.
4. Inspect persisted `parse_db_types`.

Expected:

- Scoped/default selection remains scoped/default.
- Full type set is saved only when full system parsing is selected.
- Detail UI clearly distinguishes full system parsing from scoped/custom parsing.

## Scenario 6: Model generation precheck after project rename

1. Use or create a managed site matching the observed failure:
   - site id: `quicktest-250160-8080`
   - custom deployment project name: `9001`
   - source E3D project: `AvevaPlantSample`
   - package/runtime root: `dist/package/Plant3D-AIOS-win-x64/release`
2. Ensure tree/db-meta prerequisites are missing or stale enough for generation to trigger automatic repair.
3. Trigger model generation from the admin site detail page or equivalent admin API.
4. Inspect the sidecar generation log.

Expected:

- Log must not show `gen_tree_only 模式, 项目: 9001, 类型: DESI` unless `9001` is also a real configured source project.
- Log should show output project `9001` and source project list containing `AvevaPlantSample`.
- No panic appears at `src/versioned_db/database.rs` from `Option::unwrap()` on project path lookup.
- If source project resolution is invalid, the error identifies missing source mapping and includes `included_projects`.
- If source project resolution is valid, precheck regenerates prerequisites and model generation proceeds beyond tree/db-meta repair.

## Evidence To Record

For completion, record:

- Admin site id and project code.
- Generated `DbOption-parse.toml` identity fields.
- Active `cata_closure.json` path.
- CATA before/after count log lines.
- Final CATA DB list in parse config.
- Parse DB type values before and after edit/clone/quick deploy flows.
- Generation precheck source project log lines.
- Sidecar binary path/hash used by the packaged release.
- Generation job id, final status, and any remaining failure reason after replacing the packaged sidecar.
