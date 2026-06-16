# Feature Specification: Custom Project Output Namespace

**Feature Branch**: `[016-custom-project-output-namespace]`

**Created**: 2026-06-16

**Status**: Draft

**Input**: User description: "站点部署时默认使用我提供的自定义项目名；一个系统项目可以是多个 E3D 项目的集合；项目名决定数据库名称、目录名称、外部访问名称等平台命名空间；included_projects 仍保持 E3D 的名称和路径；部署站点可以重命名，不要求迁移旧输出；修复 CATA closure manifest 路径不一致导致 CATA 全量解析；修复模型生成前自动补 indextree 时误用部署项目名导致生成失败。"

## Grill-Me Analysis Decisions

| Decision Branch | Recommended Answer | Rationale |
|---|---|---|
| Deployment project identity | The admin-provided custom project name is the default managed-site runtime identity and collection-level namespace. | Operators need one platform project name for database, directories, and external access, even when the deployment contains multiple E3D source projects. |
| Source E3D project identity | `included_projects` and `project_dirs` continue to use the original E3D project names and paths. | Source scanning must stay tied to real E3D files; renaming deployment identity must not mutate source project identity. |
| Output namespace | Generated `scene_tree`, `parquet`, CATA closure manifest, DbOption project name, and runtime database namespace use the custom project name. | All managed-site runtime artifacts must agree on one deployment identity to avoid path mismatches. |
| Existing output migration | Do not migrate historical output directories. | Re-deploy/re-parse should regenerate artifacts under the current custom project name; old output can remain as history/stale artifacts. |
| CATA partial-parse bug | Treat manifest path mismatch and manifest dependency type mismatch as primary defects; parse type defaulting is covered as a regression guard. | The observed sites either read the manifest from the wrong output namespace or generated a closure manifest containing DESI template dependencies that were dropped by CATA-only plan alignment. |
| Model generation precheck bug | Tree/db-meta auto-generation must iterate real source E3D projects, not the deployment/output project name. | The observed `quicktest-250160-8080` failure used `project_name=9001` as a source project and panicked before model generation could begin. |
| Room computation default | Room computation is optional and must not run automatically after generation unless explicitly enabled. | A scoped room compute cleanup failure can incorrectly fail an otherwise successful model generation flow; room computation is a downstream optional analysis step. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Custom Name Owns Runtime Output (Priority: P1)

As an admin creating or redeploying a managed site from one or more E3D projects, I need the project name I typed in the admin UI to be the default runtime/output namespace so that generated artifacts, database namespace, external access name, and viewer files are grouped under the deployment name I chose.

**Why this priority**: This fixes the direct identity split where a site named `9002` generated files under `AvevaPlantSample`, causing later orchestration to read the wrong paths.

**Independent Test**: Create or redeploy a site with custom project name `9002` and source E3D project `AvevaPlantSample`; verify generated DbOption files and generated output root use `9002` for runtime output paths.

**Acceptance Scenarios**:

1. **Given** a site has custom project name `9002` and source project path ending in `AvevaPlantSample`, **When** parse/generate configuration is written, **Then** the runtime `project_name` and output subfolder are `9002`.
2. **Given** the same site is redeployed, **When** derived output is regenerated, **Then** new `scene_tree`, `parquet`, and closure artifacts are written under the custom project name namespace.
3. **Given** old output exists under the source E3D name, **When** the site is redeployed after this feature, **Then** old output is not migrated and does not become the active lookup path.

---

### User Story 2 - Preserve E3D Source Project Names (Priority: P1)

As an admin deploying from one or more E3D projects, I need the original E3D project names and paths to remain unchanged in source selection so that the parser continues to find the correct DB files.

**Why this priority**: The custom deployment name is not necessarily a real E3D project folder; replacing source identities would break source scanning.

**Independent Test**: Generate a site configuration for custom project name `9002` with source project `AvevaPlantSample`; verify `included_projects` contains `AvevaPlantSample` and `project_dirs` points at the real E3D project path.

**Acceptance Scenarios**:

1. **Given** the admin renames the managed deployment, **When** DbOption files are generated, **Then** `included_projects` still contains the original E3D project names.
2. **Given** a multi-project site has several source E3D projects, **When** the deployment project name changes, **Then** each source project entry keeps its original name and directory.
3. **Given** a source E3D path does not match the custom deployment name, **When** parse preview runs, **Then** it resolves DB files from the source E3D path, not from a synthetic custom-name folder.

---

### User Story 3 - CATA Closure Uses Active Output Namespace (Priority: P1)

As an admin using partial CATA parsing, I need the generated CATA closure manifest to be read from the same output namespace where it was written, and I need every manifest-covered catalog/template dependency to enter the final parse plan, so that partial CATA parsing actually provides the data required by model generation.

**Why this priority**: The observed regressions happened because the manifest either existed under a different project subfolder or covered DESI template dependencies that were filtered out by CATA-only plan alignment.

**Independent Test**: Redeploy `manual_db_nums=[250160]` with `cata_partial_parse=true`; verify the log reads `output/<custom_project_name>/scene_tree/cata_closure.json` and the final parse plan contains the manifest-covered CATA/DESI dependency DBs.

**Acceptance Scenarios**:

1. **Given** CATA closure generation succeeds, **When** web_server aligns the parse plan, **Then** it reads the manifest under the custom project output namespace.
2. **Given** the manifest covers dependency DBs `[7015, 250124, 250162, 250193]`, **When** final parse config is written, **Then** manifest-covered CATA/DESI dependency DB files are included and manifest-outside CATA DB files are removed from the final plan.
3. **Given** manifest reading fails, **When** parse continues or fails according to existing fail-open behavior, **Then** the log clearly states the exact path that failed and does not silently claim partial narrowing succeeded.

---

### User Story 4 - Prevent Accidental Full CATA Selection (Priority: P2)

As an admin configuring a scoped deployment, I need quick deploy, create, edit, and preview flows to preserve explicit parse DB type intent so that an empty or scoped selection does not silently become full system parsing.

**Why this priority**: The path mismatch is the direct bug, but defaulting/editing behavior can still make operators think a scoped parse was requested when the saved site contains all parse DB types.

**Independent Test**: Create, edit, clone, and quick-deploy a scoped site; verify saved `parse_db_types` match the selected preset or explicit choice and do not become all supported types unless the admin chose full system data.

**Acceptance Scenarios**:

1. **Given** quick deploy selects a scoped/default preset, **When** the site is saved, **Then** `parse_db_types` reflects that preset and is not silently expanded to all supported types.
2. **Given** an existing site has an empty `parse_db_types` from an older record, **When** the edit UI opens, **Then** the UI shows a clear default/preset state without accidentally saving full system parsing unless the admin confirms it.
3. **Given** full system parsing is selected, **When** the site is saved, **Then** all supported parse DB types are persisted intentionally and shown as full system parsing in details.

---

### User Story 5 - Model Generation Precheck Uses Source Projects (Priority: P1)

As an admin generating a model after renaming a managed site, I need generation prechecks such as missing tree/db-meta repair to use the real source E3D projects so that custom deployment names do not break model generation.

**Why this priority**: A parsed site can still fail during model generation before geometry work begins if precheck treats a display/runtime project name such as `9001` as an E3D source project.

**Independent Test**: For a site with custom project name `9001`, source project `AvevaPlantSample`, and `manual_db_nums=[250160]`, remove or miss the active tree/db-meta artifact, trigger model generation, and verify precheck regenerates using `AvevaPlantSample` rather than failing on `project_path(9001)`.

**Acceptance Scenarios**:

1. **Given** active tree/db-meta artifacts are missing and the site project name differs from the source E3D project, **When** model generation starts, **Then** precheck uses `included_projects`/source project paths to regenerate prerequisites.
2. **Given** no source project can be resolved from `included_projects` or legacy fallbacks, **When** precheck attempts regeneration, **Then** generation fails with an actionable configuration error and not a panic.
3. **Given** multiple source E3D projects are configured, **When** precheck regenerates tree/db-meta artifacts, **Then** it processes each resolvable source project without using the deployment output name as a source root.

---

### Edge Cases

- Custom project name differs only by case from an existing output namespace on Windows.
- Custom project name contains whitespace or characters that are valid in UI but unsafe for filesystem folders.
- A previous deployment already has stale output under the source E3D project name.
- Multi-project sites have one custom deployment name but multiple E3D source names.
- `cata_closure.json` exists under both old and new output namespaces.
- CATA closure generation succeeds but produces an empty manifest.
- Parse plan contains CATA entries without reliable DB number metadata.
- Older records have empty `parse_db_types` or pre-feature site-id/source-name mixed outputs.
- `project_name` is numeric or otherwise valid as an output/database name but is not an E3D source project folder.
- Generation precheck runs when `db_meta_info.json` or tree files are missing after a rename.
- `included_projects` contains multiple projects and only some are resolvable on disk.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST treat the admin-provided custom `project_name` as the managed-site runtime identity for generated configuration, runtime database namespace, external access name, and active output subfolders.
- **FR-002**: System MUST keep original E3D project names and directories in source-selection fields such as `included_projects` and `project_dirs`.
- **FR-003**: System MUST ensure parse, generate, validation, viewer URL construction, parquet validation, scene tree lookup, and CATA closure manifest lookup use the same active output project namespace.
- **FR-004**: System MUST write regenerated `scene_tree`, `parquet`, parse manifests, metrics, and CATA closure files under the custom project output namespace after redeploy/re-parse.
- **FR-005**: System MUST NOT automatically migrate or delete older output folders created under source E3D project names.
- **FR-006**: System MUST align the final CATA parse plan with the manifest read from the active custom output namespace when CATA partial parse succeeds, including manifest-covered CATA and DESI template dependencies.
- **FR-007**: System MUST preserve existing fail-open behavior when the CATA manifest is missing or unreadable, while logging the attempted manifest path and the fact that narrowing did not happen.
- **FR-008**: System MUST log the custom output namespace, manifest path, CATA file count before alignment, and CATA file count after alignment for each partial CATA parse.
- **FR-009**: System MUST prevent quick deploy/create/edit flows from silently expanding scoped or empty `parse_db_types` into full system parsing unless full system parsing is explicitly selected.
- **FR-010**: System MUST display persisted parse DB type state clearly enough for an admin to distinguish scoped/default parsing from full system parsing.
- **FR-011**: System MUST keep existing managed sites readable and redeployable without requiring historical output migration.
- **FR-012**: System MUST support multi-project sites by keeping source E3D project identities separate from the single managed deployment output identity.
- **FR-013**: System MUST use source E3D project identities, not deployment/output project identity, when automatically regenerating tree or db-meta prerequisites before model generation.
- **FR-014**: System MUST fail with an actionable error when no source project path can be resolved for precheck regeneration.
- **FR-015**: System MUST avoid panics in source-project path resolution during parse, precheck, and generation workflows.
- **FR-016**: System MUST allow the managed project name to be independent from every source E3D project name, including for multi-project deployments.
- **FR-017**: System MUST treat room computation as an optional downstream step; managed-site generation MUST skip automatic room computation unless an explicit operator/system opt-in is enabled.

### Key Entities

- **Managed Project Site**: The admin-managed deployment record with `site_id`, custom `project_name`, one or more source E3D projects, parse settings, ports, and runtime status.
- **Custom Project Identity**: The admin-provided deployment/collection name used for runtime database namespace, active output subfolders, and externally visible project access.
- **Source E3D Project Identity**: The original E3D project name and path used to discover DB files.
- **Output Namespace**: The project-named subfolder under a site output root containing `scene_tree`, `parquet`, manifests, and metrics for the active deployment identity.
- **CATA Closure Manifest**: The generated manifest whose covered CATA and template dependency DB numbers define the final CATA partial parse plan.
- **Parse Type Selection**: The persisted set of DB types selected by quick deploy, create, edit, or preset flows.
- **Generation Precheck**: The prerequisite verification/repair step before model generation that ensures tree files, db-meta information, and transform coverage exist for target dbnums.
- **Room Computation**: Optional downstream spatial relationship analysis that may be run after model data exists, but is not required for model generation success.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For a custom project `9002` backed by E3D source `AvevaPlantSample`, 100% of newly generated active output artifacts are created under `output/9002` after redeploy/re-parse.
- **SC-002**: The same site keeps `included_projects=["AvevaPlantSample"]` and the real source E3D path in generated config.
- **SC-003**: A partial CATA parse with manifest-covered dependency DBs `[7015, 250124, 250162, 250193]` produces a final parse plan containing those covered CATA/DESI dependency files and no manifest-outside CATA DB files.
- **SC-004**: Parse logs include the manifest path and CATA before/after count for every CATA partial parse attempt.
- **SC-005**: Existing stale output under old source-project folders remains untouched, and redeploy succeeds without manual migration.
- **SC-006**: Scoped quick deploy/create/edit flows persist the selected parse DB types accurately in all tested scenarios.
- **SC-007**: A renamed site with missing tree/db-meta prerequisites reaches model generation precheck without panic and either regenerates prerequisites from the real source project or returns a clear configuration error.
- **SC-008**: In the `quicktest-250160-8080` rename scenario, model generation no longer fails with `called Option::unwrap() on a None value` during tree/db-meta precheck.

## Assumptions

- `site_id` remains the immutable technical identity for routes, logs, task ownership, and process coordination.
- Custom `project_name` is validated and normalized before being used as a folder or database namespace.
- Source E3D project names are still required for DB discovery and must not be overwritten by deployment rename semantics.
- Old output directories can remain on disk; cleanup/migration is a separate explicit maintenance feature.
- Validation uses admin service HTTP/POST flows, CLI/json checks, runtime file inspection, and log inspection; repository rules prohibit adding/running `cargo test` for `web_server`.
- Model generation may invoke shared parse/precheck helpers; those helpers must treat `project_name` as output identity and `included_projects` as source identity in managed-site configs.
