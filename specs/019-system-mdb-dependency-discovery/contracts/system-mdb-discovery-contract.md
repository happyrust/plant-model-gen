# Contract: System-Library MDB Dependency Discovery

## Purpose

Define externally verifiable behavior for MDB-name quick deploy and MDB candidate discovery. The contract ensures dependency paths are known only after system-library parsing, not guessed from directory names.

## Quick Deploy Request Contract

An MDB-name quick deploy request MAY provide:

```json
{
  "mbd_name": "/ALL",
  "search_roots": ["D:\\AVEVA\\Projects\\E3D2.1"],
  "project_path": "",
  "projects": [],
  "dbnum": null,
  "db_file": null
}
```

At least one of `search_roots`, `project_path`, or `projects` MUST be present when `mbd_name` is present.

If `mbd_name` is absent, legacy `db_file` quick deploy behavior MUST remain unchanged.

## MDB Name Normalization Contract

- Missing leading slash MUST be normalized for comparison.
- Case differences MUST NOT prevent matching.
- Empty or unset MDB names MUST NOT match.

Examples:

- `ALL` matches `/ALL`.
- `/all` matches `/ALL`.
- Empty string does not match any MDB.

## Discovery Contract

Before quick deploy treats dependencies as known, the system MUST:

1. Build candidate project roots from explicit `projects[]` or by scanning `search_roots` / `project_path`.
2. Build a DB file inventory by reading DB file headers under the candidate roots.
3. Fully parse supported system-library files: `SYST`, `GLOB`, and `GLB`.
4. Enumerate MDB candidates and member DBs from parsed system-library content.
5. Locate each member DB against the inventory and classify it as `available`, `missing`, or `ambiguous`.

Directory scan results alone MUST NOT be considered sufficient dependency proof.

## MDB Candidate Response Contract

Each MDB candidate MUST include:

```json
{
  "mdb_name": "/ALL",
  "project": "AvevaPlantSample",
  "source_file": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample\\aps250000_0001",
  "source_db_type": "SYST",
  "syst_file": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaPlantSample\\aps250000_0001",
  "dbnums": [250160, 250193],
  "db_files": [],
  "available_count": 2,
  "missing_count": 0,
  "ambiguous_count": 0,
  "type_counts": {
    "DESI": 1,
    "CATA": 1
  },
  "ready_to_deploy": true
}
```

`syst_file` is a compatibility alias and MAY point to a non-SYST source when the actual `source_db_type` is `GLOB` or `GLB`. Consumers SHOULD prefer `source_file` and `source_db_type`.

## Member DB Status Contract

Each member DB row MUST include:

```json
{
  "dbnum": 250193,
  "db_type": "CATA",
  "db_name": "/ALL/CATA",
  "file_name": "aps250193_0001",
  "file_path": "D:\\AVEVA\\Projects\\E3D2.1\\AvevaCatalogue\\aps250193_0001",
  "source_project": "AvevaCatalogue",
  "status": "available",
  "candidates": []
}
```

Status semantics:

- `available`: exactly one candidate file exists.
- `missing`: no candidate file exists.
- `ambiguous`: multiple candidate files exist; `candidates` MUST list them.

## Quick Deploy Resolution Contract

For a matching and deployable MDB, quick deploy MUST resolve:

- `projects`: complete dependency project collection.
- `project_path`: primary project path.
- `dbnum`: selected target DB number.
- `db_file`: selected target DB file.
- `project_name`: provided deployment name or a stable default.
- `warnings`: discovery warnings plus a summary of the resolved MDB, target DB, and dependency project count.

Target DB selection priority:

1. Explicit `dbnum`.
2. Explicit `db_file`.
3. Exactly one deployable design member.

If more than one target candidate remains, quick deploy MUST fail and ask the caller to narrow the request.

## Failure Contract

Quick deploy MUST fail before creating or generating a site when:

- No candidate projects are discovered.
- No supported system-library files are found.
- Requested `mbd_name` is not found.
- Matching MDB dependencies are missing.
- Matching MDB dependencies are ambiguous.
- Multiple target DB candidates match and no explicit `dbnum`/`db_file` resolves them.

Failure responses MUST include actionable warnings or details suitable for an operator to adjust search roots or project selection.

## Site Drawer Contract

The site configuration drawer path helper MAY compose:

```text
project_path = root + "\\" + project_or_mdb_folder_name
```

It MUST NOT claim dependencies are verified. Dependency verification is performed only by scan/MDB-candidates/quick-deploy backend flows.
