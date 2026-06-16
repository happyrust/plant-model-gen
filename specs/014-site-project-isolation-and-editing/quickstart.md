# Quickstart: Site Project Isolation And Post-Deploy Editing

## Prerequisites

- Run the admin web server normally for this repository.
- Use HTTP/POST/PUT validation against the running service. Do not create or run `cargo test`.
- Have an admin token/session available if auth is enabled.

## 1. Create Site With Explicit Project Name

Example payload:

```json
{
  "project_name": "Spec014ProjectA",
  "project_path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
  "project_code": 7011,
  "manual_db_nums": [250160],
  "parse_db_types": [],
  "auto_parse_related_dbnums": true,
  "cata_partial_parse": true,
  "gen_model": true,
  "gen_mesh": true,
  "gen_spatial_tree": true,
  "pipeline_db_mode": "ws",
  "runtime_db_mode": "ws",
  "db_user": "spec014_admin",
  "db_password": "Spec014StrongPass@2026"
}
```

Expected:

- Response includes `project_name = Spec014ProjectA`.
- Response includes `db_data_path` derived from `Spec014ProjectA`.
- Generated config files use the same `db_data_path`.

## 2. Verify Partial Parse Options In UI

Open:

- Site create drawer.
- Site edit drawer.
- Site detail configuration section.

Expected:

- "自动解析依赖库" is visible.
- "按需解析 CATA（部分解析）" is visible.
- Site detail displays whether CATA partial parse is active.
- Preview payload includes `auto_parse_related_dbnums` and `cata_partial_parse`.

## 3. Rename Project Name On Stopped Site

Use existing update endpoint:

```json
{
  "project_name": "Spec014ProjectB"
}
```

Expected:

- Request succeeds only when site is stopped and not parsing/generating.
- Response shows `project_name = Spec014ProjectB`.
- `db_data_path` changes to the project-name-derived path for `Spec014ProjectB`.
- Config files and metadata mention `Spec014ProjectB`.
- Old DB data folder is no longer used.
- Log records old/new project names and paths.

## 4. Reject Rename While Running

Start the site, then submit:

```json
{
  "project_name": "Spec014ProjectC"
}
```

Expected:

- Request fails before directory moves.
- Error tells the user to stop the site first.
- Subsequent `GET /api/admin/sites/{id}` still shows the old project name and old `db_data_path`.

## 5. Reject Directory Conflict

Create or keep a non-empty target data directory for the desired new project identity, then submit rename.

Expected:

- Request fails with a clear data directory conflict.
- No registry/config/folder mixed state.

## 6. Backward Compatibility Check

Update a non-name field:

```json
{
  "cata_partial_parse": false
}
```

Expected:

- Site updates successfully.
- `db_data_path` does not change.
- Detail page reflects CATA partial parse disabled.

## Validation Record

Fill this after implementation:

```text
Backend HTTP create:
Backend HTTP rename stopped:
Backend HTTP rename running rejection:
Backend config path inspection:
Frontend create/edit/detail inspection:
Notes:
```
