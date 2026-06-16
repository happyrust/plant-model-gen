# Feature Specification: Site Project Isolation And Post-Deploy Editing

**Feature Branch**: `014-site-project-isolation-and-editing`

**Created**: 2026-06-15

**Status**: Draft

**Input**: User description: "站点部署时数据库启动应按 project name 使用不同文件目录隔离；默认开启的部分解析 CATA 和其它部分解析选项应在站点详情、创建和编辑里可见；部署完成后仍可编辑 project name，并重命名对应文件夹、E3D 数据库名和文件夹名称。使用 grill-me skill 分析并编写 spec kit。"

## User Scenarios & Testing

### User Story 1 - Use Project Name As Database Storage Boundary (Priority: P1)

As an admin deploying multiple managed sites, I need the SurrealDB data directory used during site startup to be derived from the site project name instead of only the site id, so each E3D project has a clearly separated database folder and cannot accidentally reuse another project's data.

**Why this priority**: This is the runtime correctness boundary. If DB data is tied only to `site_id`, later project renames, cloned sites, and deployment retries make it hard to reason about which project owns which database files.

**Independent Test**: Create or update two managed sites with distinct `project_name` values, deploy/start them through HTTP admin APIs, then verify each site reports and uses a distinct project-name-based `db_data_path` in the API response, `DbOption*.toml`, and DB startup logs.

**Acceptance Scenarios**:

1. **Given** a site with `project_name = AvevaPlantSample`, **When** the site is created or started, **Then** its `db_data_path` is derived from a normalized project identity based on `AvevaPlantSample`.
2. **Given** two sites have different project names, **When** both are deployed or started, **Then** their SurrealDB data directories are different even if their web/db ports are reassigned.
3. **Given** the generated `DbOption.toml`, `DbOption-parse.toml`, and `DbOption-generate.toml` are inspected, **When** DB mode is `ws` or `file`, **Then** every SurrealDB path points to the project-name-based data directory.
4. **Given** a project-name-derived data directory already exists for another site, **When** a site tries to claim that same project identity, **Then** the backend rejects the operation with a clear conflict instead of overwriting data.

---

### User Story 2 - Expose Partial Parse Options Everywhere (Priority: P1)

As an admin configuring a site, I need to see and edit the CATA partial parse and related partial parse options in create, edit, and site detail views, so the deployed behavior matches the visible configuration.

**Why this priority**: `cata_partial_parse` is already default-enabled in the data contract. If the UI hides it in any important view, users cannot explain parse logs, closure manifest behavior, or why a CATA library was partially parsed/skipped.

**Independent Test**: Open the create drawer, edit drawer, and site detail page; verify `auto_parse_related_dbnums`, `cata_partial_parse`, and parse DB type/partial parsing explanation are visible and persisted through save/reload.

**Acceptance Scenarios**:

1. **Given** a user opens the new-site drawer, **When** default values render, **Then** "自动解析依赖库" and "按需解析 CATA（部分解析）" are visible, with `cata_partial_parse` defaulting to enabled.
2. **Given** a user edits an existing site, **When** the drawer loads current site data, **Then** the partial parse controls reflect the persisted backend values.
3. **Given** a user opens site detail, **When** the configuration section renders, **Then** it displays whether automatic related DB parsing and CATA partial parsing are enabled, including a concise explanation of partial parsing behavior.
4. **Given** `auto_parse_related_dbnums` is disabled, **When** `cata_partial_parse` is displayed, **Then** the UI explains that CATA partial parse only takes effect when automatic related DB parsing is enabled.

---

### User Story 3 - Rename Project Name After Deployment (Priority: P1)

As an admin maintaining deployed sites, I can change a site's `project_name` after deployment so that corrected project naming is reflected in the managed registry, generated config, local runtime folders, database folder, and E3D-facing configuration values.

**Why this priority**: Project names often start as quick-deploy defaults and later need to become stable operational names. Forcing users to recreate a site just to fix the project name risks losing parsed/generated artifacts.

**Independent Test**: Deploy a site, stop it if it is running, update `project_name` via `PUT /api/admin/sites/{id}`, and verify the API response, generated configs, metadata, output/project folders, and DB data directory are renamed consistently.

**Acceptance Scenarios**:

1. **Given** a deployed but stopped site, **When** `project_name` is changed from `OldName` to `NewName`, **Then** the site record, metadata, config files, project-name-derived DB path, and generated output project folder all use `NewName`.
2. **Given** the old project-name DB directory exists, **When** the rename succeeds, **Then** the directory is moved to the new project-name path and the old path is not used by later starts.
3. **Given** the site is currently running or parsing/generating, **When** `project_name` rename is requested, **Then** the backend rejects the rename with an actionable message to stop the site first.
4. **Given** a target project-name directory already exists and is non-empty, **When** a rename would overwrite it, **Then** the backend rejects the rename and leaves the original site unchanged.
5. **Given** rename fails during file migration, **When** the API returns an error, **Then** the registry row, config files, and folders are not left in a mixed old/new state.

---

### User Story 4 - Preserve Existing Deployment Behavior Where Names Do Not Change (Priority: P2)

As an operator using existing sites, I need normal edit/deploy/start flows to continue working when `project_name` is unchanged, so this feature does not introduce unnecessary migration work.

**Why this priority**: The change touches core site lifecycle paths. Backward compatibility for unchanged project names prevents regressions in ordinary config edits.

**Independent Test**: Update an existing site's non-name fields, deploy/start the site, and verify no DB directory migration occurs and the site continues to use its current project data directory.

**Acceptance Scenarios**:

1. **Given** a site edit changes only ports, parse options, or generation options, **When** the update succeeds, **Then** no project data directory is renamed.
2. **Given** an old site still has a legacy `site_id/data/surreal.db` path, **When** it starts without a project-name edit, **Then** the system either keeps the legacy path or performs an explicit one-time migration with a visible log entry.
3. **Given** quick deploy creates a new site, **When** no project name is provided, **Then** the derived project name and site name continue to follow existing quick-deploy defaults.

## Edge Cases

- `project_name` contains spaces, Chinese characters, punctuation, or case differences.
- `project_name` changes only by case on Windows.
- A stopped site has a stale `db_pid` or `web_pid` recorded.
- Rename is requested while another site points at the same data directory.
- Rename is requested after remote deployment; local metadata changes but remote data has not been migrated.
- `cata_partial_parse = true` while `auto_parse_related_dbnums = false`.
- Existing quick-deploy presets omit `cata_partial_parse` and rely on backend defaults.
- Old sites have `db_data_path` inside `runtime/admin_sites/<site_id>/data/surreal.db`.

## Grill-Me Decision Record

| Question | Recommended Answer | Status | Reasoning |
|---|---|---|---|
| Q1: What is the project identity boundary for DB startup? | Use normalized `project_name` as the human-facing storage identity, while keeping `site_id` as the stable registry key. | Recommended | Current `db_data_path(site_id)` ties storage to site id; the user explicitly wants project-name-separated folders. |
| Q2: Should `project_name` uniqueness remain enforced? | Yes, keep one active managed site per project name unless a future multi-instance feature adds explicit namespace suffixes. | Recommended | Existing create path already rejects project-name conflicts; this protects DB directory ownership. |
| Q3: Can project name be changed while the site is running? | No for v1; require stopped/non-active state before migration. | Recommended | Moving RocksDB/SurrealDB files while processes hold locks is unsafe. |
| Q4: Should rename alter original E3D source project folders under `project_path`? | No for v1; rename only managed runtime/generated folders and E3D-facing config values. Ask for explicit confirmation before touching source E3D directories. | Needs confirmation | The user said "E3D 数据库名和文件夹名称", but automatically renaming source files is destructive and outside current managed-site ownership. |
| Q5: Should remote deployed data be renamed automatically? | No for v1; update local defaults and show a remote redeploy/migration requirement. | Needs confirmation | Remote migration requires SSH side effects and rollback handling; local admin rename should not silently move remote production data. |
| Q6: Should CATA partial parse default remain enabled? | Yes. | Resolved by code | `CreateManagedSiteRequest`, `PreviewManagedSiteParsePlanRequest`, and quick deploy already default `cata_partial_parse` to true. |
| Q7: Where should partial parse options be visible? | Create drawer, edit drawer, parse preview, and site detail configuration section. | Resolved | Create/edit already pass the fields; detail currently needs explicit display. |

## Requirements

### Functional Requirements

- **FR-001**: New managed sites MUST derive `db_data_path` from a normalized project identity based on `project_name`, not only from `site_id`.
- **FR-002**: The normalized project identity MUST be deterministic, filesystem-safe, and stable across create/update/start for the same `project_name`.
- **FR-003**: The backend MUST prevent two managed sites from owning the same project-name-derived DB data directory unless they are explicitly the same `site_id`.
- **FR-004**: `DbOption.toml`, `DbOption-parse.toml`, `DbOption-generate.toml`, metadata JSON, and runtime API responses MUST all report the same effective `db_data_path`.
- **FR-005**: `ensure_site_db_started()` and file-mode DB access MUST use the effective `site.db_data_path` after any project rename.
- **FR-006**: The create and edit UI MUST show `auto_parse_related_dbnums` and `cata_partial_parse` controls with their current/default values.
- **FR-007**: The site detail UI MUST display `auto_parse_related_dbnums`, `cata_partial_parse`, and a concise explanation of whether CATA partial parsing is active.
- **FR-008**: The parse preview request MUST include `cata_partial_parse` and `auto_parse_related_dbnums` values from the current form state.
- **FR-009**: Updating `project_name` MUST remain available for a deployed site record after parsing/generation/deployment, subject to the site being stopped and not parsing/generating.
- **FR-010**: If `project_name` changes, the backend MUST migrate managed local artifacts that are keyed by project name, including DB data directory and generated output project folder names.
- **FR-011**: Project rename MUST rewrite generated configs and metadata so E3D-facing fields (`project_name`, `mdb_name` where derived, and related folder/path values) reflect the new name.
- **FR-012**: Project rename MUST be atomic from the user's perspective: on failure, the site remains usable with the old project name and old paths.
- **FR-013**: Project rename MUST reject target names that collide with an existing site project name or an existing non-empty managed project data directory.
- **FR-014**: Project rename MUST reject requests while DB/web/viewer/parse processes are running, unless a future implementation explicitly stops and verifies them before migration.
- **FR-015**: Rename operations MUST write an audit log entry containing old project name, new project name, old paths, new paths, and whether migration happened.
- **FR-016**: Existing non-name site edits MUST continue to update config and metadata without triggering project data migration.
- **FR-017**: Quick-deploy behavior MUST remain compatible when no explicit `project_name` is provided.
- **FR-018**: Remote deployment defaults MAY be recalculated from the new project name, but v1 MUST NOT silently rename remote production data without an explicit remote migration step.

### Key Entities

- **Project Data Identity**: Filesystem-safe identity derived from `project_name`, used to locate managed DB data.
- **Managed Site**: Existing admin site record keyed by `site_id`, with mutable display/config fields including `project_name`.
- **Project Rename Migration**: Backend operation that changes `project_name` and moves/rewrites managed local artifacts.
- **Partial Parse Configuration**: Visible configuration combining `auto_parse_related_dbnums` and `cata_partial_parse`.
- **Effective DB Data Path**: The path used by SurrealDB in ws/file modes after applying create/update/migration rules.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Creating and starting two sites with distinct project names produces two distinct `db_data_path` values, both visible in API responses and config files.
- **SC-002**: Changing a stopped deployed site's `project_name` updates API response, config files, metadata, output folder naming, and DB startup path without requiring site recreation.
- **SC-003**: Project rename while the site is running fails before any directory move and returns a user-visible stop-first message.
- **SC-004**: The site detail page visibly reports automatic related DB parsing and CATA partial parse status.
- **SC-005**: Toggling CATA partial parse in create/edit persists through save, reload, preview, and backend config generation.
- **SC-006**: Existing quick-deploy and normal non-name edit flows still work without forced data migration.

## Assumptions

- `site_id` remains the immutable registry/process identity; only `project_name` and project-name-derived folders are mutable.
- v1 migrates only local managed runtime/generated artifacts owned by this admin system.
- Original E3D source project folders under `project_path` are not renamed automatically until the user confirms that destructive behavior.
- Remote production data rename is out of scope for v1; users can redeploy or run a future explicit remote migration.
- Validation follows repository rules: no `cargo test`; use running web_server HTTP/POST and CLI/JSON checks where needed.

## Non-Goals

- No automatic rename of original Aveva/E3D source database files or source project directories in v1.
- No cross-machine remote data directory migration in v1.
- No support for multiple active managed sites sharing one `project_name`.
- No change to CATA closure BFS semantics.
- No redesign of `site_id`, port allocation, or process lifecycle beyond path ownership checks required by rename.
