# Contract: Quick Deploy Request Defaults

## Endpoint

`POST /api/admin/quick-deploy-test`

## Relevant Request Fields

```json
{
  "site_name": "AvevaPlantSample",
  "project_name": "AvevaPlantSample",
  "project_path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
  "dbnum": 250160,
  "auto_parse_related_dbnums": false,
  "pipeline_db_mode": "file",
  "runtime_db_mode": "file",
  "force_recreate": true
}
```

## Defaults

| Field | Omitted Default | Notes |
|-------|-----------------|-------|
| `auto_parse_related_dbnums` | `false` | Fast test mode parses only the requested target and mandatory preparse requirements. |
| `site_name` | Derived from `project_name` | The normalized site name drives the SurrealDB database folder. |
| `runtime_db_mode` | Existing endpoint default | Must still use the same `db_data_path` as parse/generate config. |

## Required Response/Artifact Effects

- The created site metadata includes `db_data_path`.
- `db_data_path` contains the normalized site name.
- `DbOption.toml`, `DbOption-parse.toml`, and `DbOption-generate.toml` all reference the same `db_data_path`.
- When `auto_parse_related_dbnums` is omitted, parse plan output must not auto-expand dependency DB files.
- When `auto_parse_related_dbnums=true`, parse plan output may include dependency DB files according to existing rules.

## Example Expected Metadata Fragment

```json
{
  "site_id": "avevaplantsample-8080",
  "project_name": "AvevaPlantSample",
  "db_data_path": "runtime/admin_sites/avevaplantsample-8080/projects/avevaplantsample/data/surreal.db",
  "pipeline_db_mode": "file",
  "runtime_db_mode": "file"
}
```

## Error Conditions

- If the site name cannot be normalized into a usable slug, the request fails with a clear validation error.
- If two newly created sites would resolve to the same `db_data_path`, the request fails or applies the existing unique site-id strategy before writing files.
- If the path parent cannot be created, the request fails before starting parse/generate sidecars.
