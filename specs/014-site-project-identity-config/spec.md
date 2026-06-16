# Feature Specification: Site Project Identity & Parse Configuration

**Feature Branch**: `[014-site-project-identity-config]`

**Created**: 2026-06-15

**Status**: Draft

**Input**: User description: "分析在站点部署时，(1）数据库的启动应该是按project name 进行按不同的文件目录来分隔的  （2）部分解析CATA这个选项是默认就开启的，在站点详情里也应该能看到这个配置选项，还有部分解析的选项，都要在站点编辑和创建里都能看得到。（3）即使部署完成了，还是能提供编辑project name 的功能，需要重命名一些对应的文件夹，以及 e3d 的数据库名和文件夹名称。使用 grill-me skill 帮我分析, 并编写 spec kit"

## Grill-Me Analysis Decisions

| Decision Branch | Recommended Answer | Rationale |
|---|---|---|
| Project identity source | `project_name` remains the business identity and also drives the generated E3D database name; `site_id` remains the immutable technical row/process identity. | Existing records and routes already rely on stable `site_id`; changing it would break links and process ownership. |
| Runtime/database folder isolation | Runtime and data folders MUST be segmented by project identity, with site-id compatibility handled by migration/lookup rather than mixed writes. | The user request is about project-name-level isolation, not port-level identity. |
| Rename timing | Project rename is allowed only when the site is not running and no parse/generate/deploy task is active. | Avoid moving live SurrealDB files, generated output, logs, and DbOption files under running processes. |
| Rename semantics | Rename is an atomic managed operation: validate target, stop blockers, move owned folders/config names, rewrite DbOption files, then update the admin record. | Partial rename creates hard-to-debug split state. |
| CATA partial parse default | `cata_partial_parse=true` by default whenever CATA related parsing is enabled; users can explicitly turn it off in create/edit. | Existing model already documents `true` as default; UI must surface it consistently. |
| Partial parse visibility | Show both `auto_parse_related_dbnums` and `cata_partial_parse` in create, edit, preview, and site details. | Operators need to know why CATA is partial, full, or skipped. |
| Deployed-site editability | Deployed/parsed sites can still edit `project_name`, but the operation is treated as a rename workflow rather than a silent field update. | The change touches filesystem and E3D identity, so it deserves explicit validation and outcome reporting. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Isolated Database Startup By Project Name (Priority: P1)

As an admin deploying multiple sites, I need each site's database startup to use directories separated by project name so that one site's SurrealDB data, generated output, and E3D database identity do not collide with another site.

**Why this priority**: Without deterministic project-level separation, deployment can reuse or overwrite data directories when site ids, ports, or copied deployments change.

**Independent Test**: Create two sites with distinct project names and deploy/start each; verify each site reports project-specific runtime/data paths and uses its own E3D database name.

**Acceptance Scenarios**:

1. **Given** two sites with different project names, **When** both are started, **Then** each database process uses a data directory derived from its own project name and not the other site's directory.
2. **Given** a site has a configured project name, **When** its DbOption files are generated, **Then** the database/project identifiers inside generated configuration match the current project name.
3. **Given** a target project-name directory already exists and belongs to another site, **When** a create or rename operation is requested, **Then** the operation is rejected with a conflict message before any process starts.

---

### User Story 2 - Visible Partial Parse Configuration (Priority: P1)

As an admin configuring a site, I need CATA partial parsing and related partial parse options to be visible and editable in both create and edit flows, and visible on the site details page after deployment.

**Why this priority**: Operators currently cannot reliably tell whether CATA parsing is partial, full, or dependency-driven, which makes deployment results hard to reason about.

**Independent Test**: Open create, edit, and site detail views for a site; verify the same parse configuration is displayed and saved across all three surfaces.

**Acceptance Scenarios**:

1. **Given** the admin opens the create-site drawer, **When** CATA or dependency parsing is selected, **Then** CATA partial parse is enabled by default and can be toggled off.
2. **Given** an existing site has `auto_parse_related_dbnums` or `cata_partial_parse` configured, **When** the edit drawer opens, **Then** those settings are prefilled and can be saved.
3. **Given** a deployed site is viewed in details, **When** the configuration tab renders, **Then** it shows parse DB types, dependency partial parse, CATA partial parse, system rebuild policy, and the active preset/custom status.

---

### User Story 3 - Rename Project Name After Deployment (Priority: P2)

As an admin, I need to rename a deployed site's project name when the real engineering project name changes, and I need the system to rename owned folders and E3D database identifiers consistently.

**Why this priority**: Project names are user-visible and operationally meaningful; blocking edits after deployment forces unsafe manual filesystem/database edits.

**Independent Test**: Deploy a site, stop it, rename its project name, then start it again and verify paths, generated configs, and detail UI all reflect the new name while prior logs/metrics remain associated with the site.

**Acceptance Scenarios**:

1. **Given** a deployed site is stopped and has no active task, **When** the admin changes `project_name`, **Then** the system previews affected folders/names and applies the rename atomically after confirmation.
2. **Given** a site is running or a parse/generate/deploy task is active, **When** the admin attempts to rename `project_name`, **Then** the system rejects the rename with the required stop/wait action.
3. **Given** a rename succeeds, **When** the site is started again, **Then** the database uses the renamed data folder and the E3D database/project name matches the new project name.
4. **Given** any folder move or config rewrite fails during rename, **When** the operation returns, **Then** the admin sees a failure with no mixed project-name state recorded as successful.

---

### Edge Cases

- Target project name normalizes to the same slug as an existing site or directory.
- Project name differs only by case on Windows filesystems.
- Old runtime/data/output folders exist but are empty, stale, or partially migrated from a failed earlier attempt.
- The site is stopped but external processes still hold files under the old data directory.
- Generated output contains project-name subfolders such as `output/<project_name>/scene_tree` that need rename or regeneration guidance.
- Logs, metrics, and historical task records must remain attached to `site_id` even when project name changes.
- Existing older sites may still have site-id-based runtime paths and need compatibility/migration behavior.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST treat `site_id` as immutable technical identity and `project_name` as editable business/database identity.
- **FR-002**: System MUST derive new database data, generated output, and E3D database identifiers from the current `project_name` for newly created sites.
- **FR-003**: System MUST prevent two managed sites from using the same normalized project identity unless they are the same site being edited.
- **FR-004**: System MUST show `auto_parse_related_dbnums` and `cata_partial_parse` controls in both site creation and site editing flows.
- **FR-005**: System MUST default CATA partial parse to enabled for new sites unless the admin explicitly disables it.
- **FR-006**: System MUST persist partial parse settings and include them in parse preview payloads, site detail payloads, and generated site configuration.
- **FR-007**: System MUST display dependency partial parse and CATA partial parse state in the site details configuration section.
- **FR-008**: System MUST allow editing `project_name` after deployment through a managed rename workflow.
- **FR-009**: System MUST reject project rename while the site is running, starting, stopping, parsing, generating, deploying, or otherwise has an active managed task.
- **FR-010**: System MUST provide a rename preview listing affected folders, config files, generated output/project folders, and E3D database/project names before applying a deployed-site rename.
- **FR-011**: System MUST apply project rename atomically from the admin user's perspective: either all recorded identity/path/config changes succeed, or the site remains in the previous usable identity with a clear error.
- **FR-012**: System MUST rewrite generated DbOption/config files after project rename so subsequent parse/generate/start actions use the new project identity.
- **FR-013**: System MUST preserve historical logs, metrics, task history, and site id references across project rename.
- **FR-014**: System MUST report path conflicts, invalid project names, permission failures, and file-lock failures with actionable messages.
- **FR-015**: System MUST keep older site-id-based sites readable and startable until they are migrated or renamed.

### Key Entities

- **Managed Project Site**: A managed deployment record with immutable `site_id`, editable `project_name`, ports, parse settings, runtime paths, and status.
- **Project Identity**: The normalized project-name identity used to derive folders and E3D database/project names.
- **Parse Configuration**: The chosen parse DB types, dependency parsing flag, CATA partial parse flag, and system rebuild policy.
- **Rename Plan**: A preview of old/new names, affected paths, blockers, and actions required to safely rename a site's project identity.
- **Runtime Artifact Set**: Owned folders/files that may be renamed or regenerated, including data directory, generated output, DbOption files, parse manifests, and project-specific subfolders.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of newly created sites expose project-specific database data paths in the site details view.
- **SC-002**: Admins can identify the CATA partial parse setting from site details in under 10 seconds without reading logs.
- **SC-003**: A stopped deployed site can be renamed and restarted successfully without manual filesystem edits.
- **SC-004**: Rename attempts with active processes or conflicting target names are blocked before filesystem changes begin.
- **SC-005**: Existing logs and task metrics remain visible after rename for the same site id.
- **SC-006**: Create/edit/preview/detail surfaces show consistent parse configuration values for at least `parse_db_types`, `auto_parse_related_dbnums`, `cata_partial_parse`, and `force_rebuild_system_db`.

## Assumptions

- `site_id` remains stable to avoid breaking routes, task ownership, log history, and process registry ownership.
- Project rename is a privileged admin action; no separate role model is introduced in this feature.
- Rename is initially local-admin only; remote deployed hosts require a separate remote synchronization feature unless already covered by existing remote deployment flows.
- If a project-specific generated output folder cannot be safely moved, the admin can be told to regenerate derived outputs after the identity rename completes.
- Validation uses admin UI/API and CLI/json workflows; the repository rule forbids adding or running cargo test/test-target validation.
