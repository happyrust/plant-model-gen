# Research: Site Project Identity & Parse Configuration

## Existing Code Findings

- `ManagedProjectSite` already stores `project_name`, `parse_db_types`, `force_rebuild_system_db`, `auto_parse_related_dbnums`, and `cata_partial_parse` in `src/web_server/models.rs`.
- `cata_partial_parse` is documented as default true in the backend model, but admin detail display currently shows parse DB types and system rebuild policy without showing dependency parse/CATA partial parse explicitly.
- `CreateManagedSiteRequest`, `UpdateManagedSiteRequest`, and `PreviewManagedSiteParsePlanRequest` already include partial parse fields in `ui/admin/src/types/site.ts`.
- Current runtime helper paths in `src/web_server/managed_project_sites.rs` are mostly `runtime/admin_sites/<site_id>/...`, while project-specific generated tree paths already include `output/<project_name>/scene_tree` in at least one helper.
- `project_name_conflict_with_conn` already provides case-insensitive uniqueness checking for project names, which should be reused/extended for rename conflicts.
- `preview_parse_plan` is proxied to the sidecar from `admin_handlers.rs` / `parse_sidecar_client.rs`, so frontend preview payloads must continue carrying parse flags to the sidecar.

## Decisions

### D1: Keep `site_id` immutable

**Decision**: Do not rename `site_id` or route identifiers when `project_name` changes.

**Reasoning**: Site id is already embedded in routes, task state, logs, process registry ownership, runtime lookup, and historical metrics. Renaming it would require a much broader migration and would harm history preservation.

**Alternatives Considered**:

- Rename `site_id` together with project name: rejected due to blast radius and broken URLs/history.
- Treat project name as display-only: rejected because user explicitly needs database/folder/E3D identity rename.

### D2: Rename only when stopped and idle

**Decision**: Require stopped/idle state for project identity rename.

**Reasoning**: Moving SurrealDB data and generated folders while processes hold files is risky on Windows and can create mixed identity state.

**Alternatives Considered**:

- Auto-stop then rename: rejected for first version because stop may fail or affect operator expectations.
- Allow running rename and defer moves: rejected because it hides split-state complexity.

### D3: Show CATA partial parse as a first-class config

**Decision**: Present `auto_parse_related_dbnums` and `cata_partial_parse` in create/edit/detail, and include their values in parse preview.

**Reasoning**: Operators need to understand why only part of CATA was parsed. The backend already models these fields, so UI visibility is the missing product behavior.

### D4: Project-name-scoped paths for new sites, compatibility for old sites

**Decision**: New sites should use project identity in managed folders; old site-id folders are supported until rename/migration.

**Reasoning**: This satisfies the new isolation model without breaking existing deployments.

## Open Follow-Up Questions

These do not block the first spec because reasonable defaults were chosen:

1. Should remote deployment rename be managed in the same workflow or a later remote-sync feature? Default: later/explicitly out of scope.
2. Should logs be physically moved into project-name folders or remain site-id scoped? Default: preserve logs by site id for history continuity.
3. Should generated output be moved or invalidated/regenerated when rename fails on derived artifacts? Default: move owned folders when safe; otherwise preserve identity rename and require regeneration only for derived output if explicitly documented in result.
