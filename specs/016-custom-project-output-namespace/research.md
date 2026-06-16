# Research: Custom Project Output Namespace

## Decision: Custom project name is the platform project namespace

**Decision**: Treat the admin-provided custom `project_name` as the managed platform project namespace. It names the runtime database, generated directories, externally visible project access, and active output namespace for a deployment that may include multiple E3D source projects.

**Rationale**: The managed project is a collection boundary in this system, not a mirror of one E3D project. Operators need a stable project name for deployment/runtime access while preserving each source E3D project's original identity for file discovery.

**Alternatives considered**:

- Require the managed project name to match one source E3D project. Rejected because multi-project deployments need a separate collection name.
- Derive database or external access names from the first E3D project. Rejected because it makes deployment identity unstable when the source set changes.
- Rename E3D source projects to match the managed project. Rejected because source DB discovery depends on real project names and paths.

## Decision: Custom project name is the active output namespace

**Decision**: Use the admin-provided custom `project_name` as the active runtime/output namespace for generated DbOption `project_name`, runtime database namespace, `output/<project_name>`, scene tree, parquet, and CATA closure manifest lookup.

**Rationale**: The operator can rename the managed deployment independently of the original E3D source folder. In the observed site, custom project name `9002` was the managed deployment identity, but sidecar output was under `AvevaPlantSample`; web_server then tried to read `output/9002/scene_tree/cata_closure.json` and failed. The correct fix is not to make web_server chase source names for output, but to ensure all regenerated runtime artifacts consistently use the custom deployment name.

**Alternatives considered**:

- Read generated artifacts from the source E3D name. Rejected because it makes custom deployment names cosmetic and keeps runtime output split from admin identity.
- Rename source E3D projects to the custom name. Rejected because source DB discovery depends on real project names and paths.
- Migrate old output folders. Rejected because user explicitly does not require migration and redeploy can regenerate active output.

## Decision: Preserve E3D source names in `included_projects`

**Decision**: Keep original E3D source project names and paths in `included_projects` and `project_dirs`.

**Rationale**: These fields are source-selection fields, not deployment identity fields. They tell the sidecar where source DB files live. A custom deployment name may not exist as an E3D project folder, so replacing `included_projects` would break source lookup.

**Alternatives considered**:

- Set `included_projects` to the custom project name. Rejected because it would make source scanning depend on a synthetic folder/name.
- Duplicate both custom and source names in `included_projects`. Rejected because it can create ambiguous or duplicate source roots.

## Decision: Active output consumers use deployment identity

**Decision**: Any code that reads generated artifacts should use the custom deployment identity, not source identity.

**Rationale**: CATA closure manifest, scene tree, parquet, and viewer project lookup are generated runtime artifacts. If writers and readers disagree on the project subfolder, partial parsing and viewer loading fail even when artifacts were successfully generated.

**Alternatives considered**:

- Add fallback probing between custom and source names. Rejected as a primary behavior because it masks split-brain output and can accidentally read stale artifacts. A diagnostic fallback may be acceptable only for error messages, not as the active path.

## Decision: CATA partial parse remains fail-open on missing manifest

**Decision**: If CATA closure generation fails, or the manifest cannot be read from the active output namespace, preserve existing fail-open behavior and avoid narrowing CATA files.

**Rationale**: Missing manifest must not silently under-parse CATA. The safe behavior is to keep the conservative plan and log why narrowing did not happen.

**Alternatives considered**:

- Fail the whole parse when manifest is missing. Rejected because current semantics already tolerate manifest absence, and changing that behavior would increase deployment failures.
- Remove all CATA when manifest is missing. Rejected because it could produce incomplete parsed data without a clear failure.

## Decision: Do not migrate historical output

**Decision**: Existing folders such as `output/AvevaPlantSample` are left untouched. A re-parse/redeploy writes active artifacts under `output/<custom_project_name>`.

**Rationale**: Migration is not required for the reported bug and could be risky if old outputs are being used for diagnostics. The active path must become correct after regeneration.

**Alternatives considered**:

- Move old output into the custom namespace. Rejected because it can mix artifacts generated with different DbOption identities.
- Delete old output automatically. Rejected because it removes potentially useful forensic data.

## Decision: Parse DB type defaults need regression guardrails

**Decision**: Treat parse type defaulting/editing as a P2 regression guard: scoped and empty selections must not become full system parsing unless the admin explicitly selects full system parsing.

**Rationale**: The direct CATA full-parse cause was manifest path mismatch. However, the observed saved site also contained all parse DB types, and UI defaults can confuse operators about whether a scoped parse or full parse was requested.

**Alternatives considered**:

- Scope this feature only to path alignment. Rejected because a path fix alone still leaves a separate path to unexpected full CATA parsing.
- Remove full-system parsing option. Rejected because full parsing remains a valid operator action.

## Decision: Generation precheck uses source project identities

**Decision**: When model generation precheck must regenerate `db_meta_info.json` or tree/index artifacts, it must iterate resolvable `included_projects` and their source paths. It must not use custom deployment `project_name` as the source project argument.

**Rationale**: Managed-site DbOption files intentionally mix identities: `project_name` is the runtime/output/database namespace, while `included_projects` and `project_dirs` are source E3D discovery inputs. The observed `quicktest-250160-8080` failure happened because precheck used `project_name=9001` as a source project, `get_project_path("9001")` returned `None`, and the parse/tree generation path panicked before model generation.

**Alternatives considered**:

- Make `project_name` equal the source E3D project during generation. Rejected because it would undo the custom output namespace fix and recreate SurrealDB/output identity drift.
- Add a fallback that tries `project_name` and then source projects. Rejected as primary behavior because it preserves the wrong mental model and can hide bad managed-site config; a diagnostic error may mention both.
- Skip tree/db-meta auto-generation when renamed. Rejected because a parsed site may legitimately need precheck repair before generation.

## Decision: Room computation is optional by default

**Decision**: Disable automatic room computation after managed-site generation unless an explicit opt-in is provided.

**Rationale**: Room computation is downstream analysis, not a prerequisite for model generation. The observed failure completed the room computation body, then failed while restoring the default SQLite spatial index because `250160.tree` was unavailable to the separate sidecar process. That cleanup failure should not block model generation or mark the site failed by default.

**Alternatives considered**:

- Keep automatic room computation mandatory. Rejected because a room-specific cleanup failure can obscure successful model generation.
- Remove room computation entirely. Rejected because CLI/manual room computation remains useful as an optional operation.
- Silently ignore all room computation errors. Rejected because explicit room computation should still report real failures to the operator.
