# Data Model: System-Library MDB Dependency Discovery

## SearchRoot

Represents an operator-provided directory used to discover candidate E3D source projects.

Fields:

- `path`: canonical filesystem path.
- `source`: whether it came from `search_roots`, `project_path`, or an explicit project collection.

Validation:

- Empty paths are ignored.
- At least one search root, project path, or explicit project must be present for MDB-name quick deploy.
- Nonexistent roots produce warnings or failure depending on whether any valid roots remain.

## E3DSourceProject

Represents one candidate or explicit E3D source project.

Fields:

- `name`: source project display name, normally derived from the folder or explicit project data.
- `path`: canonical source project path.
- `role`: design or library.
- `is_primary`: whether this project is the primary project for quick deploy defaults.
- `sort_order`: stable ordering for project collection persistence.
- `dbnums`: DB numbers discovered under the project path.
- `db_types`: DB types discovered from file headers.

Relationships:

- One quick-deploy request can resolve to many `E3DSourceProject` entries.
- MDB member DB statuses point back to their source project when available.

## DbFileInventoryEntry

Represents one DB file discovered by scanning candidate project roots.

Fields:

- `dbnum`: DB number from the file header.
- `db_type`: normalized DB type from the file header.
- `session_page`: session/page freshness indicator used to prefer the latest system-library file when duplicates exist within a project.
- `file_name`: display file name.
- `file_path`: canonical file path.
- `source_project`: source project name.

Validation:

- Entries with missing dbnum or db type are ignored.
- Hidden/dotted files are ignored.
- Canonical path deduplication prevents duplicate inventory rows.

## SystemLibrarySource

Represents a parsed system-library file that can enumerate MDB candidates.

Fields:

- `db_type`: one of `SYST`, `GLOB`, `GLB`.
- `dbnum`: DB number from the file header.
- `file_name`: display file name.
- `file_path`: canonical file path.
- `source_project`: project containing the source.
- `priority`: source priority, with `SYST` before `GLOB` before `GLB`.

Relationships:

- A system library can yield zero or more `MdbCandidate` records.
- For the same source project and MDB name, the first candidate after source-priority ordering wins.

## MdbCandidate

Represents one MDB discovered from parsed system-library content.

Fields:

- `mdb_name`: normalized MDB name with leading slash.
- `project`: source project where the MDB was discovered.
- `source_file`: system-library file path used as evidence.
- `source_db_type`: system-library DB type used as evidence.
- `syst_file`: compatibility alias for historical consumers.
- `dbnums`: member DB numbers in declared order.
- `db_files`: `MemberDbFileStatus` rows for each member.
- `available_count`: number of member DBs located uniquely.
- `missing_count`: number of member DBs not located.
- `ambiguous_count`: number of member DBs with multiple candidate files.
- `type_counts`: aggregate member counts by DB type.
- `ready_to_deploy`: true only when missing and ambiguous counts are both zero.

Validation:

- Empty or unset MDB names are ignored.
- MDBs with no member DBs are skipped with a warning.
- Duplicate project/MDB pairs are deduplicated after source-priority ordering.

## MemberDbFileStatus

Represents locate status for one MDB member DB.

Fields:

- `dbnum`: member DB number.
- `db_type`: DB type from located file header, or fallback type inferred from system-library member metadata.
- `db_name`: member DB name from system-library metadata.
- `file_name`: located file name when available.
- `file_path`: located canonical file path when available.
- `source_project`: project containing the located DB file when available.
- `status`: `available`, `missing`, or `ambiguous`.
- `candidates`: all candidate paths when ambiguous.

Validation:

- `available` requires exactly one candidate file.
- `missing` requires zero candidate files.
- `ambiguous` requires two or more candidate files.

## QuickDeployResolution

Represents the normalized request after MDB-name discovery succeeds.

Fields:

- `projects`: resolved E3D source project collection.
- `project_path`: primary project path.
- `dbnum`: target DB number selected for deploy.
- `db_file`: target DB file selected for deploy.
- `project_name`: provided name or primary project-derived default.
- `warnings`: discovery warnings and resolution summary.

Validation:

- The requested MDB must match exactly one candidate after normalization.
- Target DB selection uses explicit `dbnum`, explicit `db_file`, or a single deployable design member.
- Missing or ambiguous MDB members reject resolution.
- Multiple matching target DB candidates reject resolution with details.
