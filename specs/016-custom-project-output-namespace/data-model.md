# Data Model: Custom Project Output Namespace

## Managed Project Site

Represents one admin-managed deployment. A deployment is a platform project namespace that may aggregate one or more E3D source projects.

**Key fields**:

- `site_id`: Immutable technical identity for routes, logs, process ownership, and task history.
- `project_name`: Admin-provided custom deployment identity. Used for runtime database namespace, active output namespace, and externally visible project access.
- `project_path`: Source project path or root used by legacy/single-project flows.
- `projects`: Optional multi-project source list. Each entry keeps original E3D name and path.
- `associated_project`: Optional source project name fallback.
- `parse_db_types`: Persisted parse DB type selection.
- `auto_parse_related_dbnums`: Dependency-driven related DB parsing flag.
- `cata_partial_parse`: CATA closure/manifest partial parse flag.
- `manual_db_nums`: Scoped target DB numbers for parse/generate.

**Validation rules**:

- `project_name` must be non-empty after trimming.
- Normalized `project_name` must be safe for local runtime/output path use.
- `site_id` must not change when `project_name` changes.
- `parse_db_types` must represent an explicit/default preset state and must not silently expand scoped selections to full system parsing.

## Custom Project Identity

The deployment/collection identity chosen by the admin.

**Derived from**: `ManagedProjectSite.project_name`.

**Used for**:

- Generated DbOption `project_name`.
- Runtime database namespace/name.
- Externally visible project name used by access URLs and clients.
- Active output subfolder: `output/<custom_project_name>`.
- Active scene tree root.
- Active parquet root.
- Active CATA closure manifest path.
- Viewer project parameter when resolving site-generated files.

**Not used for**:

- Source E3D DB file discovery.
- `included_projects`.
- `project_dirs`.

## Source E3D Project Identity

The original E3D project name and filesystem path used to discover source DB files. A managed project can reference multiple source identities, and none of them is required to match the custom project identity.

**Derived from**:

- Explicit `projects[]` entries when present.
- `associated_project` or canonical source project path fallback for older records.

**Used for**:

- `included_projects`.
- `project_dirs`.
- Parse preview source roots.
- DB file discovery and dependency indexing.

**Not used for**:

- Active output namespace.
- Active CATA manifest lookup.
- Runtime database namespace.
- External managed project access name.

## Output Namespace

The active generated-artifact folder for one deployment identity.

**Path shape**:

```text
runtime/admin_sites/<project_code>/<site_id>/output/<custom_project_name>/
├── scene_tree/
│   ├── cata_closure.json
│   └── db_index.sqlite
└── parquet/
```

**State rules**:

- The active namespace is always derived from custom project identity.
- Historical output under source E3D names may exist but is not authoritative after re-parse/redeploy.
- If both historical and active namespaces exist, readers use the active custom namespace.

## CATA Closure Manifest

Manifest describing CATA DB numbers and refnos required by the scoped target.

**Key fields**:

- `by_dbnum`: Map of CATA DB number to covered refnos.

**State rules**:

- If manifest loads successfully from the active output namespace, final parse config may be narrowed to manifest-covered CATA DBs.
- If manifest is missing/unreadable, final parse config is not narrowed and the attempted path is logged.
- Empty manifest is valid only when closure generation succeeded and manifest was readable.

## Parse Type Selection

Persisted operator intent for which DB types are included in parse.

**Allowed states**:

- Scoped/default preset.
- Full system preset.
- Custom explicit selection.
- Older empty record fallback requiring clear UI/API handling.

**Validation rules**:

- Full system parsing is persisted only when explicitly selected or when preserving a legacy site that already has all types.
- Empty values from quick deploy or UI forms must not be interpreted differently between preview, save, and detail display.

## Generation Precheck

Prerequisite verification and repair before model generation.

**Inputs**:

- Target `manual_db_nums` or inferred generation dbnums.
- Active output namespace for reading/writing generated prerequisites.
- Source E3D project identities for parsing source DB files.

**Used for**:

- Checking whether tree files exist for target dbnums.
- Loading or generating `db_meta_info.json`.
- Refreshing transform coverage before geometry generation.

**Identity rules**:

- Uses custom project identity only for output locations.
- Uses source E3D project identity for parsing/repair input.
- Must reject missing source-project mappings with a clear error.
- Must not panic if a project path cannot be resolved.
