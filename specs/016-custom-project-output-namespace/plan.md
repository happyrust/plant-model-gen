# Implementation Plan: Custom Project Output Namespace

**Branch**: `[016-custom-project-output-namespace]` | **Date**: 2026-06-16 | **Spec**: `specs/016-custom-project-output-namespace/spec.md`

**Input**: Feature specification from `/specs/016-custom-project-output-namespace/spec.md`

## Summary

Unify managed-site runtime artifacts around the admin-provided custom project name as a collection-level namespace while preserving original E3D project identities for source scanning. A managed project may contain multiple E3D source projects; its name owns database, directory, and external access naming, but does not need to match any E3D project name. Fix CATA partial parsing so web_server reads the closure manifest from the same custom output namespace where sidecar writes it, then use that manifest to narrow the final CATA parse plan. Ensure model-generation prechecks regenerate tree/db-meta prerequisites from source E3D projects rather than deployment/output names. Add guardrails so quick deploy/create/edit do not silently expand scoped parse selections into full CATA/system parsing.

## Technical Context

**Language/Version**: Rust backend; Vue 3 + TypeScript admin UI.

**Primary Dependencies**: Axum web server/admin API, rusqlite-backed managed-site registry, existing parse sidecar client, existing DbOption TOML generation, existing Vue admin site components.

**Storage**: Admin SQLite `deployment_sites.sqlite`, managed runtime directories under `runtime/admin_sites`, generated DbOption TOML files, output folders under `runtime/admin_sites/<project_code>/<site_id>/output/<project_name>`.

**Testing**: No `cargo test` or Rust test-target compilation for `web_server`. Validate through running service HTTP/POST flows, CLI/json checks, generated TOML inspection, runtime file/log inspection, and targeted TypeScript checks if UI code changes.

**Target Platform**: Windows local admin/deploy package and source development environment.

**Project Type**: Rust web service + Vue admin frontend.

**Performance Goals**: Path and config generation remain constant-time relative to site record size. CATA plan alignment should only process parse-plan entries and manifest DB keys, not scan all E3D files in web_server.

**Constraints**: Preserve older output folders without migration. Do not mutate original E3D source project names or paths. Keep manifest-missing fail-open behavior. Do not introduce E3D file parsing responsibility into `web_server`.

**Scale/Scope**: Managed admin deployment sites, including single-project and multi-project source configurations.

## Constitution Check

The repository rule forbids `cargo test`/test-target validation for `web_server` and requires running service + HTTP/POST validation. This plan follows that rule. `.specify/memory/constitution.md` contains placeholders only and adds no further gates.

## Project Structure

### Documentation (this feature)

```text
specs/016-custom-project-output-namespace/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── custom-project-output-namespace-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── data_interface/
│   └── db_meta_manager.rs             # db_meta loading and automatic indextree/db-meta regeneration
├── versioned_db/
│   └── database.rs                    # source project path resolution during parse/tree generation
├── web_server/
│   ├── managed_project_sites.rs       # project identity helpers, DbOption writing, output/manifest paths
│   ├── models.rs                      # only if payload/response fields need explicit documentation or defaults
│   └── admin_handlers.rs              # only if API response/route behavior changes
└── parse_sidecar.rs                   # only if preview/source-project handling needs sidecar payload adjustment

ui/admin/src/
├── api/sites.ts
└── components/sites/
    ├── SiteDrawer.vue
    ├── SiteConfigSections.vue
    └── parse-db-types.ts

scripts/
└── smoke/                             # optional HTTP/CLI smoke script if manual commands become repetitive
```

**Structure Decision**: Keep implementation inside existing managed-site backend and admin UI modules. Add helper functions before creating a new Rust module; this feature is primarily correcting identity semantics and path selection, not adding a new subsystem.

## Technical Approach

### 1. Split deployment identity from source identity

Current helper semantics must be made explicit:

- Deployment identity: custom `site.project_name`, used for runtime database name, generated DbOption `project_name`, output namespace, external access name, viewer project parameter, parquet validation root, scene-tree root, and CATA manifest lookup.
- Source identity: one or more source E3D project names from `site.projects`, `associated_project`, or canonical project path, used for `included_projects`, `project_dirs`, DB file discovery, and parse preview source roots.

The deployment identity is the platform project namespace for the whole source collection. It is not required to equal any source E3D project name, and code should not infer source project paths from it.

Recommended helper names:

- `site_deployment_project_name(site)` returns validated custom deployment name.
- `site_source_project_names(site)` returns original E3D source names.
- `site_active_output_project_name(site)` delegates to deployment name and is used by every output lookup.

Avoid using `site_source_project_name(site)` for active output paths.

### 2. Generate DbOption files with mixed identity intentionally

Generated DbOption TOML should contain:

- `project_name = <custom deployment project name>`
- `output_root = <site output root>`
- `included_projects = [<E3D source project names>]`
- `project_dirs = [<E3D source project dirs>]`

This preserves source scanning while causing sidecar output to land under `output/<custom_project_name>`. The same custom name also remains the database/external project identity exposed by the managed deployment.

### 3. Align output consumers to the same namespace

Audit and update consumers that currently use source identity for generated output:

- `site_project_tree_dir`
- `cata_manifest_path_for_site`
- `cata_index_path_for_site`
- parquet validation root
- viewer URL project parameter
- scene tree/detail payload paths
- any generated manifest/log metadata that reports project output path

All active generated-output consumers should use the custom deployment identity.

### 4. Keep old output as non-authoritative history

Do not migrate `output/<source_e3d_name>`. After a new parse/generate run, the active output is `output/<custom_project_name>`. If old and new output both exist, service reads the active custom output path.

### 5. Preserve CATA partial parse fail-open behavior

When CATA closure succeeds and manifest is readable:

- Read manifest from `output/<custom_project_name>/scene_tree/cata_closure.json`.
- Narrow final parse plan CATA entries to manifest-covered DB numbers.
- Rewrite `DbOption-parse.toml` before the parse job starts.
- Log manifest path and CATA before/after file counts.

When manifest is missing or unreadable:

- Keep existing fail-open behavior.
- Log attempted path and explain that CATA narrowing did not happen.

### 6. Guard parse type selection semantics

Normalize parse DB type handling so empty/scoped selections do not accidentally mean full system parsing in quick deploy/create/edit flows. Full parsing remains valid only when the admin selects the full system preset or all supported types explicitly.

Admin UI should distinguish:

- quick/scoped preset
- custom explicit selection
- full system selection
- older empty record fallback

### 7. Keep generation precheck on source identity

Model generation precheck can call shared db-meta/tree generation helpers when prerequisites are missing. In managed-site configs, generated DbOption files intentionally contain a mixed identity:

- `project_name = <custom deployment/output name>`
- `included_projects = [<source E3D project names>]`
- `project_dirs = [<source E3D project dirs>]`

Therefore precheck and auto-repair code must not call parse/tree generation with `db_option.project_name` as the source project. It should:

- iterate resolvable `included_projects`;
- use `DbOption::get_project_path(project)` to guard each source project before invoking parse/tree repair;
- fail with an actionable error if no source project can be resolved;
- avoid `unwrap()` on project path lookup in parse/tree generation entry points.

The specific regression signal is `quicktest-250160-8080`: `project_name=9001`, source `included_projects=["AvevaPlantSample"]`, missing tree/db-meta, and model generation failed before geometry generation with `called Option::unwrap() on a None value`.

### 8. Make room computation optional

Room computation is a downstream spatial relationship analysis step and should not be mandatory for model generation success. Automatic room computation is disabled by default; it may be enabled only by an explicit operator/system opt-in such as `AIOS_AUTO_ROOM_COMPUTE=1`.

When disabled:

- generation should complete without launching a `room compute` sidecar job;
- room-compute logs should record the skip reason;
- deployment validation should treat the skipped room compute as non-blocking;
- CLI/manual room computation remains available for operators who need it.

## Validation Plan

- Reproduce the known site shape: custom project `9002`, source E3D `AvevaPlantSample`, `manual_db_nums=[250160]`, `cata_partial_parse=true`.
- Trigger re-parse/redeploy and inspect generated `DbOption-parse.toml`: `project_name` is `9002`, `included_projects` contains `AvevaPlantSample`.
- Inspect generated files: active closure manifest exists at `output/9002/scene_tree/cata_closure.json`.
- Inspect parse log: manifest read path is `output/9002/...`; CATA before/after count is printed.
- Inspect final parse config/manifest: manifest-outside CATA DB files are removed after alignment.
- Validate that stale `output/AvevaPlantSample` is ignored and not migrated.
- Validate quick deploy/create/edit save parse DB types without unintended full-system expansion.
- Validate model generation on `quicktest-250160-8080` or equivalent renamed site after deleting/missing tree/db-meta prerequisites:
  - generation precheck logs source project `AvevaPlantSample`, not deployment name `9001`;
  - no `Option::unwrap()` panic occurs in source-project path resolution;
  - result is either successful prerequisite regeneration and continued generation, or a clear configuration error if source projects are invalid.
- Validate generation with automatic room computation disabled:
  - no `room compute` sidecar job is launched after successful generation;
  - `room-compute.log` records that automatic room computation was skipped;
  - site generation status is not failed by room-compute cleanup.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Mixed identity in one generated config | Runtime output must use custom name while source scanning must use E3D names | Using only one name either breaks source scanning or recreates the output path mismatch |
| Non-migration of historical output | User explicitly does not require migration | Migrating old output would add risk and scope without fixing redeploy behavior |
