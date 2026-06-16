# Implementation Plan: Deployment Project Identity Over E3D Collection

**Branch**: `[018-deployment-project-identity]` | **Date**: 2026-06-16 | **Spec**: `specs/018-deployment-project-identity/spec.md`

**Input**: Feature specification from `/specs/018-deployment-project-identity/spec.md`

## Summary

Make the deployment project name the single, authoritative outward identity for a deployment that is a collection of one or more E3D source projects, with the E3D names used only for source discovery. Most behavior already exists (014 uniqueness/rename, 016 output namespace). This feature adds: the explicit independence invariant, a coincidence warning, a static regression guard that prevents outward-identity code from using an E3D source name, and consistent presentation across configuration surfaces.

## Technical Context

**Language/Version**: Rust backend; Vue 3 + TypeScript admin UI.

**Primary Dependencies**: existing managed-site backend (`src/web_server/managed_project_sites.rs`), admin UI site components, PowerShell guard scripts under `scripts/guard/`.

**Storage**: No schema change; reuses `site.projects`, `project_name`, generated DbOption files.

**Testing**: No `cargo test` / Rust test-target for `web_server`. Validate via running service HTTP/POST, generated config inspection, the static guard, and admin UI checks.

**Target Platform**: Windows primary; Unix compatible.

**Project Type**: Rust web service + Vue admin frontend.

**Performance Goals**: Identity resolution is O(1); the guard is a static scan run on demand/CI.

**Constraints**: Reuse the centralized deployment-name helper from 016; do not migrate legacy artifacts; never use E3D source names for outward identity.

**Scale/Scope**: Managed admin deployment sites, single- and multi-E3D collections.

## Constitution Check

Repository rule forbids `cargo test`/test-target validation for `web_server` and requires running-service validation. This plan follows that. `.specify/memory/constitution.md` has placeholders only.

## Project Structure

### Documentation (this feature)

```text
specs/018-deployment-project-identity/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── deployment-identity-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/
└── web_server/
    ├── managed_project_sites.rs     # centralize outward identity on site_deployment_project_name; coincidence warning
    └── models.rs                    # only if a warning field needs to surface in responses

ui/admin/src/
└── components/sites/
    ├── SiteDrawer.vue               # coincidence warning + uniqueness messaging
    └── SiteConfigSections.vue       # show deployment name vs E3D collection clearly

scripts/guard/
└── deployment_identity_guard.ps1    # static guard: outward-identity consumers must not use E3D source name
```

**Structure Decision**: No new module. Reinforce the existing 016 helper boundary and add a guard script mirroring `scripts/guard/web_server_parse_boundary_guard.ps1`.

## Technical Approach

### 1. Centralize outward identity (verify + reinforce)

- Confirm every outward-identity surface resolves through `site_deployment_project_name`:
  - database name (`site_runtime_database_name`), runtime dir (`site_runtime_dir_for_project(site.project_name, ...)`), output dir (`site_output_root` + deployment-named subfolder), `site_project_tree_dir`, parquet root, viewer `output_project` (`build_viewer_url`).
- Confirm source-only helpers (`site_source_project_name`, `site_parse_project_names`, `existing_project_roots`, `site_included_projects_and_dirs`) feed only `included_projects`/`project_dirs` and source discovery.

### 2. Coincidence warning

- During create/edit/clone/quick-deploy and preview, if the normalized deployment name equals any E3D source project name in the collection, attach a non-blocking warning to the response (and surface it in the admin UI). The operation still succeeds.

### 3. Uniqueness (reuse 014)

- Keep deployment-name uniqueness enforcement (014 FR-003). Ensure the same normalization is applied across all surfaces.

### 4. Static regression guard

- Add `scripts/guard/deployment_identity_guard.ps1` that scans `src/web_server/managed_project_sites.rs` (and related) to assert that outward-identity functions do not call `site_source_project_name`. Encode an allowlist of source-only call sites; fail if `site_source_project_name` appears inside an outward-identity function body.
- Mirror the structure/exit-code conventions of `web_server_parse_boundary_guard.ps1`.

### 5. UI presentation

- In `SiteConfigSections.vue`, present the deployment name as the outward identity and list the E3D source projects as the collection.
- In `SiteDrawer.vue`, show the coincidence warning and keep uniqueness validation messaging.

## Validation Plan

- Generate config for a multi-E3D deployment named `9002` (source `AvevaPlantSample` + `AvevaCatalogue`); confirm outward surfaces use `9002` and `included_projects` carry E3D names.
- Set deployment name equal to an E3D name; confirm success + warning + no functional conflict.
- Run `scripts/guard/deployment_identity_guard.ps1`; confirm pass on current code and fail when an outward path is switched to the source name.
- Verify uniqueness rejection across create/edit/clone/quick-deploy.
- Verify details/preview presentation consistency.
- `cargo fmt`; `cargo check --features web_server` if Rust changes; admin UI type-check if UI changes.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Dedicated static guard script | The independence invariant is easy to regress silently | Relying on review/manual checks already failed once (CATA manifest path used source name) |
