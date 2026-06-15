# Quickstart: SurrealDB 站点数据目录隔离验证

## Prerequisites

- Work from repository root: `D:/work/plant-code/plant-model-gen-range-id`
- Do not run Rust tests or compile test targets.
- Use admin web server + HTTP requests for web_server validation.

## Static Check

```powershell
cargo check --bin web_server --features web_server
```

Expected:

- Command completes successfully.
- No test target is compiled.

## Scenario 1: Quick Deploy Defaults To Fast Single-DB Parse

Start the admin server, then submit a quick deploy request for AvevaPlantSample `250160` without `auto_parse_related_dbnums`.

Expected artifacts:

```text
runtime/admin_sites/avevaplantsample-8080/metadata.json
runtime/admin_sites/avevaplantsample-8080/DbOption.toml
runtime/admin_sites/avevaplantsample-8080/DbOption-parse.toml
runtime/admin_sites/avevaplantsample-8080/DbOption-generate.toml
```

Expected checks:

- `metadata.json` contains `db_data_path`.
- `db_data_path` includes `projects/avevaplantsample/data/surreal.db`.
- All three TOML configs use the same SurrealDB path.
- `parse-plan-manifest.json` does not show dependency DB files added solely by automatic related DB parsing.

## Scenario 2: Explicit Dependency Parsing Still Works

Repeat the same quick deploy request with:

```json
{
  "auto_parse_related_dbnums": true
}
```

Expected checks:

- `db_data_path` remains under the site-name directory.
- Related DB files may appear in `parse-plan-manifest.json` according to existing dependency rules.
- The explicit `true` request is distinguishable from omitted/default behavior.

## Scenario 3: Different Sites Do Not Share RocksDB

Deploy `AvevaPlantSample` and `AvevaMarineSample` using the same endpoint.

Expected checks:

```text
runtime/admin_sites/avevaplantsample-8080/projects/avevaplantsample/data/surreal.db
runtime/admin_sites/avevamarinesample-8080/projects/avevamarinesample/data/surreal.db
```

- Paths are different.
- Each path can be identified from the site name.
- Starting or stopping one site does not target the other site's SurrealDB process unless both have been explicitly configured to use the same path.

## Scenario 4: Existing Site Compatibility

Open an existing site whose stored `db_data_path` still points to:

```text
runtime/admin_sites/<site_id>/data/surreal.db
```

Expected checks:

- Existing site still uses its stored path.
- No automatic migration occurs.
- Recreating the site writes the new site-name-based path.
