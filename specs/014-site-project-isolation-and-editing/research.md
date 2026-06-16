# Research: Site Project Isolation And Post-Deploy Editing

## Current Code Findings

### Managed DB Path Is Currently Site-ID Based

`src/web_server/managed_project_sites.rs` currently derives DB data path as:

```rust
fn db_data_path(site_id: &str) -> PathBuf {
    site_runtime_dir(site_id).join("data").join("surreal.db")
}
```

This means storage follows `runtime/admin_sites/<site_id>/data/surreal.db`, not the E3D `project_name`. Config generation then writes that same value to:

- `[web_server].surreal_data_path`
- `[surrealdb].path`
- `surrealkv.path`
- API response `ManagedProjectSite.db_data_path`

### Project Name Is Already Unique On Create

`create_site()` checks `project_name_conflict_with_conn(conn, req.project_name.trim(), None)` and rejects duplicate project names. This can be reused as the first ownership guard for project-name-derived storage.

### Project Name Is Already In Update Contract

`UpdateManagedSiteRequest` already contains optional `project_name`, and the frontend type also exposes it. The missing behavior is the migration contract and path rewrite when a deployed site changes `project_name`.

### Partial Parse Exists In Data Contract

The backend and frontend types already include:

- `auto_parse_related_dbnums`
- `cata_partial_parse`

`CreateManagedSiteRequest`, `PreviewManagedSiteParsePlanRequest`, and quick deploy default CATA partial parsing to true. `SiteDrawer.vue` already has create/edit controls and preview payload fields. `SiteConfigSections.vue` currently displays parse DB types and system-library policy but does not explicitly display CATA partial parse status.

## Decisions

### Decision 1: Keep `site_id` Stable, Derive Data Path From `project_name`

**Decision**: `site_id` remains the immutable key for registry rows, logs, task IDs, and process snapshots. A new helper derives the project data folder from normalized `project_name`.

**Rationale**: Changing `site_id` would break API URLs, task history, logs, SSE snapshots, and remote task references. The user need is storage/config identity, not registry identity.

**Rejected Alternative**: Rename `site_id` on project rename. This would cascade through many URLs and registry references and is too risky for v1.

### Decision 2: Reject Active-Site Rename

**Decision**: Project rename is only allowed when DB/web/viewer/parse processes are inactive and site status is not `Starting`, `Running`, or `Stopping`.

**Rationale**: SurrealDB/RocksDB and web_server may hold file locks. Windows directory rename under active process is unreliable and risks partial state.

**Rejected Alternative**: Automatically stop the site before rename. This hides a disruptive action inside an edit save and complicates rollback.

### Decision 3: Migrate Managed Local Artifacts Only

**Decision**: v1 migrates managed local DB data and generated output/config references. It does not rename original E3D project source directories.

**Rationale**: Managed runtime artifacts are owned by this admin system. Source E3D folders may be shared by other tools and require explicit operator confirmation.

### Decision 4: Detail Page Must Show Effective Partial Parse State

**Decision**: Add explicit rows/badges in `SiteConfigSections.vue` for automatic related DB parsing and CATA partial parsing.

**Rationale**: Users need to explain parse behavior after deployment. Showing only parse DB type chips does not reveal closure manifest partial-parse behavior.

## Open Questions

1. Should a future version offer an explicit destructive action to rename source E3D folders/files under `project_path`?
2. Should remote deployed DB paths be migrated automatically, or should project rename require explicit remote redeploy?
3. Should legacy sites using `site_id/data/surreal.db` be migrated lazily on first project-name edit only, or proactively on next start?

## Risks

- Windows case-only rename can be tricky. Use a temporary intermediate path when normalized project identity changes only by case.
- Atomic rollback of directory moves is hard if the final config rewrite fails. Prefer validating all targets before moving and writing a migration log.
- Existing data path may already contain project-name-independent data. Migration must not overwrite non-empty target directories.
- UI defaults can drift if presets omit `cata_partial_parse`; presets should set it explicitly for clarity.
