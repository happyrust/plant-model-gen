# Data Model: Deployment Project Identity Over E3D Collection

## Deployment Project (Managed Site)

The outward-facing deployment.

**Attributes**:

- `site_id`: immutable technical identity.
- `project_name`: editable deployment identity; the SOLE outward identity.
- `projects`: collection of E3D source projects (see below).
- `project_code`: numeric code used for SurrealDB namespace.

**Outward identity derivations (all from `project_name`)**:

- database name (`site_runtime_database_name` -> `site_deployment_project_name`).
- runtime directory (`site_runtime_dir_for_project(project_name, site_id)`).
- output directory and subfolders (`site_output_root` + deployment-named `scene_tree`/`parquet`/`cata_closure.json`).
- external/viewer access name (`build_viewer_url` `output_project`).

**Rules**:

- Outward identity MUST resolve only from `project_name`.
- `project_name` MUST be unique across managed sites (normalized), per 014.
- `project_name` MAY equal an E3D source project name; if so, emit a warning.

## E3D Source Project

A member of the deployment collection.

**Attributes**:

- `name`: original E3D project name.
- `path`: real E3D project directory.
- `role`: `design` or `library`.
- `is_primary`: marks the primary design project.
- `sort_order`: ordering within the collection.

**Used for (source-only)**:

- `included_projects` (names) and `project_dirs` (paths).
- source DB discovery and parse roots.

**Never used for**:

- database name, runtime/output directories, or external access name.

## Project Collection

The `site.projects` set.

**Rules**:

- A deployment has one or more E3D source projects.
- Primary selection: `is_primary`, else first `design`, else first member.
- The deployment name is independent of the primary member's name.

## Independence Invariant

A cross-cutting rule rather than a stored entity.

**Statement**: Outward-identity surfaces derive from `project_name`; source-discovery surfaces derive from the E3D collection. No outward-identity surface reads an E3D source name.

**Enforcement**: Static guard `scripts/guard/deployment_identity_guard.ps1`.

## Coincidence Warning

A non-blocking advisory.

**Trigger**: normalized `project_name` equals any E3D source project name in the collection.

**Effect**: operation succeeds; warning surfaced in API response and admin UI.
