# Implementation Plan: Site Project Identity & Parse Configuration

**Branch**: `[014-site-project-identity-config]` | **Date**: 2026-06-15 | **Spec**: `specs/014-site-project-identity-config/spec.md`

**Input**: Feature specification from `/specs/014-site-project-identity-config/spec.md`

## Summary

Make project name a managed, editable deployment identity without changing immutable site id. Newly generated runtime/database/output paths and E3D database names should be project-name scoped; existing sites remain compatible and gain an explicit rename workflow. Surface dependency partial parse and CATA partial parse consistently in create, edit, preview, and details UI.

## Technical Context

**Language/Version**: Rust backend; Vue 3 + TypeScript admin UI.

**Primary Dependencies**: Axum web server/admin API, rusqlite-backed admin registry, existing parse sidecar client, existing Vue admin store/API layer.

**Storage**: Admin SQLite managed sites table; runtime files under `runtime/admin_sites`; SurrealDB data folders; generated DbOption TOML files; generated output folders.

**Testing**: No cargo test or test-target compilation. Validate through running web_server/admin HTTP flows and CLI + JSON where backend-only validation is needed.

**Target Platform**: Windows local admin/deploy environment, with existing remote deployment paths kept out of scope unless they consume the same generated config.

**Project Type**: Rust web service + Vue admin frontend.

**Performance Goals**: Rename preview completes in under 1 second for normal site directory sizes because it should inspect paths and blockers, not copy data. Rename execution time is proportional to folder move cost.

**Constraints**: Do not kill unrelated processes; do not move live database directories; preserve old sites; avoid parsing E3D data in web_server beyond existing sidecar boundaries.

**Scale/Scope**: Managed admin deployment sites, one site rename at a time under the existing operation lock.

## Constitution Check

The repository-specific rule forbids cargo test/test-target validation and requires web_server validation via running service + HTTP/POST. This plan follows that constraint and keeps validation outside Rust test targets.

No additional constitution constraints are defined in `.specify/memory/constitution.md` beyond placeholders.

## Project Structure

### Documentation (this feature)

```text
specs/014-site-project-identity-config/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── site-project-identity-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── web_server/
│   ├── managed_project_sites.rs
│   ├── admin_handlers.rs
│   ├── models.rs
│   └── site_project_identity.rs        # optional helper module if logic grows
└── options.rs                          # only if generated DbOption identity fields need explicit normalization

ui/admin/src/
├── api/sites.ts
├── stores/sites.ts
├── types/site.ts
├── components/sites/
│   ├── SiteDrawer.vue
│   ├── SiteConfigSections.vue
│   └── parse-db-types.ts
└── views/
    ├── SiteDetailView.vue
    └── SitesView.vue

scripts/smoke/
└── site_project_identity_smoke.ps1     # optional HTTP smoke validation
```

**Structure Decision**: Keep feature implementation in the existing managed-site backend/UI modules. Add a helper module only if rename planning and filesystem migration exceed a readable size inside `managed_project_sites.rs`.

## Technical Approach

1. Introduce a normalized project identity helper:
   - Trim and validate `project_name`.
   - Produce filesystem-safe slug/name used for project-scoped runtime/data/output locations.
   - Enforce case-insensitive uniqueness on Windows-compatible semantics.

2. Add project-identity-aware paths while preserving old site access:
   - New sites use project-name-scoped runtime/data/output directories.
   - Existing site-id-scoped folders remain readable.
   - Site payload reports both current paths and identity/migration state if needed.

3. Add rename preview/apply flow:
   - Preview validates target name, checks duplicates, active task/process blockers, existing path conflicts, and lists affected paths.
   - Apply requires the same blockers to still be clear, performs moves/renames/config rewrites under the existing operation lock, updates SQLite record, and emits an actionable result.

4. Make parse settings visible and consistent:
   - Ensure create defaults keep `cata_partial_parse=true`.
   - Show `auto_parse_related_dbnums` and `cata_partial_parse` in create/edit drawer.
   - Include both fields in preview payload and update/create payloads.
   - Add read-only display in `SiteConfigSections.vue`.

5. Regenerate config after identity changes:
   - Generated DbOption files use the new project name and matching database name/folder values.
   - Start/parse/generate paths consume the rewritten config.

## Validation Plan

- HTTP create-site flow: create a site with default CATA partial parse and confirm API/detail payload contains `cata_partial_parse=true` and project-scoped data path.
- HTTP edit-site flow: toggle CATA partial parse and dependency parse, save, reload, and confirm detail + preview consistency.
- HTTP rename preview/apply flow: deploy/parse or create a stopped site, rename project name, verify affected path report and resulting API payload/config path values.
- HTTP blocker flow: try rename while site is running or parse status is running; verify rejection before filesystem changes.
- CLI/json check if needed: inspect generated DbOption JSON/TOML values and runtime directory names.
- Run `cargo fmt` only if Rust files are edited. Do not run cargo test.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Managed rename workflow instead of simple field update | Project name controls folders, config, and E3D database identity | A simple SQLite update would leave database files and generated config under the old identity |
| Compatibility for old site-id paths | Existing deployments may already store data under `runtime/admin_sites/<site_id>` | Forced migration on read/start could break deployed sites without operator intent |
