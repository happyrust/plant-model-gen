# Implementation Plan: SurrealDB 站点数据目录隔离

## Goal

让受管站点和 quick deploy 在创建配置时生成可读、可隔离、可验证的 SurrealDB 数据目录：

- 数据库文件夹与站点名称对应，便于从磁盘路径直接识别归属。
- quick deploy/API 默认不自动解析依赖 DB，以支持单 DB 快速测试。
- file pipeline 与 ws runtime 继续共享同一站点专属 `db_data_path`，不破坏 RocksDB 排他锁互斥。

## Current Flow

```text
create site / quick deploy
  -> infer site_id from site_name + web_port
  -> db_data_path = runtime/admin_sites/<site_id>/data/surreal.db
  -> write DbOption.toml / DbOption-parse.toml / DbOption-generate.toml
  -> parse/generate sidecars open that db_data_path
```

Observed problems:

- `data/surreal.db` is generic under the site root and does not visibly encode the site name.
- Reusing a site name/port in quick tests can reuse existing RocksDB state unless the site is fully recreated.
- The quick deploy request can still pull in related DB files by default in some call paths, slowing single-target smoke tests.

## Target Flow

```text
create site / quick deploy
  -> infer site_id from site_name + web_port
  -> derive db slug from site_name
  -> db_data_path = runtime/admin_sites/<site_id>/projects/<site_slug>/data/surreal.db
  -> write same db_data_path to all generated configs and metadata
  -> parse/generate/runtime all use the same path
```

Quick deploy dependency parsing:

```text
request.auto_parse_related_dbnums omitted
  -> false
  -> parse only target DB plus mandatory preparse requirements

request.auto_parse_related_dbnums = true
  -> preserve existing related DB discovery behavior
```

## Design

### 1. Database path derivation

Use the existing site-name slug rule as the only naming source for the database directory. The path should be deterministic:

```text
runtime/admin_sites/<site_id>/projects/<site_slug>/data/surreal.db
```

Where:

- `<site_id>` remains the root-level site identifier and includes port uniqueness.
- `<site_slug>` is derived from `site_name`, not from `project_name`.
- `project_name` remains the output project grouping key.

### 2. Site creation and quick deploy config generation

Update all new-site construction paths so `ManagedProjectSite.db_data_path` is set once from the site name and then reused everywhere:

- normal managed-site creation
- quick deploy preview/new-site path
- quick deploy execution/new-site path

Avoid recomputing separate SurrealDB paths while writing parse/generate/runtime configs.

### 3. Runtime directory creation

Directory creation must follow `site.db_data_path` rather than assuming `runtime/admin_sites/<site_id>/data`.

Required behavior:

- create `logs/` under the site root;
- create the parent directory of `site.db_data_path`;
- preserve existing directories when rerunning;
- do not migrate or delete old root `data/surreal.db`.

### 4. Config and metadata consistency

All generated artifacts must reference the same path:

- `DbOption.toml`
- `DbOption-parse.toml`
- `DbOption-generate.toml`
- `metadata.json`
- SurrealDB ws launch command
- parse/generate sidecar command

`metadata.json` should expose `db_data_path` so operators can validate the path without inspecting TOML.

### 5. Quick deploy default dependency behavior

Change request normalization so omitted `auto_parse_related_dbnums` means `false` for quick deploy/API fast testing.

Preserve explicit behavior:

- `auto_parse_related_dbnums=false`: do not auto-expand related DB files.
- `auto_parse_related_dbnums=true`: run existing related DB discovery and CATA/DICT inclusion logic.

Do not change normal managed-site forms unless they intentionally reuse the same quick deploy request defaults.

### 6. Validation

Static validation:

```powershell
cargo check --bin web_server --features web_server
```

HTTP validation:

1. Start admin web server.
2. Call quick deploy for `AvevaPlantSample` with `dbnum=250160`, omitting `auto_parse_related_dbnums`.
3. Confirm `metadata.json` contains a `db_data_path` under `projects/avevaplantsample/data/surreal.db`.
4. Confirm generated TOML files use the same path.
5. Confirm parse plan does not auto-add related DB files unless the request explicitly sets `auto_parse_related_dbnums=true`.

## Risks

### R1: Existing sites still point to the old path

This is intentional. Existing sites should not be silently migrated. Operators can recreate or explicitly migrate if needed.

### R2: Site name and project name differ

Use site name for DB folder, project name for output folder. This matches the user-facing deployment identity and preserves current output grouping.

### R3: Omitted dependency parsing may surprise callers expecting full closure

Document the quick deploy default clearly and require explicit `auto_parse_related_dbnums=true` for full dependency behavior.

### R4: Path rule must not break file/ws exclusivity

All exclusivity checks already key off `db_data_path`; keep that invariant and avoid secondary path derivation.

## Rollout

1. Implement site-name-based `db_data_path` derivation for new site creation.
2. Update runtime directory creation and metadata output.
3. Update quick deploy default dependency parsing.
4. Validate with `cargo check --bin web_server --features web_server`.
5. Run HTTP quick deploy smoke tests for omitted and explicit `auto_parse_related_dbnums`.
