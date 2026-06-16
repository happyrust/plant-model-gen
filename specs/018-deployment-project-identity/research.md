# Research: Deployment Project Identity Over E3D Collection

## Context from prior specs

- `014-site-project-identity-config` established: `project_name` is the editable business/database identity; `site_id` immutable; deployment-name uniqueness across sites (FR-003); rename workflow; parse-config visibility.
- `016-custom-project-output-namespace` established: outward output namespace (scene_tree/parquet/CATA manifest), DbOption `project_name`, parquet root, and viewer `output_project` all use the deployment name; `included_projects`/`project_dirs` keep E3D source names; no migration of legacy artifacts.

This feature adds the missing explicit pieces: the collection-first model, the independence invariant, a coincidence warning, and a regression guard.

## Decision: deployment name is the sole outward identity

**Decision**: The deployment project name determines database name, runtime/output directory names, and external (viewer) access name.

**Rationale**: A deployment aggregates multiple E3D projects; addressing it by any single member's name would be ambiguous and brittle. A single deployment-owned identity is stable regardless of collection membership.

**Alternatives considered**:
- Derive outward identity from the primary E3D project name. Rejected: ties identity to one member and breaks when membership/primary changes.

## Decision: E3D names are source-only and independent

**Decision**: E3D source project names/paths are used only for `included_projects`/`project_dirs` and source discovery, never for outward identity. The two namespaces are independent; equal values are allowed.

**Rationale**: The deployment name may legitimately differ from, or coincide with, an E3D name. Treating them as one namespace caused the diagnosed CATA manifest path bug (outward path resolved via a source name).

**Alternatives considered**:
- Forbid deployment name equal to any E3D name. Rejected: unnecessary restriction; they are different fields. A warning is sufficient.

## Decision: coincidence warning, not rejection

**Decision**: When the deployment name equals an E3D source name, allow the operation and warn.

**Rationale**: No functional conflict exists because the fields are independent; a warning helps operators avoid confusion without blocking valid setups.

## Decision: static regression guard

**Decision**: Add a static guard asserting outward-identity consumers never call the E3D source-name helper.

**Rationale**: The invariant is easy to regress silently; a guard catches it deterministically. The repository already uses guard scripts (e.g., `web_server_parse_boundary_guard.ps1`).

**Alternatives considered**:
- Runtime assertion. Rejected: cannot enforce at the call-site granularity and adds runtime cost.
- Rely on code review. Rejected: it already failed once.

## Decision: reuse, do not duplicate

**Decision**: Reference 014 (uniqueness/rename) and 016 (output namespace) instead of restating their requirements; 018 adds only the model + invariant + guard + presentation.

**Rationale**: Keeps each spec focused and avoids conflicting duplicate requirements.
