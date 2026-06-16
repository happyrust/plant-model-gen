# Data Model: Site Project Isolation And Post-Deploy Editing

## ManagedProjectSite

Existing entity keyed by immutable `site_id`.

### Existing Fields Used By This Feature

- `site_id`: Stable admin registry key. Must not change during project rename.
- `site_name`: Display/site label. Independent from `project_name`.
- `project_name`: Mutable E3D-facing project identity.
- `project_path`: Source E3D project root. v1 does not rename this directory.
- `projects`: Multi-project composition. v1 does not rename source project entries automatically.
- `auto_parse_related_dbnums`: Enables automatic dependency DB inclusion.
- `cata_partial_parse`: Enables CATA closure manifest partial parsing when automatic related DB parsing is active.
- `config_path`: Site config path under site runtime.
- `runtime_dir`: Site runtime directory under `runtime/admin_sites/<site_id>`.
- `db_data_path`: Effective SurrealDB data path. New sites and renamed sites should use project-name-derived path.
- `status`, `parse_status`, `db_pid`, `web_pid`, `viewer_pid`, `parse_pid`: Used to block unsafe rename while active.

### Derived Fields

#### Project Data Identity

Filesystem-safe slug derived from `project_name`.

Recommended normalization:

1. Trim whitespace.
2. Preserve ASCII letters/digits, `-`, `_`, and `.`.
3. Replace other characters with `_` or a short hash-suffixed safe form.
4. Collapse repeated separators.
5. Add a stable short hash of the original project name if needed to avoid collisions.
6. Treat identity comparison as case-insensitive on Windows.

Example:

```text
project_name = "AvevaPlantSample"
project_data_identity = "AvevaPlantSample"
db_data_path = runtime/admin_site_data/projects/AvevaPlantSample/surreal.db
```

The exact root can be chosen during implementation, but it must be outside source E3D directories and visible through `db_data_path`.

## Project Rename Migration

Transient operation, not necessarily persisted as a separate table in v1.

### Inputs

- `site_id`
- `old_project_name`
- `new_project_name`
- `old_db_data_path`
- `new_db_data_path`
- `old_output_project_dir`
- `new_output_project_dir`

### Preconditions

- Site exists.
- New project name is non-empty and valid.
- No other site owns the new project name or new DB data path.
- Site has no active DB/web/viewer/parse process.
- Target directories either do not exist or are empty/owned by the same site.

### Effects

- Move managed DB data directory from old project identity to new project identity when old path exists.
- Move or rewrite generated output folder(s) keyed by old project name.
- Update registry row fields: `project_name`, `db_data_path`, `updated_at`.
- Rewrite config files and metadata.
- Append audit log entry.

### Failure Behavior

- Validation failures happen before filesystem moves.
- If migration cannot complete safely, return error and keep the old registry/config/path values.
- If a filesystem move succeeds but a later step fails, implementation must either rollback the move or mark the site failed with a clear recovery log. The preferred v1 target is full rollback.

## Partial Parse Configuration

Logical configuration shown in UI.

### Effective State

```text
auto_parse_related_dbnums=false, cata_partial_parse=true
=> CATA partial parse configured but inactive.

auto_parse_related_dbnums=true, cata_partial_parse=true
=> CATA closure manifest partial parsing active.

auto_parse_related_dbnums=true, cata_partial_parse=false
=> related CATA DBs are parsed fully.
```

### UI Display Fields

- Automatic related DB parsing: enabled/disabled.
- CATA partial parse: enabled/disabled.
- Effective CATA behavior: active/inactive/full related CATA parse.

## API Shape

No required new top-level endpoint for v1. Existing `PUT /api/admin/sites/{id}` can carry `project_name`.

Optional response extension if implementation needs explicit migration reporting:

```json
{
  "site": {
    "site_id": "quicktest-250160-8080",
    "project_name": "NewProject",
    "db_data_path": "..."
  },
  "migration": {
    "project_name_changed": true,
    "old_project_name": "OldProject",
    "new_project_name": "NewProject",
    "old_db_data_path": "...",
    "new_db_data_path": "...",
    "moved_paths": ["..."]
  }
}
```

If keeping the response unchanged is preferred, the migration summary must still be visible in logs and refreshed site response.
