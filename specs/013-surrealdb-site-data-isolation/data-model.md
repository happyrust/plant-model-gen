# Data Model: SurrealDB 站点数据目录隔离

## Managed Site

Represents one deployable site managed by the admin server.

Fields relevant to this feature:

- `site_id`: Stable site root identifier, typically derived from site name and web port.
- `site_name`: User-facing deployment name. Source for the database directory slug.
- `project_name`: Model/output project identity. Continues to drive `output/<project_name>`.
- `runtime_dir`: Site root under `runtime/admin_sites/<site_id>`.
- `db_data_path`: Canonical SurrealDB RocksDB directory for this site.
- `pipeline_db_mode`: Parse/generate mode, `file` or `ws`.
- `runtime_db_mode`: Runtime mode, `file` or `ws`.

Validation rules:

- `site_name` must normalize to a non-empty slug.
- `db_data_path` must be under the site runtime directory for new sites.
- `db_data_path` must include the normalized site-name slug for new sites.

## Database Data Path

Represents the directory opened by SurrealDB/RocksDB.

Expected new-site shape:

```text
runtime/admin_sites/<site_id>/projects/<site_slug>/data/surreal.db
```

Validation rules:

- Config files and metadata must use the same value.
- The parent directory must be created before parse/generate/runtime starts.
- Existing old paths remain valid for existing sites.

## Quick Deploy Request

Represents the HTTP request that creates a temporary or managed site for fast validation.

Fields relevant to this feature:

- `site_name`: Optional user-facing site name; defaults from project name when omitted.
- `project_name`: Project identity.
- `dbnum` or `db_file`: Target DB selector.
- `auto_parse_related_dbnums`: Optional dependency parsing switch.
- `force_recreate`: Whether to recreate an existing same-name site.

Validation rules:

- Omitted `auto_parse_related_dbnums` means `false`.
- Explicit `true` enables related DB discovery.
- Explicit `false` keeps the parse target narrow.

## Runtime Metadata

Represents operator-visible facts written to the site runtime directory.

Fields relevant to this feature:

- `site_id`
- `project_name`
- `pipeline_db_mode`
- `runtime_db_mode`
- `db_data_path`
- `output_root`

Validation rules:

- `db_data_path` must match generated TOML config values.
- `output_root` remains independent from `db_data_path`.

## Data Directory Owner

Represents the process or mode currently holding a RocksDB data directory.

Fields relevant to this feature:

- `db_data_path`
- `site_id`
- `pid`
- `mode`
- `role`

Validation rules:

- Exclusivity checks must continue to key on `db_data_path`.
- Different sites with different `db_data_path` values must not block each other.
