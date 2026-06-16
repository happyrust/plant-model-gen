# Data Model: Site Project Identity & Parse Configuration

## ManagedProjectSite

Existing managed site record extended/clarified by this feature.

### Fields

- `site_id`: Immutable technical identity used by routes, task ownership, process registry, logs, metrics, and compatibility lookup.
- `project_name`: Editable business/database identity. Must be non-empty, normalized for uniqueness, and used for E3D database/project naming.
- `project_identity_slug`: Derived filesystem-safe normalized identity. May be persisted or recomputed, but API responses should expose enough path/identity state for operators.
- `project_path`: Source project root path. Existing validation still applies.
- `parse_db_types`: Selected database classes to parse, such as SYST, DESI, CATA, DICT, GLB, GLOB.
- `auto_parse_related_dbnums`: Whether dependency-related DB numbers should be automatically included.
- `cata_partial_parse`: Whether CATA dependency parsing uses partial/on-demand CATA closure instead of full CATA parse. Default true.
- `force_rebuild_system_db`: Whether SYST data is forcibly rebuilt.
- `runtime_dir`: Current effective runtime directory.
- `db_data_path`: Current effective database data path.
- `config_path`: Current effective primary DbOption path.
- `parse_status` / `status`: Used to block unsafe rename states.

### Invariants

- `site_id` never changes after create.
- `project_name` is unique by normalized, case-insensitive comparison across active managed sites.
- New create/update payloads never silently set `cata_partial_parse=false`; false must be explicit.
- Rename cannot mark success unless the admin record, effective paths, and generated configs agree on the new project identity.

## ProjectIdentity

Derived identity object used by create, start, config generation, and rename planning.

### Fields

- `display_name`: Trimmed `project_name` shown to users and written as E3D project/database name.
- `normalized_key`: Case-insensitive uniqueness key.
- `filesystem_name`: Filesystem-safe name/slug for folders.
- `e3d_database_name`: Database/project identifier used inside generated DbOption/E3D config.

### Rules

- Empty names are invalid.
- Names that normalize to the same key conflict, including case-only differences on Windows.
- Filesystem names must not contain path separators, `..`, or reserved Windows device names.

## ParseConfiguration

Configuration block shared by create, edit, preview, and detail display.

### Fields

- `parse_db_types: string[]`
- `force_rebuild_system_db: boolean`
- `auto_parse_related_dbnums: boolean`
- `cata_partial_parse: boolean`
- `manual_db_nums: number[]`
- `manual_db_files: string[]`
- `generate_db_nums: number[]`
- `generate_db_files: string[]`

### Rules

- `cata_partial_parse` defaults to true on create.
- If CATA is not selected and dependency parsing is off, CATA partial parse may still be stored but UI should explain it has no effect until CATA/dependency parsing is active.
- Preview payloads must include the effective values so the sidecar can explain planned parse scope.

## ProjectRenamePlan

Preview/apply model for project name changes.

### Fields

- `site_id`
- `old_project_name`
- `new_project_name`
- `old_identity`
- `new_identity`
- `blocked: boolean`
- `blockers: string[]`
- `affected_paths: RenamePathChange[]`
- `affected_names: RenameNameChange[]`
- `requires_regeneration: boolean`
- `warnings: string[]`

### RenamePathChange

- `kind`: `runtime_dir` | `db_data_path` | `config_file` | `parse_config_file` | `generation_config_file` | `output_project_dir` | `manifest` | `other`
- `from`
- `to`
- `exists`
- `conflict`
- `action`: `move` | `rewrite` | `preserve` | `skip` | `regenerate`

### RenameNameChange

- `kind`: `project_name` | `e3d_database_name` | `associated_project` | `display_label`
- `from`
- `to`

## ProjectRenameResult

Apply result returned to UI.

### Fields

- `site: ManagedProjectSite`
- `plan: ProjectRenamePlan`
- `applied_actions: string[]`
- `warnings: string[]`
- `message`

### Rules

- If apply fails, response must not claim success and must include the first actionable failure.
- Historical records remain keyed by `site_id` and are not rewritten merely for display name changes.
