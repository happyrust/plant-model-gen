# Implementation Plan: Site Project Isolation And Post-Deploy Editing

**Branch**: `014-site-project-isolation-and-editing` | **Date**: 2026-06-15 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/014-site-project-isolation-and-editing/spec.md`

## Summary

Change managed-site storage so the effective SurrealDB data path is based on a normalized `project_name`, surface partial parse options consistently in create/edit/detail UI, and allow stopped deployed sites to rename `project_name` with a controlled local migration of managed data/config artifacts. Keep `site_id` as the stable registry and process identity.

## Technical Context

**Language/Version**: Rust backend with Axum web server, SQLite admin registry, SurrealDB process management, Vue 3 admin frontend.

**Primary Dependencies**: `src/web_server/managed_project_sites.rs`, `src/web_server/models.rs`, `ui/admin/src/components/sites/SiteDrawer.vue`, `ui/admin/src/components/sites/SiteConfigSections.vue`, `ui/admin/src/types/site.ts`, admin sites HTTP API.

**Storage**: Existing admin registry SQLite row keyed by `site_id`; local runtime under `runtime/admin_sites/<site_id>`; current DB path is `runtime/admin_sites/<site_id>/data/surreal.db`. New project data identity should be derived from `project_name` while preserving `site_id` registry identity.

**Testing**: Do not create or run `cargo test`. Validate with a running web_server and HTTP/POST/PUT requests. Inspect generated JSON/TOML/filesystem paths. Frontend validation can be done through the admin UI or UI automation.

**Target Platform**: Windows local admin deployment first, with portable path rules for Linux remote defaults.

**Performance Goals**: Rename should be metadata/directory move only and complete in seconds for normal local directories; no full reparse/regenerate required.

**Constraints**: Do not rename source E3D project directories in v1. Do not auto-migrate remote production DB data. Do not move DB data while processes are active. Preserve quick-deploy compatibility.

**Scale/Scope**: One site rename at a time via existing operation lock; one owning site per project-name-derived DB path.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **No cargo tests for web_server**: PASS. Validation uses running service + HTTP/POST/PUT.
- **aios-database validation via CLI + JSON when needed**: PASS. Config/output verification is file/JSON/TOML based.
- **No destructive source E3D rename by default**: PASS. v1 stays within managed runtime artifacts.
- **Process safety before directory move**: PASS. Rename requires stopped/non-active site.

No gate violations.

## Project Structure

### Documentation (this feature)

```text
specs/014-site-project-isolation-and-editing/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── managed-site-project-rename-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/web_server/
├── models.rs
├── managed_project_sites.rs
└── admin_handlers.rs

ui/admin/src/
├── types/site.ts
├── components/sites/SiteDrawer.vue
└── components/sites/SiteConfigSections.vue
```

**Structure Decision**: Keep the core migration and path ownership rules in `managed_project_sites.rs`; keep request/response shape in `models.rs`; keep UI exposure in site drawer/detail components. Do not add a new service layer unless migration logic grows beyond this feature.

## Phase 0: Research

Output: [research.md](./research.md)

Resolved decisions:

- `site_id` remains immutable; `project_name` becomes the mutable human-facing data identity.
- New DB paths should be produced by a helper that normalizes `project_name` into a filesystem-safe slug.
- Rename is rejected unless the site is stopped and no parse/generate/web/viewer/db process is active.
- Rename moves only managed local artifacts in v1.
- Partial parse controls already exist in `SiteDrawer.vue`; detail display and preset propagation need tightening.

Open decisions requiring user confirmation:

- Whether a later version should rename original E3D source DB folders/files under `project_path`.
- Whether remote deployed DB directories should be migrated automatically or only on explicit remote redeploy.

## Phase 1: Design & Contracts

Output:

- [data-model.md](./data-model.md)
- [contracts/managed-site-project-rename-contract.md](./contracts/managed-site-project-rename-contract.md)
- [quickstart.md](./quickstart.md)

## Implementation Approach

1. Add helper(s) for project data identity normalization and project-name-based DB data path derivation.
2. Change new site creation to set `db_data_path` from `project_name`.
3. Add conflict checks for project-name data path ownership.
4. Extend `update_site` project-name path to detect rename, require inactive site, move managed local data/output artifacts, update registry row, and rewrite config files.
5. Add rollback-safe ordering: validate target paths, move directories to temporary staging where needed, persist row, rewrite config, then finalize old path cleanup.
6. Make site detail display partial parse configuration.
7. Ensure presets and form payloads include `cata_partial_parse` explicitly where defaults matter.
8. Validate through HTTP create/update/start flows and admin UI inspection.

## Post-Design Constitution Check

- **No cargo tests for web_server**: PASS. Quickstart uses HTTP calls and artifact inspection.
- **Managed artifacts only**: PASS. Source E3D files are out of v1 scope.
- **Safe rename boundary**: PASS. Active processes block rename.
- **Backward compatibility**: PASS. Existing unchanged names do not force migration unless explicitly implemented with logging.

## Complexity Tracking

No constitution violations. The main complexity is atomic local folder migration on Windows; if full rollback proves risky, reduce v1 to "rename allowed only before parse/deploy" or require explicit redeploy reset after project-name change.
