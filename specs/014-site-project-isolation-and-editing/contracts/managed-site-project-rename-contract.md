# Contract: Managed Site Project Rename

## Endpoint

```http
PUT /api/admin/sites/{site_id}
Content-Type: application/json
Authorization: Bearer <admin-token>
```

Uses existing managed-site update endpoint.

## Request

Only changed fields need to be sent. Project rename is triggered when `project_name` is present and differs from the stored value after trimming.

```json
{
  "project_name": "NewProjectName",
  "auto_parse_related_dbnums": true,
  "cata_partial_parse": true
}
```

## Success Response

Existing admin response envelope:

```json
{
  "success": true,
  "message": "更新站点成功",
  "data": {
    "site_id": "quicktest-250160-8080",
    "project_name": "NewProjectName",
    "db_data_path": "D:/.../projects/NewProjectName/surreal.db",
    "auto_parse_related_dbnums": true,
    "cata_partial_parse": true,
    "config_path": "D:/.../runtime/admin_sites/quicktest-250160-8080/DbOption.toml",
    "runtime_dir": "D:/.../runtime/admin_sites/quicktest-250160-8080"
  }
}
```

If implementation adds migration details, include them under `data.migration` or a similar backward-compatible optional field.

## Failure Responses

### Active Site

```json
{
  "success": false,
  "message": "站点正在运行或存在活动进程，请先停止站点再修改项目名"
}
```

### Project Name Conflict

```json
{
  "success": false,
  "message": "项目名已存在：NewProjectName。请修改项目名称后再保存。"
}
```

### Data Directory Conflict

```json
{
  "success": false,
  "message": "项目数据目录已存在且不属于当前站点，不能覆盖：D:/.../projects/NewProjectName"
}
```

### Migration Failure

```json
{
  "success": false,
  "message": "项目名迁移失败：<reason>。站点仍保留旧项目名。"
}
```

## Required Side Effects

When `project_name` changes successfully:

- Registry row stores new `project_name`.
- Registry row stores new `db_data_path`.
- `DbOption.toml`, `DbOption-parse.toml`, `DbOption-generate.toml`, and `metadata.json` are rewritten.
- Managed local DB directory is moved or re-pointed according to the project data identity rules.
- Generated output project folder references are updated where present.
- Site log records old/new names and old/new paths.

## Non-Effects

- `site_id` does not change.
- API URL path does not change.
- Original source E3D `project_path` and source DB files are not renamed in v1.
- Remote deployed DB data is not renamed in v1.
