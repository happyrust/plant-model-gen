# Contract: Site Project Identity & Parse Configuration

## Existing Payload Fields To Preserve

Create, update, preview, get, and list site payloads must consistently carry:

```json
{
  "project_name": "MAIN_PROJECT",
  "parse_db_types": ["SYST", "DESI", "CATA"],
  "force_rebuild_system_db": false,
  "auto_parse_related_dbnums": true,
  "cata_partial_parse": true
}
```

## Create Site

### Request

`POST /api/admin/sites`

Required existing fields remain unchanged. Defaults:

- `cata_partial_parse`: defaults to `true` when omitted.
- `auto_parse_related_dbnums`: defaults to existing backend default unless explicitly set by UI.
- `parse_db_types`: normalized to known DB type names.

### Response Requirements

Response `data` must include:

- `site_id`: immutable site id.
- `project_name`: accepted project name.
- `runtime_dir`: effective project-scoped runtime directory for new sites.
- `db_data_path`: effective project-scoped database data path.
- `parse_db_types`, `force_rebuild_system_db`, `auto_parse_related_dbnums`, `cata_partial_parse`.

### Error Requirements

- Duplicate normalized project name: `409 Conflict` or existing conflict response shape with message identifying the conflicting project name.
- Invalid project name/path: `400 Bad Request` or existing managed error response shape.

## Update Site Parse Configuration

### Request

`PUT /api/admin/sites/{site_id}`

When `project_name` is unchanged, update remains a normal config edit. The request may include:

```json
{
  "parse_db_types": ["SYST", "DESI", "CATA"],
  "force_rebuild_system_db": false,
  "auto_parse_related_dbnums": true,
  "cata_partial_parse": false
}
```

### Response Requirements

The returned site reflects saved parse settings and generated config files are rewritten if needed.

## Project Rename Preview

### Request

`POST /api/admin/sites/{site_id}/project-rename/preview`

```json
{
  "project_name": "NEW_PROJECT_NAME"
}
```

### Response

```json
{
  "success": true,
  "message": "项目重命名预览完成",
  "data": {
    "site_id": "old-site-id-3100",
    "old_project_name": "OLD_PROJECT",
    "new_project_name": "NEW_PROJECT_NAME",
    "blocked": false,
    "blockers": [],
    "affected_paths": [
      {
        "kind": "db_data_path",
        "from": ".../OLD_PROJECT/data/surreal.db",
        "to": ".../NEW_PROJECT_NAME/data/surreal.db",
        "exists": true,
        "conflict": false,
        "action": "move"
      }
    ],
    "affected_names": [
      {
        "kind": "e3d_database_name",
        "from": "OLD_PROJECT",
        "to": "NEW_PROJECT_NAME"
      }
    ],
    "requires_regeneration": false,
    "warnings": []
  }
}
```

### Blocked Response

A blocked preview should still return a plan-like payload where possible:

```json
{
  "success": true,
  "message": "项目重命名存在阻塞项",
  "data": {
    "blocked": true,
    "blockers": ["站点正在运行，请先停止站点"],
    "affected_paths": []
  }
}
```

## Apply Project Rename

### Request

`POST /api/admin/sites/{site_id}/project-rename/apply`

```json
{
  "project_name": "NEW_PROJECT_NAME",
  "confirm": true
}
```

### Response

```json
{
  "success": true,
  "message": "项目名称已重命名",
  "data": {
    "site": {
      "site_id": "old-site-id-3100",
      "project_name": "NEW_PROJECT_NAME",
      "runtime_dir": ".../NEW_PROJECT_NAME",
      "db_data_path": ".../NEW_PROJECT_NAME/data/surreal.db",
      "cata_partial_parse": true
    },
    "applied_actions": [
      "moved db data directory",
      "rewrote DbOption.toml",
      "updated managed site record"
    ],
    "warnings": []
  }
}
```

### Error Requirements

- `confirm` missing/false: reject without changes.
- Site active/running/task active: reject without filesystem changes.
- Path conflict: reject without filesystem changes.
- Move/rewrite failure: return failure and include actionable message; do not report success.

## Site Detail Display Contract

`GET /api/admin/sites/{site_id}` response must be enough for UI details to display:

- Project name and effective paths.
- Parse DB type groups.
- System DB rebuild policy.
- Auto dependency parse state.
- CATA partial parse state.
- Whether CATA partial parse is effective or currently inactive because CATA/dependency parsing is not selected.
