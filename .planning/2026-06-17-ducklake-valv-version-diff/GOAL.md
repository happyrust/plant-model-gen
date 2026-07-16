# Goal: E3D Incremental Model Versioning

> **OUTDATED (2026-07-16)**：下文多处以 `release_id` 作为「用户可见版本」与 DuckLake 版本身份。  
> 已被 **`specs/023-ducklake-unit-version-by-sesno/`** 取代：交付单元真相键为 `(dbnum, refno, sesno)`；  
> `release_id` 仅作 export-batch 过渡别名。PE/ATT 源行历史见 **`specs/022-versioned-pe-att-storage/`**。  
> 读本 planning 目录时请以 023 ADR / inventory 为准，勿再按 release 图设计新功能。

## Objective

Build a production-grade incremental model versioning path for the E3D site model workflow.
The target scenario is the `AvevaMarineSample` site, especially DB `1112`, using historical
session ranges from `pdms-io`/`incremental-sesno` to verify that:

- E3D database changes can be incrementally parsed and persisted.
- Model geometry can be incrementally regenerated after those changes.
- Each generated model state can be published as an immutable release.
- Two releases can be queried, diffed, and loaded side by side for 3D comparison.

## Non-Negotiable Constraints

- Do not use `cargo test` or compile test targets.
- Validate CLI/database behavior through `aios-database` commands with JSON output.
- Validate `web_server` behavior only by running the service and calling HTTP endpoints.
- Keep SurrealDB as the generation writer for now.
- Keep Parquet as the viewer/package format for now.
- Use DuckLake as a versioned release catalog and immutable snapshot query layer, not as the
  first writer of generation output in the MVP.
- Do not depend on the current `model-writer-ducklake` path as source of truth; Oracle review
  identified gaps around transform and domain release semantics.

## Oracle-Backed Architecture Decision

Oracle's follow-up review supports a layered design:

1. Generation layer:
   - Incremental parse/save continues through the existing versioned DB and SurrealDB paths.
   - Incremental model generation continues through existing `gen_model` pipelines.
2. Package layer:
   - Post-generation Parquet export creates viewer-ready model packages.
   - Each accepted package is copied or hard-linked into an immutable release directory.
3. Version layer:
   - DuckLake stores release metadata, release graph edges, file manifests, snapshot tables,
     and later component/unit diff indexes.
   - User-facing versions are explicit `release_id`s, not raw DuckLake transaction ids.
4. Comparison layer:
   - API and UI compare two release packages.
   - The viewer should display two synchronized 3D panes and optional diff overlays.

## Success Criteria

Phase 1 is complete when:

- A CLI can register an existing generated Parquet package as a model release.
- Release registration is idempotent and validates required package files.
- Release metadata, parent edge, package files, hashes, and row counts are stored in DuckLake.
- A CLI can list releases as JSON.
- The feature builds behind a dedicated feature flag.
- The DB `1112` sample package from `AvevaMarineSample` can be registered from real output.

Phase 2 is complete when:

- Two session-derived releases can be registered from different `incremental-sesno` outputs.
- The version layer can compute component-level identity/version rows.
- Basic changed/added/deleted component diff can be queried as JSON.

Phase 3 is complete when:

- Delivery unit membership and aggregate `unit_versions` are implemented.
- Component changes propagate deterministically to affected units.
- Impact queries explain which unit changed and why.

Phase 4 is complete when:

- Web APIs expose releases, manifests, component diff, and unit impact.
- A two-pane 3D comparison interface can load two release packages and show the model delta.
- The flow has documented operational steps and failure modes.

## Current Development Slice

Phase 1 is implemented and validated. Phase 2 CLI/API MVP and the first Phase 3
unit-impact slice are implemented and validated against DB `1112` with one real
generated release plus one controlled fixture release:

- Release registration/list remains intact.
- Component snapshots are indexed from immutable release Parquet packages.
- Identity starts with `component_key = <dbnum>:<refno_u64>` and hash version
  `component_snapshot:v1`.
- `component_hash` is built from stable instance attributes plus ordered geometry rows.
- `aios-database model-version index` and `model-version diff` are available.
- HTTP APIs expose release list, component re-index, and component diff.
- HTTP APIs expose release detail and release runtime-scene JSON derived from
  immutable release Parquet packages.
- `/model-version/release-viewer` provides an internal xeokit/GLTF viewer for
  one immutable release package.
- `/model-version/compare` provides a two-pane comparison page with two
  release viewer iframes plus a diff table.
- Browser validation confirmed both panes can load DB `1112` release geometry
  from real GLB mesh files.
- `aios-database model-version index-units`, `unit-diff`, and `impact` are
  available for delivery-unit membership, aggregate unit versions, unit diff,
  and component-to-unit impact evidence.
- HTTP APIs expose `POST /api/model-version/releases/{release_id}/index-units`,
  `GET /api/model-version/unit-diff`, and
  `GET /api/model-version/component-impact`.
- `aios-database model-version index-assets` and `mesh-assets` are available
  for release mesh dependency indexing.
- HTTP APIs expose `POST /api/model-version/releases/{release_id}/index-assets`
  and `GET /api/model-version/releases/{release_id}/mesh-assets`.
- Release mesh asset indexes derive all unique `geo_hash` values from immutable
  `geo_instances.parquet`, record GLB URL/path/bytes/SHA-256/builtin status,
  and emit a derived `mesh_assets_manifest.json`.
- `index-assets --materialize` and
  `POST /api/model-version/releases/{release_id}/index-assets?materialize=true`
  copy GLB files into the immutable release package under
  `model_versions/releases/<release_id>/meshes/lod_<tag>/`.
- Release runtime-scene JSON now prefers release-local pinned mesh URLs when
  that directory exists, while retaining the global `/files/meshes` fallback.
- The current unit membership rule is deliberately conservative: unit nouns
  `BRAN`/`EQUI`/`WALL`/`FLOOR`/`HANG` are self-members; other components attach
  only to an immediate owner with one of those nouns. Owner-chain/tree-index
  membership is still a hardening task.
- `aios-database model-version publish-history` can publish an already generated
  isolated/staged Parquet package as a historical release and rejects the
  current mutable `output/<project>/parquet/<dbnum>` directory.
- The current DB1112 `791`/`897` pair is usable for diagnostic two-pane compare
  but is still not production `complete_visual`: missing mesh repair now
  separates non-renderable source inputs from repairable geometry, and opt-in
  FRADIUS fallback can generate extra degraded GLBs, but those releases must
  remain `quarantined_visual`/`degraded_visual` until exact geometry or signed
  visual-contract evidence exists.

The latest completed implementation slice is `model-version
prepare-history-replay`. It generates an isolated replay DbOption TOML and a
JSON command plan for running historical `incremental-sesno --generate-model`
in a separate process, then publishing the resulting replay Parquet package
with `publish-history`. This slice exists because the current generation stack
still re-reads `DB_OPTION_FILE` and initializes global `aios_core` option
state, so an in-process override is not production-safe yet.

`prepare-history-replay` is complete when:

- The CLI writes a replay config whose `surreal_ns` differs from the base
  config and whose `output_root` points to a release-specific replay work dir.
- The replay config constrains generation with `manual_db_nums=[1112]` and
  `export_parquet_after_gen=true`.
- The command prints clean JSON containing `generate` and `publish` commands.
- Safety checks reject invalid sesno ranges, missing source files, unsafe
  release ids, equal current/replay namespaces, current output roots, and
  config overwrite attempts unless `--force` is explicit.
- CLI validation proves the generated config and JSON paths for DB `1112`
  without using `cargo test`.

## Completed Implementation Slice: Historical Release Safety Gates

This slice turns the latest Oracle findings into production guards:

- `publish-history` must reject empty/zero-row model packages by default so a
  historical patch replay cannot be published as a full 3D release.
- The rejection must be explicit and actionable: the user should know the
  package needs a baseline hydrate/restore step before publishing.
- A deliberate override may exist only for diagnostic/patch-only workflows and
  must be visible in JSON safety checks and release metadata.
- `prepare-history-replay` must emit process-safe argv arrays in addition to
  human-readable command strings, so a future job runner does not depend on
  shell quoting for Windows paths.
- Validation must use `aios-database ... --json`; no `cargo test`.

Status:

- Implemented and validated on 2026-06-19.
- `publish-history` now rejects zero-row visual packages before DuckLake
  registration.
- `prepare-history-replay --json` now emits both command strings and argv
  arrays.

Completion criteria for this slice:

- DB `1112` empty-namespace replay output at
  `target\codex-history-replay-plan\replay_output\AvevaMarineSample\parquet\1112`
  fails `publish-history`.
- The failure happens before DuckLake release registration and creates no
  release metadata/package files.
- The existing fixture/real non-empty DB `1112` package can still be published
  through `publish-history`.
- `prepare-history-replay --json` includes both command strings and argv arrays.
- Build/check commands pass for `aios-database` with `model-version-ducklake`.

## Completed Implementation Slice: Read Path No-Mutation

This slice makes version query APIs production-safe:

- Read/query paths must not auto-index or otherwise mutate DuckLake.
- Missing component/unit/asset indexes must return a clear dependency error
  that tells the caller which explicit CLI/API indexing step to run.
- Explicit mutating commands remain allowed:
  `model-version register`, `index`, `index-units`, `index-assets`,
  `publish-history`, and matching POST endpoints.
- CLI read commands such as `diff`, `unit-diff`, `impact`, `mesh-assets`, and
  `release-scene` should fail fast if required indexes are missing.
- HTTP GET endpoints should map missing-index dependency errors to an
  actionable error response, not silently build indexes.

Status: completed on 2026-06-19 for component and unit read paths. Evidence is
recorded in `progress.md` under "Read Path No-Mutation - 2026-06-19".

Completion criteria for this slice:

- Source inspection confirms no read-path call to an `ensure_*_indexed`
  function remains.
- A controlled temporary DuckLake catalog with releases registered but missing
  component indexes makes `model-version diff --json` fail with an actionable
  missing-index error.
- After explicit `model-version index` for both releases, the same diff command
  succeeds.
- Equivalent HTTP GET validation proves the web API does not auto-index.
- Build/check commands pass for `aios-database` and `web_server` with
  `model-version-ducklake`.

## Completed Implementation Slice: Historical Replay Baseline Gate

This slice makes the historical replay boundary explicit before publish:

- Add a read-only CLI validation command for staged historical replay packages.
- The command must inspect the replay Parquet package and report whether it is
  a complete visual release candidate or only a patch-only/empty-baseline
  artifact.
- By default it must fail with an actionable error when `instances` or
  `geo_instances` are zero, when the replay Parquet path is the current mutable
  output path, or when required package validation fails.
- The command must produce JSON evidence suitable for automation:
  row counts, path safety checks, replay classification, recommended action,
  and optional scene-tree evidence.
- It must not write DuckLake, publish releases, generate models, or mutate
  SurrealDB.
- Scene tree evidence should be visible but not a default hard gate yet, because
  the current DB `1112` visual package is non-empty and loadable while no
  `1112.tree` is present in the sampled current scene_tree output.

Status: completed on 2026-06-19. Evidence is recorded in `progress.md` under
"Historical Replay Baseline Gate - 2026-06-19".

Completion criteria for this slice:

- `model-version validate-history-replay --json` rejects the DB `1112`
  empty-namespace replay package at
  `target\codex-history-replay-plan\replay_output\AvevaMarineSample\parquet\1112`
  with `classification=patch_only_empty_baseline`.
- The same command succeeds for the non-empty DB `1112` package or fixture and
  reports `classification=complete_visual_release_candidate`.
- `publish-history` uses the same baseline validation helper so the publish
  guard and validation command cannot drift.
- Build/check commands pass for `aios-database` with
  `model-version-ducklake`; no `cargo test`.

## Active Implementation Slice: DB1112 Baseline Hydrate Discovery

This slice turns the remaining historical-release blocker into an executable
decision:

- Confirm whether the existing `init-project`, full-sync, and
  `incremental-sesno --generate-model` paths can build a complete isolated
  baseline for DB `1112` before applying a session range.
- Verify whether the full-sync path really writes PE/ATT baseline data into the
  target SurrealDB namespace, or whether current code forces a tree-only/meta
  output.
- Decide the production boundary for historical reconstruction:
  baseline package restore, baseline namespace hydrate, or a read-only
  history-state provider.
- Keep the validated rule from the previous slice: range-only replay into an
  empty namespace is patch-only and cannot be published as a full visual
  release.
- Use Oracle MCP as a second-opinion architecture review, then fold its
  findings back into this plan before implementation.

Completion criteria for this slice:

- Source inspection and at least one CLI/JSON validation establish whether the
  current code can create a non-empty isolated DB `1112` baseline package.
- If the current code cannot do it safely, the required minimal implementation
  is documented before edits begin.
- The architecture docs record the chosen baseline strategy and the DuckLake
  boundary for model data versions.
- Any implemented command has complete path/namespace safety checks and a
  JSON-readable validation result.

Status: completed as a discovery and safety-planning slice on 2026-06-19.

Outcome:

- Oracle MCP session `e3d-model-version-architectu-3` confirmed the architecture
  boundary: SurrealDB/current generator for model creation, immutable
  Parquet/GLB release packages for delivery, and DuckLake for release
  catalog/index/diff/audit.
- Source inspection confirmed that existing full-sync can hydrate only the
  source DB file's current visible state. It cannot reconstruct DB `1112`
  sesno `896` from a newer file by itself.
- `model-version prepare-history-replay --json` now emits baseline and replay
  config paths, baseline release id, baseline dbnums, five command-plan stages,
  argv arrays, and explicit baseline safety flags:
  `baseline_parse_uses_current_file_state=true`,
  `baseline_target_sesno_reconstruction_supported=false`, and
  `baseline_source_must_already_match_from_sesno=true`.
- CLI validation for DB `1112` `896 -> 897` generated the expected isolated
  plan and warning without running an unsafe fake historical baseline publish.

## Next Production Slice

The next slice should implement actual baseline hydrate/restore for DB `1112`,
rerun the generated historical replay against that baseline, and publish the
resulting non-empty replay package as the second real session-derived release.
The current baseline gate and command-plan generator prove the empty-namespace
`896 -> 897` artifact is not publishable and that current full-sync is not a
target-sesno restore; they do not create the missing historical baseline state.

## Active Implementation Slice: Target-Sesno Baseline Hydrate

This slice turns the documented baseline gap into a production-grade mechanism
or a sharply proven boundary:

- Inspect pdms-io and local parser wrappers for an existing ability to read a
  complete database state at a specified historical `sesno`.
- If such an API exists, integrate it as a dedicated baseline hydrate command
  that writes PE/ATT/tree/model prerequisites into an isolated replay namespace
  without touching current state.
- If such an API does not exist, implement the smallest safe adapter boundary:
  a CLI-visible `hydrate-history-baseline` or equivalent planner that rejects
  unsupported target-sesno hydrate by default, accepts only verified physical
  source snapshots/restored baseline packages, and preserves machine-readable
  evidence for automation.
- Keep `prepare-history-replay` honest: it may generate baseline/replay
  configs, but it must not imply current-file full-sync is a historical restore.
- Use DB `1112` `896 -> 897` as the validation case, but treat a zero-row
  replay package as a negative result, not a success.

Success criteria for this slice:

- The codebase has an explicit baseline hydrate/restore entrypoint or an
  explicit validated unsupported-state contract; the behavior is no longer
  implicit inside `prepare-history-replay`.
- CLI JSON exposes authoritative evidence: requested baseline sesno, source
  type, target namespace/output root, whether the baseline is a true
  target-sesno reconstruction, and what validation was performed.
- The DB `1112` validation path either produces a non-empty baseline visual
  package from a proven baseline source, or fails with an actionable JSON
  reason that prevents fake historical publication.
- Documentation and progress records are updated after each meaningful step.
- No `cargo test`; validation uses `aios-database` CLI + JSON and, later, HTTP
  for viewer/API behavior.

## Current Continuation: Source Observation Contract

Status: implemented as a production preflight contract on 2026-06-20.

- `model-version observe-source` now creates a read-only source observation
  manifest for a DB file before parse/generation.
- The command records DB identity, latest/resolved sesno, SHA-256, file size,
  quiet-window stability, manifest path/hash, and recommended action.
- It supports explicit source DB paths and dbnum resolution through
  `db_index.sqlite` when `sqlite-index` is available.
- It rejects wrong dbnum, missing source files, and accidental manifest
  overwrite; `--require-stable` turns unstable evidence into a non-zero gate.
- DB1112 validation against
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001` produced
  `resolved_sesno=897` and stable SHA-256
  `70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2`.
- `incremental-sesno` now accepts `--source-observation-manifest` plus an
  optional manifest hash, validates it before parsing, and verifies the source
  DB hash again after parse/save/generation.
- DB1112 `896 -> 897` guarded incremental parse/save completed with
  `session_count=1`, `element_count=169`, `pe_rows=169`, `att_rows=169`, and
  `source_hash_unchanged=true`.

Next requirement: have `watch-incremental` create and pass a per-update source
observation manifest automatically, instead of requiring the operator to supply
one manually.

Current evidence:

- `model-version inspect-history-baseline` is implemented as a read-only
  target-sesno inspection contract.
- DB `1112` sesno `896` resolves exactly from
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`, but the
  command reports `visible_refno_count=5`, `index_error_count=1`, and
  `full_state_enumeration_supported=false`.
- This is a validated unsupported-state result for historical baseline hydrate,
  not a visual release candidate. The next valid paths are a physical baseline
  source snapshot, restoring a previously published baseline package/namespace,
  or adding a proven pdms-io full-state hydrate provider.
- `model-version prepare-physical-baseline-snapshot` is implemented as the
  first safe physical-source path. It creates an isolated project snapshot,
  replaces the active target DB file with a historical physical DB file, writes
  an isolated DbOption TOML, and never mutates the original AVEVA project.
- DB `1112` physical baseline evidence:
  - Source candidate:
    `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy`
    has header dbnum `1112` and latest sesno `791`.
  - Snapshot command created
    `target\codex-physical-baseline\ams1112-791\project_path\AvevaMarineSample\ams000\ams1112_0001`
    plus
    `target\codex-physical-baseline\ams1112-791\DbOption-physical-baseline.toml`.
  - Snapshot replacement resolves exact sesno `791`.
  - Original active
    `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`
    still resolves latest sesno `897`.
  - Generated config uses isolated `project_path`, `output_root`, and
    `surreal_ns=codex_baseline_ams1112_791`, with `total_sync=true`,
    `save_db=true`, `gen_model=false`, and dependency-aware `manual_db_nums`.

## Active Implementation Slice: Physical Baseline Release Hardening

This slice turns the DB `1112` physical baseline from "non-empty export" into a
publishable visual release candidate.

Current verified evidence:

- Physical baseline source:
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001 copy`.
- Physical baseline session: latest sesno `791`.
- Isolated namespace: `codex_baseline_ams1112_791`.
- Fixed generation writer table counts:
  - `pe_transform=149330`
  - `inst_relate=31044`
  - `inst_info=30632`
  - `inst_geo=1894`
  - `inst_relate_aabb=30484`
- Former failing relation now resolves to the current CATA-hash inst info:
  `inst_relate:[17496,254370,0] -> inst_info:14658783752023738325`.
- Explicit DB `1112` Parquet export succeeded under
  `target\codex-physical-baseline\ams1112-791\validation-export-fixed\1112`:
  - `instances=47698`
  - `geo_instances=31292`
  - `transforms=30495`
  - `aabb=28372`
  - `tubings=56`
  - `ptsets=6999`

Hard success criteria for this slice:

- Mesh completeness is resolved or explicitly classified. A publishable visual
  release must not silently accept unresolved `missing_geo_hashes`.
- If missing mesh hashes are real generation misses, generation/export must be
  fixed and rerun until the manifest is complete or the remaining hashes have a
  documented non-visual/builtin classification.
- `publish-history` or release asset indexing must refuse/stage incomplete mesh
  assets, not expose them as a ready comparison release.
- The physical baseline package can be registered/published only after package
  validation and mesh asset validation pass.
- A second release is generated from a later physical/latest state or a proven
  incremental replay, then both releases are compared through CLI/API and the
  two-pane viewer.
- Validation must use `aios-database` CLI + JSON and web_server HTTP/browser
  checks; `cargo test` remains prohibited.

Immediate next work:

1. Diagnose the 24 missing DB `1112` mesh hashes reported by
   `missing_mesh_report_1112.json`.
2. Determine whether the hashes are absent from the mesh directory, generated
   under a different LOD/name, skipped by mesh persistence, or invalid rows that
   should be excluded/classified.
3. Fix the smallest production-safe boundary and rerun export validation.
4. Update this Goal/progress file after each meaningful verification step.

Latest architecture/validation update:

- The consolidated architecture and development plan is now documented in
  `docs/plans/2026-06-20-e3d-model-version-mesh-baseline-architecture-dev-plan.md`.
- `validate-history-replay --json` classifies the DB1112 791 package as
  `missing_mesh_assets`:
  - `instances=47698`
  - `geo_instances=31292`
  - `missing_mesh_geo_hashes=24`
  - `missing_mesh_owner_refnos=42`
  - `mesh_assets_complete=false`
- `publish-history` refuses the package before DuckLake registration. The
  negative release id
  `codex-ams1112-physical-791-missing-mesh-gate` is not present in
  `model-version list --json`.
- Current decision: do not allow this package as a normal visual release.
  Implement mesh repair/classification or explicit degraded release semantics
  before publishing.

Latest implementation update:

- `model-version repair-missing-meshes` was implemented and validated against
  the DB `1112` missing mesh report.
- Of the original 24 missing mesh hashes, 2 regenerated successfully and 22
  remain classified as bad CSG/profile generation failures.
- Parquet export now records raw/render/quarantine mesh validation evidence.
- With explicit quarantine enabled, the DB `1112` physical baseline `791`
  exports a renderable package:
  - `instances=29545`
  - `geo_instances=31252`
  - `render_missing_geo_hashes=0`
  - `raw_missing_geo_hashes=22`
  - `quarantined_geo_hashes=22`
- `validate-history-replay --json` classifies this package as
  `quarantined_visual_release_candidate` and `ready_for_publish=true`.
- The package was published as
  `codex-ams1112-physical-791-quarantine`.
- DuckLake mesh asset index reports:
  - `geo_hash_count=1192`
  - `present_count=1192`
  - `missing_count=0`
- Same-release diff for
  `codex-ams1112-physical-791-quarantine` returns zero added/deleted/changed
  and `unchanged=29545`.
- `inspect-history-baseline` for target sesno `790` and `791` still reports
  `full_state_enumeration_supported=false`; target-sesno hydrate is not solved
  yet.

Remaining before calling the full goal complete:

- Treat `codex-ams1112-physical-791-quarantine` as a quarantined visual
  baseline test release, not as a fully clean production release.
- Execute the generated replay command from that physical baseline for a real
  historical range available from the chosen source pair.
- Reject or mark patch-only any replay package whose manifest contains zero
  model rows; the DB `1112` `896 -> 897` empty-namespace replay persisted
  169 element changes but exported `instances=0`, so it is not valid
  two-pane 3D release evidence.
- Publish the validated non-empty replay Parquet package with `publish-history`.
- Register a second real session-derived release, not only a controlled fixture.
- Implement or fix a proven target-sesno full-state hydrate provider, or acquire
  a second physical baseline source that can generate a real second release.
- Harden `publish-history` into an atomic/idempotent publish state machine before
  treating it as production release creation.
- Ensure read APIs do not auto-index or mutate DuckLake; missing indexes should
  return dependency errors with explicit remediation.
- Integrate immutable release package loading into the richer production
  plant3d-web viewer, if that viewer remains the final user-facing surface.
- Replace the conservative direct-owner/self-unit membership rule with a
  production owner-chain/tree-index resolver and lineage evidence for nested
  components such as `VALV -> EQUI -> BRAN`.

## Latest Oracle MCP And Second Release Planning Update

Oracle MCP sessions re-read on 2026-06-20:

- `e3d-model-version-architectu-3`
- `e3d-ducklake-version-plan`

Decision confirmed:

- Use DuckLake in this version for catalog, manifests, component/unit indexes,
  diff/impact, and audit evidence.
- Do not use DuckLake as the model-generation writer or GLB/Parquet body store.
- Define the model version as `release_id + immutable Parquet/GLB package +
  validation evidence + DuckLake indexes`.
- A `sesno` range is parse/source evidence, not a complete model version unless
  a full baseline state exists before applying the increment.

Current second-release evidence:

- A DB1112 physical 897 snapshot was created under
  `target\codex-physical-baseline\ams1112-897` with isolated namespace
  `codex_baseline_ams1112_897`.
- Full parse/generation of that 897 snapshot did not complete within the
  practical validation window and was terminated. This is now a production
  hardening requirement: bounded progress, timeout, checkpoint, and resume
  diagnostics are needed for full snapshot generation.
- The existing current DB1112 package was registered as
  `codex-ams1112-current-897-partial` in the same DuckLake catalog only as a
  chain-validation release:
  - `instances=106`
  - `geo_instances=163`
  - `mesh_asset_index.missing_count=0`
- Diff from `codex-ams1112-physical-791-quarantine` to
  `codex-ams1112-current-897-partial` returns:
  - `added=106`
  - `deleted=29545`
  - `changed=0`
  - `unchanged=0`

This proves two-release catalog/index/diff mechanics, but it is not a valid
incremental model-change proof because the 897 release is partial. The next
production-valid route is either a true target-sesno hydrate/increment replay
or a completed 897 full physical snapshot release.

## Current Web Validation Status - 2026-06-20

Validated through `web_server` on port `3910` with the shared DB1112 DuckLake
catalog:

- release list, runtime-scene, release-local GLB URL, mesh-assets, and diff
  HTTP APIs all respond successfully;
- compare page loads two release panes and the diff table;
- browser diagnostics show:
  - left pane loaded `2090/2090` geometries, failed `0`;
  - right pane loaded `163/163` geometries, failed `0`;
  - xeokit scene/model state is non-empty for both panes.

After the continuation viewer slice, the compare page now renders visible
high-contrast AABB proxy geometry derived from each release runtime-scene while
still loading release-local GLBs with zero failures. Treat this as a valid
two-pane spatial comparison proof for the current backend release packages, but
not yet as final production mesh-viewer sign-off. The remaining UI hardening is
actual GLB material/edge rendering, richer selection/highlight, camera sync,
and production plant3d-web or XKT integration.

## Continuation Focus - 2026-06-20

Current priority has advanced from pure data loading to visible 3D comparison.
The proxy viewer slice provides browser evidence that both panes render
distinguishable geometry. Next priority is to graduate from proxy geometry to
production-grade mesh visualization and to continue the larger baseline
hydrate/true second-release implementation path.

## Latest Oracle MCP Architecture Plan - 2026-06-20

New Oracle MCP browser session:

- `e3d-model-version-architectu-20260620`
- transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-model-version-architectu-20260620\artifacts\transcript.md`

The refined Chinese architecture and development plan is now documented in:

```text
docs/plans/2026-06-20-e3d-incremental-model-version-ducklake-oracle-plan.md
```

Current decision:

- DuckLake remains in scope for catalog, manifests, component/unit indexes,
  diff/impact, and audit evidence.
- DuckLake must not become the model-generation writer or GLB/Parquet body
  store.
- `release_id` is the user-facing model version. `sesno` is source evidence
  and does not become a visual model version until baseline, generation,
  package validation, asset materialization, indexing, and publish all pass.
- The next production implementation priority is release status/publish
  atomicity, DuckLake read/write split, explicit baseline hydrate/restore, and
  then a true second DB1112 release.

## Latest Implementation Update - 2026-06-20

Release status machine first slice is implemented and validated:

- `ModelReleaseStatus` is now part of `ModelReleaseRecord`.
- DuckLake `model_releases` has a backward-compatible `release_status`
  migration and `model_release_status_events`.
- Existing rows are backfilled as `published`.
- Normal release list returns only `published` releases.
- Read paths reject non-published releases for component diff, unit diff,
  component impact, runtime-scene, and mesh-asset reads.
- `publish-history` now stages first, records validation/materialization/index
  transitions, promotes to `published` after success, and marks `failed` on
  post-registration errors.

Validation evidence:

- `cargo build --bin aios-database --features model-version-ducklake` passed.
- Temporary CLI publish-history status smoke passed with
  `release_status=published` and `component_count=106`.
- `cargo build --bin web_server --features model-version-ducklake` passed using
  `target\codex-web-validate-build`.
- HTTP `GET /api/model-version/releases?project=AvevaMarineSample` on port
  `3910` returns two releases, both `published`.
- HTTP diff between the two DB1112 validation releases still returns
  `added=106`, `deleted=29545`, `changed=0`, `emitted=1`.

Remaining production blockers:

- Split DuckLake writer and readonly open paths.
- Make asset completeness and release-local mesh retention strict status gates.
- Implement true baseline hydrate/restore or complete a second full DB1112
  source release.
- Replace the current partial 897 fixture with a real second session-derived
  or full-snapshot release before claiming production completion.

## Latest Implementation Update - 2026-06-20 Readonly/Writer Split

DuckLake read/write separation is implemented and validated:

- `ModelVersionDuckLakeStore::open_writer()` is now the mutation path:
  it creates required directories, acquires the metadata writer lock, attaches
  DuckLake read-write, and runs schema creation/migration.
- `ModelVersionDuckLakeStore::open_readonly()` is now the GET/read path:
  it requires an existing catalog/data directory, does not acquire the writer
  lock, attaches DuckLake with `READ_ONLY`, and validates that the read schema
  is present.
- CLI and web read paths now use readonly open for release list, component
  diff, unit diff, component impact, mesh-assets reads, and runtime-scene reads.
- Register/status/index/publish commands continue to use writer open.
- Readonly schema migration errors now point users to writer commands such as
  `register-release`, `publish-history`, or `index-release`, not readonly
  `list`.

Validation evidence:

- `cargo fmt --check` passed.
- `cargo build --bin aios-database --features model-version-ducklake` passed
  with only existing upstream warnings.
- `cargo build --bin web_server --features model-version-ducklake` passed with
  only existing upstream warnings.
- CLI readonly list against the shared AvevaMarineSample DuckLake catalog:
  `release_count=2`, `statuses=published,published`.
- CLI readonly diff:
  `added=106`, `deleted=29545`, `changed=0`, `unchanged=0`, `emitted=1`.
- Manual lock proof:
  a hand-created `metadata.ducklake.lock` did not block readonly CLI list.
- Isolated web validation server is running on `127.0.0.1:3910`, PID `70296`.
- HTTP release list and diff match the CLI results.
- HTTP runtime-scene responses return release-local `mesh_base_url` and
  `mesh_url_pattern` for both validation releases.

Next blockers remain:

- strict asset completeness/release-local retention as publish gates;
- no current/global mesh fallback for published historical runtime scenes;
- source manifest, baseline state manifest, generation job id, and asset
  manifest hash in release metadata;
- a true second DB1112 release, replacing the current partial 897 fixture as
  production evidence.

## Latest Implementation Update - 2026-06-20 Release-Local Asset Gate

Release-local asset completeness is now enforced for published runtime scenes
and `publish-history`:

- Published `runtime-scene` now requires an existing mesh asset index for visual
  releases.
- `runtime-scene` rejects releases with `missing_count > 0`, stale asset-index
  row counts, missing release-local mesh directory, or non-builtin assets that
  were indexed outside release-local `meshes/lod_<lod>/`.
- Web runtime-scene no longer falls back to `/files/meshes/lod_<lod>`.
- `publish-history` rejects visual packages when `materialize_assets=false`.
- `publish-history` rejects `missing_count > 0` after materialization before
  promoting a release to `published`.
- Direct low-level `model-version register` now creates `staged` releases by
  default so an unmaterialized visual package cannot bypass publish gates and
  appear in the default published release list.

Validation evidence:

- `cargo fmt --check` passed.
- `cargo build --bin aios-database --features model-version-ducklake` passed
  with only existing upstream warnings.
- `cargo build --bin web_server --features model-version-ducklake` passed with
  only existing upstream warnings.
- Web validation server is running on `127.0.0.1:3910`, PID `26416`.
- Existing published runtime-scene reads still succeed for both validation
  releases and return release-local `mesh_base_url`.
- Temporarily hiding the 897 partial release-local mesh directory returns HTTP
  `424 Failed Dependency` and the directory was restored.
- CLI negative publish-history without `--materialize-assets` exits with code
  `1` and creates no metadata catalog.
- CLI positive publish-history with `--materialize-assets` into a temporary
  catalog publishes successfully with:
  `component_count=29545`, `mesh_present=1192`, `mesh_missing=0`.
- Direct low-level `model-version register --json` into a temporary catalog
  returned `release_status=staged` and `default_list_count=0`.
- HTTP release list and diff regressions still match prior results.
- Final HTTP runtime-scene regression for
  `codex-ams1112-current-897-partial` returns `component_count=2`,
  `geometry_count=2`, and a release-local `mesh_base_url`.

Next blockers at that point:

- source manifest, baseline state manifest, generation job id, and asset
  manifest hash in release metadata;
- true DB1112 second release from baseline hydrate/restore or a full second
  physical snapshot;
- rerun browser two-pane validation after the next UI-facing slice.

## Latest Implementation Update - 2026-06-20 Release Provenance Fields

Release record provenance fields are now implemented:

- `ModelReleaseRecord` exposes `source_manifest_path`,
  `source_manifest_hash`, `baseline_state_manifest_path`,
  `baseline_state_manifest_hash`, `generation_job_id`,
  `asset_manifest_path`, and `asset_manifest_hash`.
- DuckLake `model_releases` has compatible writer migrations for these fields.
- Readonly APIs require the migrated schema before serving release reads.
- Idempotent register backfills missing provenance fields for existing releases
  without overwriting existing values.
- Asset indexing updates `asset_manifest_path` and `asset_manifest_hash` on the
  release record.
- Baseline-state manifest hash mismatches fail before catalog creation.

Validation evidence:

- `cargo build --bin aios-database --features model-version-ducklake` passed
  with only existing upstream warnings.
- `cargo build --bin web_server --features model-version-ducklake` passed with
  only existing upstream warnings.
- Temporary register smoke produced a staged release with source manifest hash,
  baseline state manifest hash, and `generation_job_id`.
- Negative register with a wrong baseline-state manifest hash returned exit
  code `1` and created no catalog.
- Negative register with a missing baseline-state manifest path returned exit
  code `1` and created no catalog.
- Shared AvevaMarineSample validation catalog was migrated/backfilled; both
  published DB1112 validation releases now expose source manifest hash,
  asset manifest hash, and generation job id.
- Web validation server is running on `127.0.0.1:3910`, PID `46376`.
- HTTP release list reports:
  `release_count=2`, `source_manifest_hashes_present=2`,
  `asset_manifest_hashes_present=2`.
- HTTP diff still returns `added=106`, `deleted=29545`, `changed=0`,
  `unchanged=0`, `emitted=1`.
- HTTP runtime-scene still returns release-local mesh URL.

Next blockers remain:

- true DB1112 second release from baseline hydrate/restore or a full second
  physical snapshot;
- real baseline state manifest attached to the true release pair;
- browser two-pane validation after the true second release is available.

## Latest Implementation Update - 2026-06-20 Physical Baseline State Manifest

Physical baseline snapshots now produce hashable baseline-state evidence:

- `prepare-physical-baseline-snapshot` writes
  `baseline_state_manifest.json` under the snapshot root.
- The manifest records source DB hash, replacement DB hash, snapshot/config
  paths, output root, Surreal namespace, copy/link counts, and safety checks.
- The CLI response returns `baseline_state_manifest_path` and
  `baseline_state_manifest_hash`.
- Non-JSON CLI output prints the manifest path/hash for operator handoff.

Validation evidence:

- `cargo build --bin aios-database --features model-version-ducklake` passed
  with only existing upstream warnings.
- `cargo build --bin web_server --features model-version-ducklake` passed with
  only existing upstream warnings.
- A real physical baseline snapshot smoke against
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001` wrote
  `baseline_state_manifest.json` with hash
  `29372c887b997481fb27ad77391d73cc40fc86336d921c8dafd7525daf4eec68`.
- The manifest reports `replacement_db_sha256_matches=True`,
  `file_count=448`, and `original_project_not_modified=True`.
- Temporary `publish-history --materialize-assets` using that baseline
  manifest in metadata published in an isolated catalog with
  `baseline_hash_matches=True`, `mesh_missing=0`, and
  `component_count=29545`.
- HTTP regression on `127.0.0.1:3910`, PID `67844`, still returns two shared
  published releases, release-local runtime scene, and the established diff
  counts.

Next blockers remain:

- produce or obtain a true second DB1112 full release;
- attach a real baseline state manifest to that true release pair;
- rerun browser two-pane validation on the true pair.

## Latest Implementation Update - 2026-06-20 DB1112 897 Candidate

Current architectural decision:

- DuckLake remains the model-version catalog/index/diff/audit layer.
- DuckLake does not own parser writes, generation workspace, or GLB/Parquet
  bodies.
- The model data version is `release_id` plus source/baseline/generation/asset
  manifests and validation/index evidence.

New source audit:

```text
D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams1112_0001
latest_sesno=767

D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
latest_sesno=897
```

Code update:

- Physical baseline manifests now include `source_db_latest_sesno`.
- The snapshot CLI response also includes and prints `source_db_latest_sesno`.
- The snapshot CLI response now also includes `generate_full_model`, an
  explicit isolated full-generation/export command:
  `aios-database -c <snapshot-config> --regen-model --dbnum 1112 --export-parquet-after-gen`.

Validation:

- `cargo fmt --check` passed.
- `cargo build --bin aios-database --features model-version-ducklake` passed.
- Created isolated 897 candidate snapshot:

```text
snapshot_id=codex-ams1112-897-candidate-20260620-053630
baseline_state_manifest_hash=8766d612b70e6aa3e09200b54fb9daa9b7a10545a811d85324ac589fd03d0082
source_db_latest_sesno=897
source_db_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
replacement_db_sha256=70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2
file_count=448
hardlinked_count=448
copied_count=0
original_project_not_modified=True
```

- Command-chain validation snapshot:

```text
snapshot_id=codex-ams1112-897-command-check-20260620-054350
source_db_latest_sesno=897
generate_full_model=aios-database -c target\codex-physical-baseline\ams1112-897-command-check-20260620-054350\DbOption-physical-897 --regen-model --dbnum 1112 --export-parquet-after-gen
generate_has_regen_model=True
generate_has_export=True
file_count=448
hardlinked_count=448
original_project_not_modified=True
```

Next execution path:

1. Parse/save the isolated 897 snapshot using `commands.parse`.
2. Run full model generation/export for DB1112 using
   `commands.generate_full_model`.
3. Validate mesh assets and scene/package evidence.
4. Publish a full 897 release only after all gates pass.
5. Compare the full baseline release and full 897 release through CLI, HTTP,
   and browser two-pane viewer.

Latest parse attempt:

- Ran `commands.parse` for sanitized snapshot
  `codex-ams1112-897-parse-20260620_054746`.
- The isolated config connected to namespace
  `codex_baseline_ams1112_897_parse_20260620_054746`.
- DB1112 opened and reported `422107` refnos.
- The debug-build parse did not complete within the interactive validation
  window: it was still CPU-active after about 46 minutes and had no progress
  output beyond the initial DB read lines.
- The validation process was stopped cleanly by PID; no `aios-database`
  process was left running.
- Source DB hash remained unchanged:
  `70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2`.

Updated next requirement:

- Before claiming 897 full parse/generation is production-ready, add or use a
  bounded runner with progress visibility for full physical parse/generation
  and rerun the parse to completion.

Latest observability update:

- `src/versioned_db/database.rs` now emits `[parse-progress]` stdout
  heartbeat lines on the full parse path.
- Verified against a real 897 isolated snapshot:

```text
snapshot_id=codex-ams1112-897-heartbeat-20260620_064024
DB1112 heartbeat=[parse-progress] db_basic_done ... refnos=422107 chunks=5
progress_line_count=14
source_db_sha256_after_stop=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
```

Remaining requirement:

- Heartbeat is only stdout observability. Production acceptance still needs a
  bounded runner with persisted status, cancellation/timeout semantics, and a
  rerun of `commands.parse` to normal exit.

Latest persisted progress update:

- Parse heartbeat is now also written to the existing task metrics JSON when
  `AIOS_TASK_METRICS_PATH` is set.
- Implemented fields include current stage, project/file/dbnum/db_type,
  save_db, total refnos/chunks, completed chunks, last chunk, parsed attrs,
  elapsed ms, and `updated_at`.
- Real 897 smoke validation:

```text
snapshot_id=codex-ams1112-897-metrics-20260620_065144
metrics_path=target\codex-physical-baseline\ams1112-897-metrics-20260620_065144\parse-metrics.json
observed_stage=db_basic_done
observed_dbnum=1112
observed_refnos_total=422107
observed_chunks_total=5
aios_database_processes_after_stop=0
source_db_sha256_after_stop=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
HTTP release list: success=True release_count=2 statuses=published,published
```

Updated remaining requirement:

- The next required production slice is a bounded runner around parse and full
  generation, not more ad hoc foreground CLI execution. It must capture normal
  exit, non-zero exit, timeout, cancellation reason, and final metrics before
  `commands.generate_full_model` is attempted.

Oracle follow-up:

- Successful session:
  `C:\Users\dpc\.oracle\sessions\e3d-version-inline-review\artifacts\transcript.md`.
- Oracle confirmed the main architecture:
  - DuckLake is the controlled catalog/index/diff/audit layer.
  - Immutable release packages and manifests are the model-data version truth.
  - Existing parse/generation paths should not be rewritten to DuckLake as the
    primary writer.
- Oracle added production gates that are now part of the Goal:
  - split release lifecycle from release quality;
  - add single-writer DuckLake publish/index queue;
  - keep runtime-scene GET read-only with no repair/index/fallback;
  - add bounded runner state and generation metrics;
  - re-hash hardlinked physical snapshot files before parse/generate/publish;
  - treat the existing 791/897 pair as smoke evidence until a full 897 physical
    release exists.

Latest lifecycle/quality implementation:

- Implemented the first Oracle correction by splitting release publication
  lifecycle from visual/data quality.
- DuckLake now stores:
  - `release_lifecycle`: `staged`, `validating`, `assets_materialized`,
    `indexed`, `published`, `failed`.
  - `release_quality`: `complete_visual`, `quarantined_visual`,
    `degraded_visual`, `patch_only`, `non_visual`.
- Legacy `release_status` is retained as a compatibility field only.
- Existing catalog rows were migrated/backfilled:

```text
codex-ams1112-current-897-partial
  lifecycle=published
  quality=degraded_visual
  component_count=106

codex-ams1112-physical-791-quarantine
  lifecycle=published
  quality=quarantined_visual
  component_count=29545
```

- HTTP quality filtering was validated on the isolated `web_server` running on
  `127.0.0.1:3910`:

```text
default published list: 2 releases
quality=degraded_visual: 1 release
quality=quarantined_visual: 1 release
complete_visual_only=true: 0 releases
invalid quality: HTTP 400
```

Updated interpretation:

- `published` now means the release is visible/registered, not necessarily a
  complete production visual release.
- `complete_visual` is the quality gate that should be used for final 3D model
  comparison acceptance.
- The current two releases remain useful catalog/API/UI smoke fixtures, but
  they do not satisfy the final DB1112 791-vs-897 production evidence gate.

Latest bounded runner implementation:

- Implemented CLI-first durable supervision:
  - `model-version run-command`
  - `model-version run-status`
  - `model-version cancel-run`
- The runner records command argv, executable, cwd, env keys, pid, stdout,
  stderr, metrics path, timeout, timestamps, exit code, cancellation reason,
  source DB hash before/after, and hash unchanged status.
- Validation proved:

```text
success path: run-command --help -> status=succeeded exit_code=0
failure path: default catalog list -> status=failed exit_code=1 with stderr captured
timeout path: sleep 10 with timeout 1 -> status=timed_out and child PID gone
cancel path: running sleep 30 -> cancel-run -> status=cancelled and child PID gone
hash/metrics path: source_db_hash_unchanged=True and metrics snapshot populated
```

Updated next requirement:

- The next real DB1112 897 physical parse attempt must be launched through this
  bounded runner with `AIOS_TASK_METRICS_PATH`, source DB hash before/after, and
  a timeout/cancel policy.
- `commands.generate_full_model` remains gated on a normal parse exit. The
  first generation metrics slice now exists, but full generation/export success
  still needs to be proven against DB1112 897 before production sign-off.

Latest command-plan compatibility and DB1112 897 runner smoke:

- Prepared command plans emit argv arrays whose first element may be
  `aios-database`. The bounded runner now preserves that original `argv`,
  strips the leading executable only for the spawned child command, and records
  `child_argv` plus `argv_included_executable` in `run.json`.
- The new `child_argv` and `argv_included_executable` fields are serde-defaulted
  so older runner status files remain readable by `run-status` and
  `cancel-run`.
- A real DB1112 897 snapshot was launched through the runner with:

```text
snapshot_id=codex-ams1112-897-runner-smoke-20260620_0810
source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001
source_db_sha256=70F18C70116F392EAE533B75FB8F4043D031A5F049448531CC1DFC43FAF7D3C2
run_id=runner-897-parse-smoke-20260620_0810
timeout_secs=30
```

- The smoke intentionally timed out at 30 seconds, but proved the prepared argv
  starts the real parser and reaches the DB1112 expensive stage:

```text
status=timed_out
argv_included_executable=True
child_argv=["-c","target\\codex-physical-baseline\\codex-ams1112-897-runner-smoke-20260620_0810\\DbOption-physical-897"]
metrics_stage=db_basic_done
db1112_refnos=422107
db1112_chunks=5
source_db_hash_unchanged=True
aios_database_processes_after_timeout=0
```

- This still is not a successful 897 full parse. The next acceptance run must
  reuse this runner path with an operator-approved timeout, finish parse
  normally, then run `commands.generate_full_model` under the same supervision.

Latest generation metrics implementation:

- `TaskMetrics.generate.progress` records the currently active generation
  stage, optional detail, elapsed milliseconds, and timestamp.
- `run_generate_model`, `run_regen_model`, and the direct `incremental-sesno
  --generate-model` path now emit progress stages around Surreal connection,
  transform-root collection, target collection, cleanup, and `gen_all_geos_data`.
- The IndexTree generation path now emits inner progress stages for
  initialization, geometry generation, instance write, batch barrier, boolean
  operation, web bundle export, sqlite spatial index, and completion.
- `finish_generate_stage_from_model_store` snapshots current model-store counts
  from SurrealDB into the same metrics JSON when the generation stage reaches a
  terminal point.
- Failure-path validation was run through the bounded runner:

```text
run_id=runner-generate-metrics-fail-20260620_082231
command=aios-database -c db_options/DbOption --regen-model --dbnum 0 --export-parquet-after-gen
status=failed
exit_code=1
metrics_exists=True
metrics_success=False
metrics_stage=collect_transform_refresh_roots
stderr=Error: dbnum=0 下未找到任何 SITE，无法刷新 pe_transform
```

- This validates metrics finalization on an early generation failure. It does
  not replace the required successful 897 full parse, generate, package, publish,
  and two-pane visual comparison run.

Latest Oracle MCP follow-up:

- API mode failed immediately because `OPENAI_API_KEY` is not available in this
  environment.
- A no-attachment browser-mode Oracle session was started with the compressed
  architecture context: `e3d-model-version-ducklake-browser`.
- The browser session completed and reinforced the current architecture:
  - treat file watching as `SourceObservation`, not a parse/publish trigger;
  - start with full-state diff between DB1112 sesno 897 and latest, then add
    native sesno-range delta only after full-parse equivalence is proven;
  - keep user-visible model version as immutable `release_id` plus manifest and
    payload hashes;
  - keep DuckLake as a rebuildable catalog/index/diff/audit layer, not payload
    truth, job coordinator, mutable workspace, or user version id.
- Transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-model-version-ducklake-browser\artifacts\transcript.md`.

Latest HTTP runner/API slice:

- `POST /api/model-version/runs`, `GET /api/model-version/runs/{run_id}`, and
  `POST /api/model-version/runs/{run_id}/cancel` are now wired into
  `web_server`.
- The API only permits an `aios-database` executable and rejects arbitrary
  executables before starting a child process.
- `run_bounded_command` now finalizes a failed `run.json` when process spawn
  fails after initial state creation.
- The non-DuckLake feature-gated store stub now matches the release API surface
  for `open_writer`, `open_readonly`, and `update_release_status`, returning a
  clear feature-required error instead of breaking `web_server --features
  web_server` builds.
- Real HTTP validation was run against a temporary web_server on port 3921:

```text
start=POST /api/model-version/runs
command=aios-database --help
run_id=http-runner-help-20260620-0904
launch_observed=True
argv_included_executable=True
child_argv=["--help"]
status=succeeded
exit_code=0
cancel_after_success.kill_attempted=False
negative_executable_check=powershell.exe rejected
```

Latest Oracle MCP status:

- Attachment-based review `e3d-model-version-ducklake-current` completed.
- Transcript:
  `C:\Users\dpc\.oracle\sessions\e3d-model-version-ducklake-current\artifacts\transcript.md`.
- It confirmed the current architecture and added next-step hardening:
  - convert the generic HTTP `aios-database argv` runner into structured
    model-version pipeline run kinds;
  - sandbox all runner paths (`state_dir`, `cwd`, stdout/stderr/metrics) under
    project-controlled roots;
  - replace single-file source hash evidence with a source observation manifest
    covering dependency DB/catalog/spec/material files;
  - add stage-aware heartbeat/metrics, not only metrics-file mtime;
  - prioritize a successful DB1112 897 full parse/generate/release before
    expanding DuckLake further.

Current unfinished acceptance boundary:

- DB1112 `sesno=897` has only reached a supervised 30-second parse smoke
  (`db_basic_done`, 422107 refnos, source hash unchanged).
- A normal 897 full parse, latest full parse, model generation, release
  packaging, DuckLake indexing, diff, and two-pane visual comparison are still
  required before the overall goal can be marked complete.

## Active Implementation Slice: Structured Pipeline Runner + Source Observation Evidence

The next production hardening step is to keep the existing generic bounded
runner as an internal primitive, but expose model-version operations through
structured pipeline requests. This prevents the HTTP layer from becoming a
generic `aios-database argv` launcher and creates a stable evidence boundary
before DB1112 897/latest long-running parse and generation jobs.

This slice is complete when:

- A source observation manifest type exists and records project, dbnum,
  primary source DB file, file size, modified time, SHA-256, requested sesno,
  resolved sesno when known, quiescence metadata, and dependency file hashes
  when provided.
- The HTTP model-version API has at least one domain-specific pipeline endpoint
  that builds a bounded-run request server-side instead of accepting arbitrary
  command paths from callers.
- Pipeline-generated `state_dir`, stdout/stderr paths, metrics path, and
  source observation manifest path are constrained under a project-controlled
  run directory.
- The new endpoint is validated through a real `web_server` process and HTTP
  `POST`/`GET` calls. The validation must not run cargo tests.
- Progress and decisions are recorded in `progress.md`, including any remaining
  risks before the full DB1112 897 parse/generate/release path.

Status: completed on 2026-06-20 for the first domain-specific pipeline endpoint.

Implemented boundary:

- `ModelSourceObservationManifest` records source DB file evidence, quiescence,
  requested/resolved sesno fields, and optional dependency file hashes.
- `POST /api/model-version/runs/prepare-physical-snapshot` now accepts a
  structured request and builds the bounded-run command server-side.
- Generated run paths are constrained under
  `output/<project>/model_versions/runs`.
- Generated physical snapshot paths are constrained under
  `output/<project>/model_versions/physical_baselines/<snapshot_id>`.
- The endpoint writes source observation evidence before launching the runner,
  but validates the executable allowlist before creating that evidence.
- Real HTTP validation proved both:
  - positive DB1112 source observation and physical snapshot preparation;
  - negative executable rejection without leftover source observation manifest.

Next implementation slice:

- Add structured endpoints for parse baseline, generate full model, validate
  package, publish release, index release, and compare release pair.
- Expand source observation dependency discovery beyond caller-provided files.
- Upgrade metrics heartbeat from file mtime to stage-aware progress with
  sequence/checkpoint/budget evidence.
- Then rerun DB1112 `sesno=897` full parse to normal exit, followed by full
  model generation and release publication.

## Completed Implementation Slice: Structured Parse Baseline Endpoint

Status: completed on 2026-06-20 as the second domain-specific pipeline endpoint.

Implemented boundary:

- `POST /api/model-version/runs/parse-baseline` accepts `project`, `run_id`,
  `snapshot_id`, optional `dbnum`, timeout/cancel settings, and an
  `aios-database` executable.
- The endpoint does not accept arbitrary parse config paths or argv arrays.
  It derives the physical snapshot root from
  `output/<project>/model_versions/physical_baselines/<snapshot_id>`.
- It reads and validates `baseline_state_manifest.json` before launch.
- It verifies project, dbnum when provided, snapshot id, config path, output
  root, and replacement DB file are all under the controlled snapshot root.
- It recomputes the baseline-state manifest SHA-256 and records it in the run
  response/env.
- It re-observes the snapshot replacement DB file before parse and fails if the
  observed hash differs from the baseline-state manifest hash evidence.
- It records the baseline-state manifest as a source observation dependency.
- It launches only:
  `aios-database -c <snapshot DbOption>`.

Validation evidence:

- Real HTTP run against DB1112 snapshot
  `http-prepare-physical-1112-20260620-0937` started the parser, reached
  `stages.parse.progress.stage=db_basic_done` for `dbnum=1112`,
  `refnos_total=422107`, then timed out under the bounded runner as requested.
- The runner recorded `source_db_hash_unchanged=true` and no child process
  remained after timeout.
- Negative executable and missing snapshot requests were rejected without
  creating source observation manifests.

Next implementation slice:

- Add structured `generate-full-model` endpoint that consumes the same
  baseline-state manifest and parse run evidence, writes generation metrics,
  and refuses to run unless the parse prerequisite is explicitly satisfied or
  the caller marks the run as a smoke/diagnostic attempt.

## Completed Implementation Slice: Structured Generate Full Model Endpoint

Status: completed on 2026-06-20 as the third domain-specific pipeline
endpoint.

Implemented boundary:

- `POST /api/model-version/runs/generate-full-model` accepts `project`,
  `run_id`, `snapshot_id`, optional `dbnum`, optional `parse_run_id`,
  timeout/cancel settings, and an `aios-database` executable.
- The endpoint does not accept arbitrary generation argv arrays or config
  paths. It derives the physical snapshot root from
  `output/<project>/model_versions/physical_baselines/<snapshot_id>`.
- It reads and validates `baseline_state_manifest.json` before launch.
- It verifies project, dbnum when provided, snapshot id, config path, output
  root, and replacement DB file are all under the controlled snapshot root.
- It recomputes the baseline-state manifest SHA-256 and records it in the run
  response/env.
- It re-observes the snapshot replacement DB file before generation and fails
  if the observed hash differs from the baseline-state manifest hash evidence.
- Production mode requires `parse_run_id` to reference a bounded runner record
  with `kind=parse_baseline`, `status=succeeded`,
  `source_db_hash_unchanged=true`, matching source DB path, and matching
  before/after source hashes.
- `allow_incomplete_parse=true` is available only as an explicit diagnostic
  escape hatch and requires `diagnostic_reason`.
- It launches only:
  `aios-database -c <snapshot DbOption> --regen-model --dbnum <dbnum> --export-parquet-after-gen`.

Validation evidence:

- Missing `parse_run_id` is rejected with HTTP 400 and no source observation
  manifest is created.
- A timed-out parse run
  `http-parse-baseline-1112-timeout-20260620-0954` is rejected with HTTP 424
  and no source observation manifest is created.
- Diagnostic smoke run
  `http-generate-full-diagnostic-1112-20260620-1019` launched through the
  bounded runner, wrote a source observation manifest, failed quickly in the
  current incomplete parse state, proved `source_db_hash_unchanged=true`, and
  left no child process alive.
- A `powershell.exe` executable request is rejected with HTTP 400 before source
  observation creation.

Next implementation slice:

- Run `parse-baseline` with an operator-approved long timeout until normal
  success for DB1112 897.
- Re-run `generate-full-model` in production mode with the successful parse run
  id.
- Add validation/publish/index endpoints so a successful generated package can
  become an immutable release and then a two-pane comparison candidate.

## Edge Cases To Preserve

- Missing Parquet package directory.
- Missing `manifest.json`.
- Missing required Parquet files.
- Corrupt or unreadable manifest JSON.
- Empty package or zero-row model.
- Registering the same release twice.
- Registering the same release id with different file hashes.
- Parent release id missing or already present.
- DB number mismatch between CLI, manifest, and package path.
- Partial copied release package after interruption.
- DuckLake extension install/load failure.
- Metadata path unwritable.
- Data path unwritable.
- Windows path normalization and spaces in paths.
- Running without `model-version-ducklake` feature.
- Release registered before the component snapshot schema existed.
- Diff requested before a release has been indexed.
- Diffing missing releases.
- Diffing releases from different projects or dbnums.
- Same-release diff should be empty and still return counts.
- Unit aggregate hashes must be stable across releases when member content and
  membership are unchanged; release id must not leak into `membership_hash`.
- Unknown unit noun filters should fail as HTTP 400 / CLI error, not silently
  return misleading empty diffs.
- Unassigned unit memberships must be counted and explainable.
- Component moved between units must appear as old/new membership impact, not
  only as a component content change.
- Future multi-db release packages.
- Future web server startup must not accidentally trigger full generation during API validation.

## Evidence Links

- Oracle follow-up: `oracle_followup_2026-06-19.md`
- Architecture plan: `docs/plans/2026-06-19-model-version-ducklake-architecture-plan.md`
- Production architecture and development plan:
  `docs/plans/2026-06-19-e3d-model-version-production-architecture-dev-plan.md`
- Incremental/backend plan: `docs/plans/2026-06-19-e3d-incremental-site-model-generation.md`

## Goal Continuation Update - 2026-06-20

Current authoritative state:

- The DB1112 897 production-shaped chain has completed a real full
  `parse-baseline` and `generate-full-model` run:
  - `snapshot_id=http-prepare-physical-1112-smallchunk-long-20260620-1113`
  - `parse_run_id=http-parse-baseline-1112-smallchunk-long-20260620-1113`
  - `generate_run_id=http-generate-full-1112-cleanup-heartbeat-20260620-1241`
  - `source_hash_unchanged=true`
- Backend site model generation is no longer blocked by missing SITE data once
  the complete baseline parse has succeeded.
- The generated 897 package is a valid parse/save/generate/export evidence
  package, but not a complete visual release:
  - `classification=missing_mesh_assets`
  - `ready_for_publish=false`
  - `missing_geo_hashes=23`
  - `missing_owner_refnos=208`
- `repair-missing-meshes` was attempted for all 23 hashes and produced
  `generated_hashes=0`, `still_missing_hashes=23`,
  `status=generation_failed_bad`.

Active production slice:

- Reuse or implement the explicit missing-mesh quarantine/export policy for the
  DB1112 897 package.
- Publish 897 only as `quarantined_visual` unless the bad geometry generation
  path is fixed and validation reports `complete_visual_release_candidate`.
- Index the published 897 release into DuckLake after validation.
- Compare it with the existing DB1112 791 quarantined visual release in CLI,
  HTTP, and the two-pane browser viewer.

Updated Done gate for the current slice:

- A real 897 immutable release exists with release-local assets and explicit
  quality evidence.
- Same-release 897 diff is zero.
- 791-vs-897 diff is generated from DuckLake indexes, not a controlled fixture.
- `/model-version/compare` loads two real release ids in two WebGL panes.
- Missing/quarantined mesh rows are surfaced as quality/provenance evidence and
  are never silently treated as complete visual geometry.

## Continuation Slice - Explicit DuckLake Catalog Migration

Why this slice matters:

- The web_server read path is intentionally read-only and must not silently
  mutate DuckLake catalog schema.
- Older local/project catalogs may lack newly added release-quality evidence
  columns.
- Operators need an explicit, scriptable command that applies catalog migrations
  before starting read-only web_server deployments.

Success criteria:

- `aios-database model-version migrate --project AvevaMarineSample --json`
  opens the DuckLake writer path, applies all compatible schema migrations, and
  returns a structured JSON report with metadata path, data path, schema name,
  release count, key table availability, and release quality column presence.
- The command is safe to run repeatedly.
- The command does not generate models, publish releases, index releases, or
  mutate immutable Parquet/GLB release packages.
- CLI verification proves the existing DB1112 catalog remains readable after
  migration and the 791->897 diff counts remain stable.
- HTTP verification proves the read-only web_server still returns release
  quality annotations after migration.
- Progress and architecture documents record the command, validation evidence,
  and remaining risks.

Completed evidence for this slice:

- `model-version migrate` now exists as an explicit writer-path catalog
  migration command.
- Repeated JSON runs against `AvevaMarineSample` report:
  - `release_count=4`;
  - all required DuckLake tables present;
  - all release provenance/quality columns present;
  - `release_quality_columns_present=true`;
  - `migrated=true`.
- The command only touches DuckLake schema/catalog metadata; it does not
  generate, publish, index, or mutate immutable Parquet/GLB packages.
- CLI diff after migration remains stable:
  - component diff 791 -> 897:
    `added=5059 deleted=2525 changed=43 unchanged=23549`;
  - unit diff 791 -> 897:
    `added=91 deleted=17 changed=119 unchanged=548`.
- Rebuilt and restarted `web_server` on port 3926 after stopping only the
  previous validation process holding the executable lock.
- HTTP read validation after migration:
  - `/api/version` returns build date `2026-06-20 16:01:25 UTC+8`;
  - `/api/model-version/releases?project=AvevaMarineSample` returns quality
    reasons and validation flags for both DB1112 physical releases;
  - `/api/model-version/diff` returns stable 791 -> 897 counts;
  - `/api/model-version/releases/{release_id}/runtime-scene` succeeds for both
    791 and 897 with release-local mesh URL patterns.
- Browser validation after migration:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-post-migrate-agent-browser.png`
  shows both WebGL panes, `quarantined_visual` quality badges, quality reasons,
  and the stable 791 -> 897 diff summary.

## Continuation Slice - DuckLake Schema Migration Audit

Why this slice matters:

- The first explicit `migrate` command proves idempotent schema readiness, but
  production operators also need to know which migrations have actually been
  applied.
- Read-only deployments should fail fast not only for missing provenance
  columns, but also for missing migration audit infrastructure.
- The remaining compatibility default where a missing release status can read
  as `published` should be isolated behind an explicit backfill/audit trail
  before it can be removed safely.

Success criteria:

- DuckLake writer migration creates a schema migration audit table.
- Each compatible schema/backfill migration records an idempotent migration id
  and timestamp.
- `model-version migrate --json` reports applied migration ids and migration
  count in addition to table/column readiness.
- Read-only schema validation requires the migration audit table and gives the
  same explicit `model-version migrate` remediation if it is missing.
- The command remains safe to run repeatedly and does not mutate immutable
  release packages.
- CLI/HTTP/browser validation proves 791/897 list, diff, runtime-scene, and
  two-pane compare behavior remain stable after the audit migration.

Completed evidence:

- `model-version migrate --project AvevaMarineSample --json` now reports:
  - `release_count=4`;
  - `schema_migration_count=5`;
  - `applied_schema_migrations` containing ids `0001` through `0005`;
  - `required_tables.model_version_schema_migrations=true`;
  - `release_quality_columns_present=true`;
  - `missing_tables=[]`;
  - `missing_release_columns=[]`;
  - `migrated=true`.
- Re-running the same command leaves `schema_migration_count=5`, proving the
  current audit recording is idempotent.
- CLI regression remains stable:
  - component diff 791 -> 897:
    `added=5059 deleted=2525 changed=43 unchanged=23549`;
  - unit diff 791 -> 897:
    `added=91 deleted=17 changed=119 unchanged=548`;
  - same-release 897 diff:
    `added=0 deleted=0 changed=0 unchanged=28651`.
- Rebuilt `web_server` with `web_server,model-version-ducklake`, restarted it
  on port `3926`, and verified:
  - `/api/version` build date `2026-06-20 16:28:25 UTC+8`;
  - `/api/model-version/releases?project=AvevaMarineSample` returns quality
    reasons and flags for both physical releases;
  - `/api/model-version/diff` returns the stable 791 -> 897 counts;
  - both 791 and 897 `runtime-scene` responses use release-local mesh URL
    patterns.
- Browser validation screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-schema-audit-agent-browser.png`
  shows two WebGL panes, `quarantined_visual` badges, quality reasons, and
  diff cards for Added `5059`, Deleted `2525`, Changed `43`, Unchanged `23549`,
  Emitted `200`.

Remaining risks after this slice:

- Read-only schema validation requires the audit table but does not yet verify
  that the exact required migration id set exists.
- Migration id insertion is app-level idempotent; concurrent writers still need
  a writer lock/single-writer queue or a server catalog.
- Missing-status compatibility fallback, publish attempt/reconcile, richer
  quarantine/validation report hashes, and production-scale tiled comparison
  remain open P0/P1 items for the overall goal.

## Continuation Slice - Required Migration Id Enforcement

Why this slice matters:

- The schema audit table proves that migration records can exist, but a
  read-only deployment also needs to know whether all migrations required by
  the current binary are present.
- Without required-id validation, a catalog with a manually created empty audit
  table could pass read-only schema validation while missing an important
  compatibility backfill.
- Operators need JSON evidence that distinguishes applied ids from missing
  required ids.

Success criteria:

- The current binary owns a single required migration id list used by both
  writer reporting and read-only validation.
- `model-version migrate --json` reports `required_schema_migrations` and
  `missing_schema_migrations`.
- Read-only schema validation fails with the existing migration remediation if
  any required migration id is missing.
- The change remains package-safe: it does not mutate immutable Parquet/GLB
  release packages and only touches DuckLake catalog/audit metadata.
- CLI validation proves repeated migration remains idempotent and DB1112
  791/897 diff counts remain stable.
- HTTP validation against a rebuilt `web_server` proves read-only release list,
  diff, and runtime-scene still work after enforcing required ids.

Completed evidence:

- Oracle MCP review sessions used as second-opinion architecture input:
  - `e3d-model-version-ducklake-current`;
  - `e3d-ducklake-review-core-inline`.
- The implementation now has one required migration id list:
  - `0001_base_model_version_schema`;
  - `0002_release_lifecycle_quality_columns`;
  - `0003_release_quality_evidence_columns`;
  - `0004_release_provenance_columns`;
  - `0005_release_status_lifecycle_quality_backfill`.
- `model-version migrate --project AvevaMarineSample --json` now reports:
  - `schema_migration_count=5`;
  - `required_schema_migrations` with the five ids above;
  - `missing_schema_migrations=[]`.
- Negative read-only validation was run against a temporary catalog under
  `target/codex-ducklake-migration-id-negative` only:
  - after deleting `0005_release_status_lifecycle_quality_backfill`, read-only
    `model-version list` exits `1`;
  - the error message names the missing id and tells the operator to run
    `model-version migrate`;
  - re-running `migrate` restores `missing_schema_migrations=[]`;
  - read-only `list` succeeds again.
- Real DB1112 CLI regression remains stable after the new check:
  - component diff 791 -> 897:
    `added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200`;
  - unit diff 791 -> 897:
    `added=91 deleted=17 changed=119 unchanged=548 emitted=200`;
  - same-release 897 diff:
    `added=0 deleted=0 changed=0 unchanged=28651 emitted=0`.
- Build/format validation:
  - `cargo fmt --check` passed;
  - `cargo build --bin aios-database --features "model-version-ducklake"`
    passed with existing pdms-io warnings only;
  - `cargo check --bin aios-database` without `model-version-ducklake` passed
    with existing pdms-io warnings only.
- Rebuilt and restarted `web_server` on `http://127.0.0.1:3926`:
  - stopped previous validation PID `56044` only after verifying it was
    `web_server`;
  - rebuilt again after the migration-id constant cleanup, stopped PID `27576`,
    and started PID `65428`;
  - `/api/version` build date `2026-06-20 17:00:54 UTC+8`.
- HTTP validation:
  - `/api/model-version/releases?project=AvevaMarineSample` returns four
    releases;
  - `/api/model-version/diff` for 791 -> 897 returns the stable diff counts;
  - both runtime-scene endpoints return `quarantined_visual` and release-local
    mesh URL patterns.
- Browser validation screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-required-migration-ids-agent-browser.png`
  shows two WebGL panes, `quarantined_visual` badges, quality reasons, and diff
  cards Added `5059`, Deleted `2525`, Changed `43`, Unchanged `23549`,
  Emitted `200`.

Remaining risks after this slice:

- Migration id idempotence is still app-level; production concurrent writers
  need a single-writer queue or a server catalog.
- Missing-status compatibility fallback, publish attempt/reconcile, richer
  validation/quarantine evidence, and production-scale tiled comparison remain
  open for the overall goal.

## Continuation Slice - Publish Input Safety And Provenance Ordering

Why this slice matters:

- `register` and `publish-history` are writer paths that can create immutable
  release package directories. Input validation must happen before package
  materialization.
- `release_id` was already path-safe, but `project_name`, `branch_id`, and
  `parent_release_id` were not yet uniformly validated in the core writer
  functions.
- A malformed baseline provenance payload must fail before creating an orphan
  package.
- `release_root` must not be allowed to sit inside the source/current Parquet
  directory, and source/current Parquet must not sit inside the target release
  package destination.

Success criteria:

- Core writer code validates release/project/branch/parent ids before any
  package copy or DuckLake write.
- Baseline state manifest path/hash evidence is parsed and verified before
  package materialization.
- Register/publish reject unsafe release package path relationships:
  - release destination nested below source Parquet;
  - source Parquet nested below release destination;
  - release destination nested below current Parquet;
  - current Parquet nested below release destination.
- Existing idempotent register-from-existing-release-package behavior remains
  allowed when source and destination are the same existing package directory.
- CLI negative validation proves bad inputs fail without creating release
  package directories.
- Existing DB1112 791/897 list/diff/runtime-scene behavior remains stable.

Completed evidence:

- `register_model_release` now validates release/project/branch/parent ids and
  baseline provenance before any package materialization.
- `publish-history` validates release/project/branch/parent ids and release
  package path boundaries before package validation/publish work.
- Release package destination cannot be nested under source/current Parquet,
  and source/current Parquet cannot be nested under the release destination.
- Registering from the same existing immutable package remains allowed for
  idempotent re-registration.
- CLI negative checks against temporary paths under `target/codex-publish-safety`
  proved:
  - bad `release_id` fails and creates no release root;
  - bad `project_name` fails and creates no release root;
  - bad `branch_id` fails and creates no release root;
  - `baseline_state_manifest_hash` without a path fails before creating an
    orphan release package;
  - release root nested inside source Parquet fails and creates no directory;
  - publish release root nested inside current Parquet fails and creates no
    directory.
- CLI positive check proved a valid temporary `register` can still materialize
  a package under `target/codex-publish-safety`.
- Real AvevaMarineSample CLI regression remains stable:
  - `schema_migration_count=5`;
  - `missing_schema_migrations=[]`;
  - release count `4`;
  - 791 -> 897 diff
    `added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200`;
  - 897 -> 897 same-release diff
    `added=0 deleted=0 changed=0 unchanged=28651 emitted=0`.
- Build/format validation:
  - `cargo fmt --check` passed;
  - `cargo build --bin aios-database --features "model-version-ducklake"`
    passed with existing pdms-io warnings only;
  - `cargo check --bin aios-database` passed with existing pdms-io warnings
    only.
- Rebuilt and restarted `web_server`:
  - stopped previous PID `65428` only after verifying it was `web_server`;
  - started PID `39416` on `http://127.0.0.1:3926`;
  - `/api/version` build date `2026-06-20 17:14:36 UTC+8`;
  - release list, 791 -> 897 diff, and both runtime-scene endpoints succeeded.

Remaining risks after this slice:

- Publish attempt/reconcile is still needed for crash recovery between
  materialized packages, DuckLake registration, asset indexing, and final
  publish status.
- Writer concurrency still needs a single-writer queue or server catalog.
- Richer quarantine/validation report hashes and production-scale tiled
  comparison remain open for the overall goal.

## Continuation Slice - Release Events And Reconcile Diagnostics

Why this slice matters:

- The catalog already had `model_release_status_events`, but operators and
  web/API users could not inspect those events directly.
- Interrupted publish/index workflows need a deterministic way to explain
  whether a release is publishable, incomplete, failed, or only missing
  optional indexes.
- Reconcile must be conservative by default: explain state without mutating it,
  and only update status when an explicit repair flag is provided.

Success criteria:

- CLI can list release status events for a release.
- CLI can reconcile a release and report package, component index, mesh asset,
  and unit index evidence.
- HTTP exposes the same events and reconcile report.
- Reconcile is read-only by default.
- Existing DB1112 791/897 diff and runtime-scene behavior remains stable.
- Browser evidence still shows the two-pane model comparison after this slice.

Completed evidence:

- Added typed release status event and reconcile report structures.
- Added `ModelVersionDuckLakeStore::release_events` and
  `ModelVersionDuckLakeStore::reconcile_release`.
- Added public model release façade functions:
  - `get_model_release_events`;
  - `reconcile_model_release`.
- Added CLI commands:
  - `model-version release-events`;
  - `model-version reconcile-release`.
- Added HTTP endpoints:
  - `GET /api/model-version/releases/{release_id}/events`;
  - `POST /api/model-version/releases/{release_id}/reconcile`.
- CLI validation:
  - release events for 791 returns `event_count=5`;
  - reconcile 791 and 897 both return `publishable=true`,
    `applied=false`, `problem_count=0`, `warning_count=0`;
  - 791 -> 897 component diff remains
    `added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200`.
- HTTP validation against rebuilt `web_server`:
  - started PID `68488` on `http://127.0.0.1:3100`;
  - `/api/version` build date `2026-06-20 17:39:04 UTC+8`;
  - release events endpoint returns `event_count=5`;
  - reconcile endpoint returns `publishable=true`, `applied=false`,
    `problem_count=0`;
  - diff and runtime-scene endpoints remain stable.
- Browser validation screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-reconcile-events-agent-browser.png`
  shows two loaded model panes, `quarantined_visual` badges, and stable diff
  cards.

Operational observations:

- Running two DuckLake writer/reconcile commands in parallel produced a real
  metadata file lock failure. This confirms the remaining need for a
  single-writer queue or server catalog before production multi-writer use.
- A fresh web target on D: failed during DuckDB C++ compilation and a later
  D: build failed with `os error 112` disk full. The failed temporary target
  `target/codex-web-reconcile-build` was deleted after path-boundary checks,
  and the final web_server build was done on
  `E:\codex-targets\plant-cli-ducklake-build`.

Remaining risks after this slice:

- DuckLake writer concurrency still needs single-writer orchestration.
- Reconcile currently explains and safely flips lifecycle status, but does not
  yet replay missing asset/unit index jobs automatically.
- Richer validation/quarantine report hashes and full GLB readability/hash
  checks remain open.
- Production-scale compare still needs tiled runtime-scene loading and
  synchronized selection/highlight.

## Continuation Slice - Local DuckLake Catalog Access Serialization And Oracle MCP Review

Why this slice matters:

- The previous release-events/reconcile validation exposed a real local
  DuckLake metadata collision when read-only and writer opens overlapped.
- A local file catalog is acceptable for the current single-machine
  AvevaMarineSample/DB1112 validation path, but it must fail predictably under
  concurrent CLI/web/watcher activity.
- Oracle MCP was requested for second-model architecture review. The MCP dry
  run succeeded with a narrowed source bundle, while the live browser consult
  was blocked by the local Oracle Chrome profile not being signed in.

Architecture decision:

- Keep DuckLake as the model-version catalog and query/index layer for
  immutable release metadata, component/unit snapshots, release events, and
  diff/reconcile queries.
- Keep Parquet/JSON manifests and immutable release package directories as the
  durable data-plane artifacts.
- Keep SurrealDB/SQLite as generation/runtime helpers, not as the source of
  truth for published model-version releases.
- For the local DuckLake file catalog, serialize both read-only and writer
  opens through the same sidecar metadata lock before `ATTACH`.
- For production multi-user deployment, still plan a single-writer queue or
  service/server catalog; local file locking is a conservative bridge, not the
  final distributed concurrency architecture.

Completed evidence:

- Updated `ModelVersionDuckLakeStore::open_inner` so read-only opens also call
  `MetadataFileLock::acquire` and wait on the same lock as writer opens.
- Added contextual lock acquisition errors that include read-only/writer mode
  and metadata path.
- Oracle MCP:
  - `oracle --help` was run for the session;
  - initial MCP dry run with broad files was rejected as too large
    (`~367,780 tokens`);
  - narrowed MCP dry run was acceptable (`~36,824 tokens`);
  - live browser consult failed because no ChatGPT cookies were available and
    the model selector could not be found. No API-cost Oracle run was started.
- CLI validation:
  - `cargo fmt`, `cargo fmt --check` passed;
  - `cargo build --bin aios-database --features "model-version-ducklake"`
    passed with existing pdms-io warnings only;
  - `cargo check --bin aios-database` passed with existing pdms-io warnings
    only;
  - six concurrent CLI read/write jobs all exited `0`;
  - 791 -> 897 diff remains
    `added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200`.
- HTTP validation against rebuilt `web_server`:
  - current PID `38960` on `http://127.0.0.1:3100`;
  - `/api/version` build date `2026-06-20 17:56:25 UTC+8`;
  - events/reconcile/diff/runtime-scene endpoints succeeded;
  - six concurrent HTTP read/write requests all succeeded.

Remaining risks after this slice:

- Local catalog access is now serialized, but long-running readers can block
  writers and vice versa; production still needs job orchestration and/or a
  server catalog.
- Reconcile still reports missing asset/unit work instead of replaying it.
- Validation/quarantine report hashes and GLB readability/hash verification
  remain planned.
- Production-scale compare still needs paged/tiled scene loading and
  synchronized selection/highlight.

## Continuation Slice - Mesh Asset GLB Readability Evidence

Why this slice matters:

- The current release package and mesh asset index prove that GLB files exist,
  are release-local, and have stable hashes, but not that the GLB payload can
  be parsed by the viewer/runtime.
- A two-pane model comparison can still fail late in the browser if a
  release-local GLB is corrupt. Production handoff needs this failure to be
  caught during indexing/reconcile, not by a user staring at an empty pane.

Success criteria:

- Mesh asset indexing records per-asset GLB readability evidence.
- Mesh asset index stats report checked/readable/unreadable counts.
- Reconcile treats unreadable non-builtin visual assets as blocking evidence
  problems and reports missing readability evidence clearly.
- DB1112 release assets for the 791 and 897 comparison can be re-indexed with
  readability evidence.
- Existing CLI diff and HTTP/runtime-scene behavior for 791 -> 897 remains
  stable after re-indexing.
- Progress and architecture docs explain the tradeoff: indexing does the
  heavier GLB validation work; runtime-scene should rely on indexed evidence.

Architecture decision:

- Keep GLB readability validation in the model-version asset indexing stage,
  adjacent to hash/materialization evidence.
- Persist the result in DuckLake and the mesh asset manifest so reconcile and
  HTTP operators can inspect it without reparsing every GLB on every request.
- Treat unreadable files as release evidence problems, not as frontend-only
  loading errors.

Completed evidence:

- Oracle MCP live consult `e3d-ducklake-architectu-current` completed after a
  narrowed dry run (`~181,089` input tokens). It independently supported the
  hybrid architecture: DuckLake as catalog/index/lineage, immutable
  Parquet/JSON/GLB release packages as data plane, SurrealDB as generation and
  runtime graph helper, and fail-closed release-local mesh validation as a P0
  requirement.
- DuckLake migration `0006_mesh_asset_glb_readability_columns` is applied for
  `AvevaMarineSample`; `missing_schema_migrations=[]`.
- DB1112 release-local mesh asset indexes were rebuilt:
  - `codex-ams1112-physical-791-quarantine`: `present=1192`,
    `glb_checked=1192`, `glb_readable=1192`, `glb_unreadable=0`;
  - `codex-ams1112-physical-897-quarantine`: `present=1303`,
    `glb_checked=1303`, `glb_readable=1303`, `glb_unreadable=0`.
- `reconcile-release` for both physical releases reports
  `publishable=true`, `problems=0`, and complete GLB readability evidence.
- CLI and HTTP component diff remain stable for DB1112 `791 -> 897`:
  `added=5059 deleted=2525 changed=43 unchanged=23549 emitted=200`.
- HTTP `mesh-assets` now exposes per-asset `glb_readable=true` evidence and
  release-local mesh paths.
- HTTP `runtime-scene` for 897 still returns release-local mesh URL patterns
  and `quality=quarantined_visual`.
- Browser validation screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-glb-readability-agent-browser.png`.
- Browser iframe introspection confirmed:
  - from release 791: `geometries 2288/2288`, `failed 0`;
  - to release 897: `geometries 2041/2041`, `failed 0`;
  - diff table emitted `200` rows.

Remaining risks after this slice:

- GLB readability currently proves parseable mesh primitives with non-empty
  POSITION accessors. Browser/GPU drawability is still validated by
  HTTP/browser regression, not stored as a separate catalog field.
- Runtime-scene is fail-closed on missing/unreadable index evidence for visual
  releases, so older releases must be re-indexed with the current binary before
  they can pass this stricter gate.
- Component signature to mesh asset lineage is still indirect. A future P0/P1
  slice should persist a stronger component-to-asset bridge for precise visual
  diff explanations.

## Continuation Slice - Paged Runtime Scene Loading

Why this slice matters:

- The two-pane comparison currently proves the DB1112 791/897 case with a
  bounded `limit`, but a real full-site viewer cannot require one giant
  runtime-scene payload.
- A user needs to inspect beyond the first page without changing releases or
  reloading the whole compare page.
- Backend pagination also makes future tiled/synchronized compare work possible
  without changing the release catalog design.

Success criteria:

- `runtime-scene` accepts an `offset` query parameter in addition to `limit`.
- The response exposes `offset`, `next_offset`, `total_components`, and
  `has_more` so clients can page deterministically.
- Invalid offset/limit input is clamped or rejected consistently.
- Existing default behavior remains compatible for callers that only pass
  `limit`.
- The release viewer can load an initial page and then append the next page
  from the same immutable release package.
- HTTP validation proves two consecutive DB1112 pages return different
  component windows and correct pagination metadata.
- Browser validation proves both compare iframes expose loaded/expected/failed
  geometry counts and can load at least one extra page.

Architecture decision:

- Keep pagination in `release_scene`, not in the browser only. The backend owns
  the immutable ordering (`ORDER BY refno_u64`) and the page metadata.
- Keep the page boundary at component rows. A component may contain multiple
  geometries; this avoids splitting an instance transform and its geometries
  across pages.
- Keep the existing `limit` default for compatibility, and add `offset=0` as
  the explicit first-page semantics.

Completed evidence:

- `ModelReleaseSceneResponse` now exposes `total_components`, `offset`,
  `next_offset`, and `has_more`.
- `runtime-scene` accepts `offset`, orders by `refno_u64`, and returns
  deterministic page windows.
- The release viewer can append a second page from the same immutable release
  package without reloading the iframe.
- The compare page passes `viewer_limit` into both release-viewer iframes so
  small-page browser validation can exercise pagination.
- Validation against rebuilt `web_server` PID `3792` on
  `http://127.0.0.1:3100` confirmed:
  - 897 page 0: `offset=0 next_offset=10 has_more=true total=28651`;
  - 897 page 1: `offset=10 next_offset=20 has_more=true`;
  - 791 page 0: `offset=0 next_offset=10 has_more=true total=26117`;
  - 791 page 1: `offset=10 next_offset=20 has_more=true`;
  - out-of-range 897 offset returns an empty page with `has_more=false`;
  - `limit=0` is clamped to `limit=1` for compatibility.
- Browser validation with `viewer_limit=10` confirmed both panes loaded two
  pages:
  - 791: `loadedComponents=20`, `loadedGeometries=20/20`, `failed=0`;
  - 897: `loadedComponents=20`, `loadedGeometries=12/12`, `failed=0`;
  - diff table still emitted `200` rows.
- Browser screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-paged-runtime-scene-agent-browser.png`.

Remaining risks after this slice:

- Component-row paging is not yet spatial tiling; full-site production compare
  still needs bbox/tree/tile filters.
- Camera sync, selection sync, and diff-row-to-render-object highlighting are
  still open P1/P2 UI work.
- Runtime-scene paging now protects response size, but very large visible page
  counts still need client-side eviction or LOD strategy.

## Completed Implementation Slice: Diff Row Selection And Viewer Highlight

Why this slice matters:

- The two-pane viewer now loads and pages real DB1112 release geometry, but the
  diff table is still passive. A user can see `added/deleted/changed` counts,
  yet cannot click a changed row and find the corresponding object in either
  3D pane.
- Production comparison needs an operator path from domain diff evidence to
  rendered release-local geometry. Without that, the UI is demonstrative but
  not directly actionable.

Success criteria:

- Each loaded release viewer records a stable component-to-render-object index
  from `component_key` to loaded GLB model ids and AABB evidence.
- The release viewer exposes a browser-callable API that can:
  - clear the previous selection;
  - highlight all loaded geometry for a `component_key`;
  - load a single targeted component by `component_key` from the immutable
    release package when it is not in the current page;
  - focus the camera on the selected component when AABB evidence exists;
  - report `found=false` when the component does not exist or has no renderable
    geometry in that release.
- The compare page makes diff rows clickable and calls both iframe viewers with
  the selected `component_key`.
- Added/deleted rows should naturally show `found=false` in the side where the
  component does not exist in that release; changed rows should find the object
  in both panes when the component is loaded.
- Browser validation against DB1112 791/897 proves at least one clicked diff
  row updates row selection state, calls both iframes, and returns explicit
  per-pane selection evidence.
- No `cargo test`; web behavior is validated through rebuilt service + HTTP /
  browser automation.

Architecture decision:

- Keep selection/highlight in the release-viewer iframe because it owns xeokit
  model/object ids and camera state.
- Keep compare-page logic domain-oriented: it only knows `component_key`,
  `change_type`, and per-pane selection status.
- Avoid full auto-paging for off-screen diff rows. Instead, the viewer performs
  a bounded `runtime-scene?component_key=...&limit=1` targeted load, then
  highlights/focuses the loaded component. Future tiled/bbox search remains a
  separate production navigation workflow.

Completed evidence:

- Backend `runtime-scene` accepts `component_key` and returns a deterministic
  one-component window with `has_more=false`.
- Compare rows are clickable/keyboard-selectable and call both iframe viewers.
- Browser validation used `viewer_limit=10` to prove the selected changed row
  was not merely already loaded in the first page.
- Validated changed row:
  - `component_key=1112:75144748061193`
  - `refno=17496_250377`
  - `noun=BOX`
  - from release found `1` geometry
  - to release found `1` geometry
- Screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-diff-selection-agent-browser.png`.

## Completed Implementation Slice: Two-Pane Camera Sync

Why this slice matters:

- The compare UI can now select and highlight the same `component_key` in both
  release panes, but the camera positions are still independent.
- A real operator comparing two model versions needs to rotate/zoom one pane
  and see the other pane track the same viewpoint; otherwise visual comparison
  requires constant manual alignment.

Success criteria:

- The release-viewer iframe exposes browser-callable camera APIs:
  - read the current camera snapshot;
  - apply a validated camera snapshot from the compare page;
  - expose a stable rounded camera signature for browser verification.
- The compare page provides a binary camera-sync control and clear runtime
  status.
- When camera sync is enabled, a camera change in either iframe is propagated
  to the other iframe without recursive oscillation.
- Selection/focus and page append behavior remain compatible with camera sync.
- Browser validation against DB1112 791/897 proves a camera change applied to
  one pane propagates to the other pane, with matching signatures.
- No `cargo test`; web behavior is validated through rebuilt service + HTTP /
  browser automation.

Architecture decision:

- Keep camera read/write primitives inside release-viewer because it owns the
  xeokit `viewer.camera` object.
- Keep synchronization orchestration in the compare page because it owns both
  iframes and the user-facing sync control.
- Use rounded camera signatures for sync detection so small floating-point
  drift does not create infinite ping-pong updates.

Completed evidence:

- The release-viewer iframe exposes:
  - `window.__MODEL_VERSION_GET_CAMERA()`
  - `window.__MODEL_VERSION_SET_CAMERA(snapshot, options)`
  - `window.__MODEL_VERSION_GET_CAMERA_SIGNATURE()`
- The compare page exposes an explicit `Camera sync` checkbox and runtime sync
  status.
- Browser validation against DB1112 791/897 confirmed both propagation
  directions:
  - left pane camera change propagated to the right pane with status
    `from -> to`;
  - right pane camera change propagated to the left pane with status
    `to -> from`;
  - final rounded signatures matched in both iframes;
  - `failedGeometries=0` in both panes.
- Screenshot:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-camera-sync-agent-browser.png`.

## Completed Implementation Slice: Added/Deleted Absence Visualization

Why this slice matters:

- Added/deleted rows currently return `found=false` on the side where the
  component is absent, but the 3D pane itself does not make that absence
  visually obvious.
- Operators comparing model versions need the missing side to say "absent in
  this release" or "no renderable geometry" explicitly, rather than looking
  like a viewer loading problem.

Success criteria:

- The release-viewer iframe can show and clear a visible absence notice for a
  selected component.
- The absence notice distinguishes:
  - component absent from that release;
  - component exists but has no renderable geometry after quarantine.
- The compare page passes expected presence by side:
  - added: old/from side is expected absent, new/to side expected present;
  - deleted: old/from side expected present, new/to side expected absent;
  - changed: both sides expected present.
- Browser validation against DB1112 791/897 proves at least one added row and
  one deleted row select correctly:
  - present side highlights/focuses geometry when renderable;
  - absent side shows absence notice and returns explicit evidence.
- No `cargo test`; web behavior is validated through rebuilt service + HTTP /
  browser automation.

Architecture decision:

- Do not fabricate ghost/tombstone geometry from hashes alone. Current diff
  rows contain AABB hashes, not full old/new AABB coordinates.
- Keep this as a UI-level absence indicator backed by release-scene lookup
  evidence. A future contract can add full AABB tombstones for spatial ghost
  boxes.

Completed evidence:

- Added row validated:
  - `component_key=1112:75144748078198`
  - `refno=17496_267382`
  - `noun=CYLI`
  - from side expected absent and shows `Absent in this release`
  - to side found `1` geometry
- Deleted row validated:
  - `component_key=1112:75144747883391`
  - `refno=17496_72575`
  - `noun=FLOOR`
  - from side found `1` geometry
  - to side expected absent and shows `Absent in this release`
- Screenshots:
  - `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-added-absence-agent-browser.png`
  - `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-deleted-absence-agent-browser.png`

## Current Continuation: Evidence-Bound Increment Watcher

The latest continuation extends the Oracle-backed production architecture from
manual source observation into the automated watcher path:

- `model-version observe-source` can create immutable source observation
  manifests for a DB file and selected/latest sesno.
- `incremental-sesno` can consume a source observation manifest plus expected
  manifest SHA-256 and rejects mismatched manifest hash, project, dbnum, sesno
  range, unstable quiet-window evidence, or changed primary DB hash.
- `watch-incremental` now creates one source observation manifest per detected
  dbnum update and passes the manifest path/hash into the guarded
  `incremental-sesno` runner.
- DB `1112` was validated by forcing the local db_index baseline to `896`,
  letting the watcher rescan the real AvevaMarineSample source DB to `897`,
  and confirming parse/save evidence with
  `source_observation.source_hash_unchanged=true`.
- `incremental-sesno --generate-model` now emits an explicit publication
  handoff instead of silently publishing. The handoff records the generated
  affected-scope Parquet package, package hash, rows, and a machine-executable
  `model-version register` argv.
- Handoff registrations are forced to `patch_only` with validation flags so an
  affected-scope increment cannot be mistaken for a complete visual model
  release.
- DB `1112` 896 -> 897 was validated through guarded parse/save, incremental
  model generation, post-generation Parquet export, handoff manifest creation,
  and executing the handoff `register_argv` into an immutable staged
  `patch_only` release.

Next production decision:

- Harden backend generation prechecks for DB `1112`: current validation still
  reports missing `output\AvevaMarineSample\scene_tree\1112.tree` and falls
  back to slower DB-based generation paths. This fallback succeeded, but a
  production-grade path should either generate/validate the tree index before
  incremental generation or record a deliberate degraded-mode decision.

## Current Implementation Slice: Generation Tree-Index Evidence Gate

This slice implements the first hardening step from the Oracle-backed
architecture plan:

- `incremental-sesno --generate-model` and `watch-incremental --generate-model`
  must expose machine-readable tree-index evidence for the dbnums they are
  about to generate.
- The default path may continue with degraded generation if a tree file is
  missing, but the JSON summary and publication handoff must record the missing
  `scene_tree/<dbnum>.tree` evidence.
- A new strict mode must be available for production/full visual release
  workflows. In strict mode, missing tree-index files fail before model
  generation rather than silently using slower fallback paths.
- The implementation must not auto-run long `--gen-indextree 1112` work from
  the watcher/default incremental path. Tree generation remains an explicit
  bounded operator job.

Success criteria:

- CLI help for `incremental-sesno` and `watch-incremental` exposes the strict
  tree-index gate option.
- DB `1112` strict validation fails quickly with a clear JSON/CLI error when
  `output\AvevaMarineSample\scene_tree\1112.tree` is absent.
- DB `1112` default validation can still run affected-scope generation, and
  its summary/handoff include `tree_index.ready=false`,
  `missing_dbnums=[1112]`, the checked path, and a recommended action.
- The publication handoff remains `patch_only` and does not auto-register a
  release.
- Build/fmt checks pass without running `cargo test`.
- Progress documentation records validation commands, results, decisions, and
  self-review.

Status: completed on 2026-06-20.

Evidence summary:

- `incremental-sesno --help` and `watch-incremental --help` expose
  `--require-tree-index`.
- DB `1112` strict mode fails before model generation with
  `tree_index_missing`, `mode=strict_required`, and `missing_dbnums=[1112]`.
- DB `1112` default mode still generates the affected-scope package and writes
  a handoff whose top-level evidence and register metadata include
  `tree_index.ready=false`, `mode=degraded_allowed`, and
  `missing_dbnums=[1112]`.
- The generated handoff remains `patch_only`; no automatic release
  registration occurs.
- `cargo fmt --check` and `cargo build --bin aios-database --features
  "model-version-ducklake"` passed; no `cargo test` was run.

## Current Implementation Slice: Baseline Scene-Tree Readiness Evidence

This slice tightens the `BaselineStateManager` boundary that Oracle identified
as the real model-version correctness hinge.

Current gap:

- `baseline_state_manifest.json` proves an isolated physical baseline snapshot
  and DB-file hash, but by itself it does not prove the baseline workspace has
  generated `scene_tree/<dbnum>.tree` artifacts.
- A caller can run `validate-baseline-state` and see `ready=true` even when the
  baseline output root is still missing the tree index required by full visual
  replay/generation workflows.

Required behavior:

- `model-version validate-baseline-state --json` must report scene-tree
  evidence inferred from the baseline manifest output root:
  `<output_root>/<project>/scene_tree`.
- The command must also accept `--scene-tree-dir` for explicit validation.
- A new `--require-scene-tree` flag must fail fast with a machine-readable
  error when either `scene_tree/<dbnum>.tree` or `db_meta_info.json` is missing.
- The default path remains non-blocking, but the JSON response must make the
  degraded state obvious so operators do not confuse physical snapshot
  readiness with full generation readiness.

Success criteria:

- CLI help for `model-version validate-baseline-state` exposes
  `--scene-tree-dir` and `--require-scene-tree`.
- DB `1112` physical baseline manifest validation without strict mode exits 0
  and reports `scene_tree.tree_file_exists=false` for the current 791 reuse
  snapshot.
- The same validation with `--require-scene-tree` exits 1 with
  `baseline_scene_tree_missing`.
- Existing web/API baseline validation call sites still compile and default to
  non-strict behavior.
- Build/fmt checks pass without running `cargo test`.
- Progress documentation records validation commands, results, decisions, and
  self-review.

Status: completed on 2026-06-20.

Evidence summary:

- `model-version validate-baseline-state --help` exposes `--scene-tree-dir`
  and `--require-scene-tree`.
- DB `1112` baseline snapshot
  `codex-ams1112-physical-791-reuse-20260620` validates in default mode with
  `scene_tree.required=false`, `tree_file_exists=false`, and
  `db_meta_info_exists=true`.
- The same baseline fails in strict mode with `baseline_scene_tree_missing` and
  the missing `1112.tree` path.
- `POST /api/model-version/runs/parse-baseline` on an isolated rebuilt
  `web_server` starts the bounded DB1112 baseline parse run and preserves source
  DB hash evidence; a wrong `dbnum=9999` request returns HTTP 400 before launch.
- `cargo fmt --check`, the `aios-database` feature build, and the
  `web_server` feature build/check passed; no `cargo test` was run.

## Current Implementation Slice: Publish-History Scene-Tree Gate

This slice closes the publish boundary drift identified by the latest Oracle
review:

- `validate-history-replay` already knows how to classify replay packages with
  optional scene-tree evidence.
- `validate-baseline-state` already reports and can require scene-tree
  readiness.
- `publish-history` must reuse that same evidence path instead of hard-coding
  non-strict scene-tree validation.

Success criteria:

- `model-version publish-history --help` exposes `--scene-tree-dir` and
  `--require-scene-tree`.
- `publish-history --require-scene-tree` fails before DuckLake registration
  when the replay workspace is missing `scene_tree/<dbnum>.tree` or
  `db_meta_info.json`.
- Default `publish-history` behavior remains backward compatible but records
  scene-tree evidence in JSON safety checks and release metadata.
- The failure is machine-readable through the existing
  `missing_scene_tree_baseline` classification.
- Validation uses CLI JSON/help and does not run `cargo test`.

## Current Implementation Slice: Component-To-Mesh Asset Lineage

This slice addresses the remaining visual-diff explainability gap:

- Runtime-scene already fails closed unless a release-local mesh asset index is
  present and readable.
- The viewer can highlight a selected `component_key`, but the selection result
  does not yet prove which release-local GLB assets were used for that
  component.
- A production compare UI should expose component-to-asset lineage so a user can
  audit a visual difference from diff row -> component -> geometry row -> GLB
  URL/hash/readability evidence.

Success criteria:

- `runtime-scene` geometry rows include mesh asset evidence joined from
  `model_release_mesh_assets`.
- The release viewer loads GLBs from per-geometry `mesh_asset.mesh_url` when
  available, preserving the release-local asset contract.
- Selecting a diff row returns per-pane asset lineage evidence:
  asset count, geo hashes, release-local mesh URLs, SHA-256, byte size, and GLB
  readability status where available.
- The compare page displays concise selected-component asset lineage in the
  selection status and exposes it in DOM datasets for browser verification.
- HTTP validation proves targeted DB1112 `runtime-scene?component_key=...`
  returns mesh asset evidence for a changed row.
- Browser validation proves a DB1112 791/897 diff row selection reports asset
  lineage for both panes.
- No `cargo test`; validation uses feature builds plus HTTP/browser checks.

Status: completed on 2026-06-21.

Evidence summary:

- `ModelReleaseSceneGeometry` now carries optional
  `ModelReleaseSceneMeshAssetEvidence` from `model_release_mesh_assets`.
- `release_scene` joins release-local mesh asset rows and returns
  URL/hash/byte/readability evidence with geometry rows.
- The release viewer prefers per-geometry `mesh_asset.mesh_url` before falling
  back to computed mesh URLs, preserving release-local asset ownership.
- The compare page selection status displays per-pane asset lineage and exposes
  DOM datasets for automated browser verification.
- HTTP validation found the first changed row
  `component_key=1112:75144748061191` has a legitimate no-renderable target
  side, then used `component_key=1112:75144748061193` for a both-pane asset
  proof.
- Browser validation on `791 -> 897`, `change_type=changed`, selected
  `1112:75144748061193`:
  - from assets: `count=1`, `readable=1`, URL under
    `/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-791-quarantine/...`;
  - to assets: `count=1`, `readable=1`, URL under
    `/files/output/AvevaMarineSample/model_versions/releases/codex-ams1112-physical-897-quarantine/...`;
  - both sides report SHA-256
    `0ecd246b587d82f8853559eb951da07ae6b6ea56a35ecef43e6ac11fb95c5701`.
- Desktop and mobile browser pixel probes saw non-transparent, non-black WebGL
  canvas samples in both panes.
- Evidence screenshots:
  - `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-asset-lineage-agent-browser-full.png`
  - `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-asset-lineage-mobile-agent-browser.png`
- `cargo fmt --check`, `cargo build --bin aios-database --features
  "model-version-ducklake"`, and `cargo build --bin web_server --features
  "web_server,model-version-ducklake"` passed; no `cargo test` was run.

## Current Implementation Slice: Release Pair Production Readiness Gate

This slice closes the operator handoff gap that remains after the two-pane
viewer and asset lineage work:

- The DB1112 `791 -> 897` comparison can now render and explain release-local
  GLB assets.
- The releases are still `quarantined_visual`, and the larger historical
  baseline hydrate/restore requirement is not complete.
- A real user or developer needs a single machine-readable gate that says
  whether a release pair is production-ready for visual comparison, or why it
  must remain a demo/quarantine comparison.

Required behavior:

- Add a reusable release pair readiness check over DuckLake release records and
  release indexes.
- The check must classify at least:
  - `production_ready`;
  - `quarantined_visual`;
  - `incomplete_indexes`;
  - `missing_release`;
  - `not_production_ready`.
- It must report per-release evidence:
  lifecycle, quality, validation flags, baseline manifest evidence,
  component index count, unit index count if available, mesh asset index stats,
  missing/unreadable/non-local asset counts, and recommended action.
- It must report pair evidence:
  from/to release ids, diff summary counts, whether both releases are published,
  whether both are complete visual releases, whether both have usable release
  local assets, and whether production comparison is allowed.
- Expose the check through:
  - `aios-database model-version validate-compare-readiness --json`;
  - `GET /api/model-version/compare-readiness?...`.
- The compare page should fetch and display the readiness classification so the
  user cannot confuse a quarantined/demo comparison with production sign-off.
- DB1112 `791 -> 897` validation should return a clear non-production
  classification because both sample releases are intentionally quarantined, but
  should still report asset/index evidence as present.
- No `cargo test`; validation uses CLI JSON, HTTP, feature builds, and browser
  smoke if UI changes are made.

Status: completed on 2026-06-21.

Evidence summary:

- `validate-compare-readiness --json` classifies
  `codex-ams1112-physical-791-quarantine -> codex-ams1112-physical-897-quarantine`
  as `quarantined_visual`.
- The pair is not production-ready:
  `production_ready=false`, `production_comparison_allowed=false`,
  `both_published=true`, `both_complete_visual=false`.
- Index and asset gates are present:
  `component_indexes_ready=true`, `mesh_assets_ready=true`, release-local mesh
  asset violation count is `0` on both sides, and unit indexes exist.
- Diff summary remains the expected DB1112 sample:
  `added=5059`, `deleted=2525`, `changed=43`, `unchanged=23549`.
- `GET /api/model-version/compare-readiness` returns the same evidence through
  `web_server`.
- `/model-version/compare` renders a compact readiness status above the
  two-pane viewer and exposes DOM datasets for browser verification.
- Browser validation saved:
  `.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-readiness-agent-browser.png`.
- `cargo fmt --check`, feature builds for `aios-database` and `web_server`,
  CLI JSON, HTTP, and browser checks passed; no `cargo test` was run.

## Current Implementation Slice: Historical Baseline Inspect HTTP Preflight

This slice addresses the remaining history-version selection risk:

- Users need to pick a DB1112 historical record, such as `sesno=791`, to test
  incremental model changes.
- The current CLI can inspect a target sesno through pdms-io, but web/API
  callers cannot ask for the same evidence before launching replay/generation.
- DB1112 `791` and `897` currently locate exact sessions but do not provide a
  publishable full-state baseline hydrate proof: index traversal returns only a
  tiny visible set and includes index/parse errors.
- A production backend must expose this as a read-only preflight with a clear
  recommendation instead of letting callers confuse a historical session with a
  complete visual baseline.

Success criteria:

- Add a read-only HTTP endpoint for history baseline inspection using the same
  pdms-io evidence as `model-version inspect-history-baseline`.
- The endpoint must accept project, source DB file, target sesno, optional sample
  limit, `allow_nearest_sesno`, and `detail`.
- The endpoint must validate the source DB file exists and is a file, and it
  must return clear HTTP errors for missing files, nonexistent exact sessions,
  or pdms-io parse/index failures.
- DB1112 `sesno=791` and `sesno=897` through HTTP must return exact sessions,
  `full_state_enumeration_supported=false`, visible refno count, index/parse
  error counts, sample noun counts, and a recommended action that points to
  physical baseline snapshot or a proven hydrate provider.
- The route list and architecture/verification docs must include the new
  endpoint.
- Validation must use CLI JSON and `web_server` HTTP checks; no `cargo test`.

Status: completed on 2026-06-21.

Evidence summary:

- Added read-only HTTP endpoint
  `GET /api/model-version/history-baseline-inspect`.
- The endpoint reuses `inspect_history_baseline` / pdms-io evidence, clamps
  `parse_sample_limit` to a bounded maximum, and executes file inspection in a
  blocking worker.
- Startup route logging includes the new endpoint.
- DB1112 source file
  `D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`
  was inspected through CLI and HTTP.
- HTTP `target_sesno=791`:
  `exact_sesno_found=true`, `resolved_sesno=791`, `latest_sesno=897`,
  `visible_refno_count=5`, `index_error_count=1`, `parse_error_count=2`,
  `full_state_enumeration_supported=false`.
- HTTP `target_sesno=897`:
  `exact_sesno_found=true`, `resolved_sesno=897`, `visible_refno_count=5`,
  `index_error_count=1`, `parse_error_count=2`,
  `full_state_enumeration_supported=false`.
- HTTP `target_sesno=999999` without nearest fallback returns HTTP 404 with
  `requested session 999999 does not exist`.
- HTTP `target_sesno=999999&allow_nearest_sesno=true` returns HTTP 200 with
  `resolved_sesno=897` and `exact_sesno_found=false`.
- Missing source DB file returns HTTP 404 with a clear JSON error.
- `cargo fmt --check`, targeted diff whitespace check, and feature builds for
  `aios-database` and `web_server` passed; no `cargo test` was run.

## Current Implementation Slice: Prepare-History Baseline Proof Gate

This slice turns the remaining historical replay warning into a production
boundary:

- `prepare-history-replay` currently writes runnable baseline/replay DbOptions
  even though `baseline_parse` uses the current visible source DB file state.
- That is only safe when the source file is already an isolated physical
  baseline for `from_sesno`, such as a directory produced by
  `prepare-physical-baseline-snapshot`.
- For DB1112 `791/897`, pdms-io inspect cannot prove full target-sesno hydrate,
  so the default path must not silently create a replay plan from the mutable
  current file.

Success criteria:

- `prepare-history-replay` defaults to fail-closed unless the caller explicitly
  confirms the source DB file is already the physical baseline for
  `from_sesno`.
- The CLI exposes this confirmation as a clearly named flag.
- The JSON request/response safety checks record whether the confirmation was
  supplied.
- The failure message must explain that current-file full-sync is not
  target-sesno hydrate and point to physical baseline snapshot or a proven
  hydrate provider.
- `prepare-physical-baseline-snapshot` generated hint argv must include the
  confirmation flag because that workflow constructs an isolated physical
  source.
- Validation must prove:
  - DB1112 `prepare-history-replay` without confirmation exits non-zero before
    writing/overwriting replay configs;
  - the same command with confirmation succeeds and records the confirmation in
    JSON safety checks;
  - `prepare-physical-baseline-snapshot --help`/JSON hint exposes the flag.
- No `cargo test`; validation uses CLI JSON/help and feature builds.

Status: completed on 2026-06-21.

Evidence summary:

- Added `baseline_source_confirmed_at_from_sesno` to
  `ModelHistoryReplayPrepareRequest` and `ModelHistoryReplaySafetyChecks`.
- `prepare-history-replay` now fails before writing configs unless
  `--baseline-source-confirmed-at-from-sesno` is supplied.
- The failure explains that current-file full-sync is not pdms-io target-sesno
  hydrate and points to physical baseline snapshot, published baseline restore,
  or a proven hydrate provider.
- `prepare-history-replay --help` exposes the confirmation flag.
- DB1112 validation without the flag:
  exit code `1`, message contains `requires explicit baseline source
  confirmation`, and neither replay nor baseline config was written.
- DB1112 validation with the flag:
  exit code `0`, `written=true`,
  `safety_checks.baseline_source_confirmed_at_from_sesno=true`, and
  `baseline_target_sesno_reconstruction_supported=false`.
- `prepare-physical-baseline-snapshot` JSON hint now includes
  `--baseline-source-confirmed-at-from-sesno` and its `--source-db-file` points
  to the snapshot replacement DB file, not the mutable original DB file.
- `cargo fmt --check`, targeted `git diff --check`, and feature builds for
  `aios-database` and `web_server` with
  `model-version-ducklake,surreal-save` passed; no `cargo test` was run.

## Current Analysis Slice: Oracle Model-Version Architecture Review

Status: completed on 2026-06-21.

Purpose:

- Continue architecture analysis with Oracle before the next implementation
  slice.
- Re-check the best model data versioning scheme, DuckLake boundary, DB1112
  `791 -> 897` historical replay path, and missing backend structured APIs.

Evidence summary:

- `sigmap ask` was attempted before planning but timed out after about 124s.
- `mcp__oracle.consult` was attempted first and failed immediately with
  `Transport closed`.
- The same Oracle toolchain was run through browser CLI without API-paid mode:
  - session: `e3d-model-version-plan-review`;
  - reattach command: `oracle session e3d-model-version-plan-review`;
  - input: about `94,726` tokens;
  - files: current hardening architecture plan,
    `src/web_api/model_version_api.rs`, and key version-management files for
    source observation, history baseline, history replay, physical baseline,
    baseline state, and core types.
- Oracle confirmed the current architecture direction:
  - DuckLake belongs in this version only as catalog/read-model/index/diff/audit
    and asset-lineage metadata.
  - DuckLake must not become generation writer, baseline restore, GLB/Parquet
    payload store, session replay logic, or user-facing version identity.
  - The model version contract should be explicit as
    `SOID -> BSID -> IEVID -> GJID -> RID`, where `release_id + package_hash`
    is the user-visible immutable version.
  - `sesno` is only a source/history anchor, not a model version key.
  - The backend still needs structured safety endpoints for
    `prepare-history-replay`, `publish/register`, incremental handoff, and a
    release state machine.

Architecture doc updated:

- `docs/plans/2026-06-20-e3d-version-ducklake-hardening-architecture-dev-plan.md`
  now records the latest Oracle consult, the lineage contract, DuckLake
  boundary, recommended modules, HTTP mutation endpoints, and revised
  development plan.

Next implementation target:

- Add structured
  `POST /api/model-version/runs/prepare-history-replay` using the existing CLI
  baseline proof gate.
- Prefer `snapshot_id` input so the backend reads
  `baseline_state_manifest.json`, uses the snapshot replacement DB file, and
  adds `--baseline-source-confirmed-at-from-sesno` automatically.
- Fail closed for direct live `source_db_file` requests that do not provide
  explicit physical-baseline confirmation.
- Validate through `web_server` HTTP and DB1112 real snapshot evidence; no
  `cargo test`.

## Completed Implementation Slice: Structured Prepare-History Replay HTTP Run API

Status: completed on 2026-06-21.

This slice implements the first Oracle-recommended structured backend safety
endpoint for model-version runs:

- Added `POST /api/model-version/runs/prepare-history-replay`.
- The endpoint supports a safe `snapshot_id` mode:
  - reads `baseline_state_manifest.json`;
  - validates baseline state and replacement DB hash evidence;
  - derives `source_db_file` from the physical snapshot replacement DB;
  - automatically passes `--baseline-source-confirmed-at-from-sesno`.
- Direct `source_db_file` mode is fail-closed unless
  `baseline_source_confirmed_at_from_sesno=true` is supplied.
- The bounded run records command argv, source observation manifest, baseline
  manifest dependency, stdout/stderr paths, source DB hash before/after, and
  run state.
- The route list now includes the new endpoint.

Validation evidence:

- `cargo fmt --check` passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed.
- `web_server` HTTP validation used an isolated process on port `3198`.
- Negative direct request without physical-baseline confirmation returned
  HTTP `400` with:
  `direct prepare-history-replay requires baseline_source_confirmed_at_from_sesno=true`.
- Positive DB1112 physical snapshot request succeeded:
  - `snapshot_id=codex-ams1112-physical-791-reuse-20260620`;
  - `from_sesno=791`;
  - `to_sesno=897`;
  - run id `codex-http-history-snapshot-20260621041907`;
  - `kind=prepare_history_replay`;
  - `status=succeeded`;
  - `exit_code=0`;
  - `source_db_hash_unchanged=true`.
- The generated stdout JSON includes `incremental-sesno --generate-model` and
  `publish-history --materialize-assets` argv, while keeping replay namespace,
  output root, project output, and parquet output isolated from current state.

Next implementation target:

- Add structured `publish-history` / `register` POST endpoints and centralize
  release state transitions.
- Add an incremental handoff endpoint that records staged `patch_only` or
  `quarantined_visual` evidence instead of publishing automatically.
- After those safety APIs exist, wire a full DB1112 physical snapshot replay
  generate/publish flow into the backend UI path and verify the two-pane model
  comparison from two generated releases.

## Completed Implementation Slice: Structured Publish/Register HTTP APIs

Status: completed on 2026-06-21.

Purpose:

- Close the next backend gap after `prepare-history-replay`.
- Expose explicit POST endpoints for release registration and historical
  release publication, while keeping all GET/read paths mutation-free.
- Reuse the existing `register_model_release` and
  `publish_history_model_release` domain services instead of duplicating
  DuckLake/package validation logic in the HTTP layer.

Success criteria:

- Add `POST /api/model-version/releases/register`.
- Add `POST /api/model-version/releases/publish-history`.
- Both endpoints accept structured JSON and derive safe defaults from the
  current project context:
  - `release_root=output/<project>/model_versions/releases`;
  - DuckLake metadata/data paths from `version_context`;
  - current Parquet directory for history publish from
    `output/<project>/parquet/<dbnum>` unless explicitly provided.
- `register` must create or idempotently return a staged immutable release
  from a supplied Parquet package.
- `publish-history` must enforce the existing history replay safety gates:
  non-current isolated Parquet package, non-empty visual package, baseline
  state evidence in metadata, optional scene-tree strict mode, release-local
  mesh materialization when requested, and unit indexing when requested.
- HTTP validation must prove:
  - a controlled staged register request succeeds;
  - a duplicate register request is idempotent;
  - a history publish request missing baseline metadata fails before publish;
  - startup route logging includes both new endpoints.
- Validation must run `web_server` and call HTTP endpoints. Do not run
  `cargo test`.

Current constraints:

- This slice does not yet orchestrate a full replay generate/publish workflow
  from the prepared command plan.
- This slice does not replace the CLI; it gives the backend and future UI a
  structured, auditable mutation path that shares the CLI/domain invariants.

Validation evidence:

- `POST /api/model-version/releases/register` created staged release
  `codex-http-register-final-20260621045248` from
  `output/AvevaMarineSample/parquet/1112`.
- Repeating the same POST returned `already_exists`.
- `GET /api/model-version/releases/codex-http-register-final-20260621045248`
  now returns the staged release detail, fixing the previous published-only
  detail lookup gap.
- `POST /api/model-version/releases/publish-history` with missing baseline
  state metadata returned HTTP `400` with `baseline_missing` before publishing.
- Startup route logging includes both new endpoints.
- Feature builds and HTTP validation passed; no `cargo test` was run.

Next implementation target:

- Add a structured incremental handoff endpoint.
- Start connecting the safe sequence:
  `prepare-history-replay -> generate model run -> publish-history/register`.
- Keep production comparison gated by release quality/readiness until the full
  DB1112 replay-generated pair is proven.

## Completed Implementation Slice: Structured Incremental Handoff HTTP API

Status: completed on 2026-06-21.

Purpose:

- Close the next Oracle-recommended backend safety gap after
  `prepare-history-replay`, `register`, and `publish-history`.
- Convert an `incremental_publication_handoff:v1` manifest into an explicit,
  auditable staged release registration.
- Keep affected-scope incremental outputs out of the production release path
  unless a later state-machine step proves baseline, generation, asset, and
  readiness evidence.

Success criteria:

- Add `POST /api/model-version/incremental/handoff`.
- The endpoint accepts structured JSON with:
  `handoff_manifest_path`, optional `project`, `candidate_index`, `dbnum`,
  `release_id`, `release_label`, `branch_id`, `parent_release_id`,
  `release_quality`, `release_quality_reason`, `validation_flags`, and
  `metadata_json`.
- Manifest validation must fail closed unless:
  - `manifest_version == incremental_publication_handoff:v1`;
  - `policy == explicit_register_required`;
  - `generation_success == true`;
  - the selected candidate exists by `candidate_index` or `dbnum`;
  - candidate `source_parquet_dir` is under the project output root;
  - candidate `package_hash` matches `load_model_package(source_parquet_dir)`.
- The endpoint must call the existing `register_model_release` domain service,
  never `publish_history_model_release`.
- Release quality must default to candidate `suggested_release_quality` or
  `patch_only`; `complete_visual` must be rejected for incremental handoff.
- The registered release must remain `staged` and carry validation flags:
  `incremental_handoff_affected_scope`,
  `explicit_release_registration_required`, and
  `http_incremental_handoff_reviewed`.
- The response must include the DuckLake metadata/data paths, handoff manifest
  path/hash, handoff run id, selected candidate, and registration result.
- HTTP validation must prove:
  - success using the DB1112 `896 -> 897` sample handoff manifest;
  - duplicate registration returns `already_exists`;
  - `complete_visual` override returns HTTP 400 before registration;
  - `GET /api/model-version/releases/{release_id}` can read the staged result;
  - startup route logging includes the new endpoint.

Current constraints:

- This endpoint does not execute generation and does not infer production
  readiness.
- This endpoint treats `1112.tree` degraded evidence as release metadata, not as
  a reason to silently promote an affected-scope package.
- No `cargo test`; validation must use `web_server` HTTP plus CLI/build checks.

Validation evidence:

- Added route:
  `POST /api/model-version/incremental/handoff`.
- Startup route logging includes the new endpoint.
- Success request used DB1112 handoff manifest:
  `target/codex-publication-handoff-tree-evidence/incremental-db1112-896-to-897-20260620T155135644Z.json`.
- Created staged release:
  `codex-http-handoff-20260621052235`.
- Response evidence:
  - `registration.status=created`;
  - `release_lifecycle=staged`;
  - `release_quality=patch_only`;
  - validation flags include `incremental_handoff_affected_scope`,
    `explicit_release_registration_required`, and
    `http_incremental_handoff_reviewed`;
  - selected candidate `dbnum=1112`;
  - handoff manifest hash
    `96a0ea948b9162e2ab199b252b47a138c1a2f79441fa33a2c572a2d32a71b865`.
- Repeating the same POST returned `registration.status=already_exists`.
- `release_quality=complete_visual` override returned HTTP `400` with:
  `incremental handoff cannot register complete_visual`.
- `GET /api/model-version/releases/codex-http-handoff-20260621052235`
  returned the staged release detail with `release_quality=patch_only`.
- `cargo fmt --check` passed.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed using `E:\codex-targets\plant-cli-ducklake-build`.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed using `E:\codex-targets\plant-cli-ducklake-build`.
- `git diff --check` passed for the touched code/docs with existing CRLF
  warnings only.

Next implementation target:

- Centralize the release state machine so staged `patch_only`/quarantined
  handoff releases can be promoted only after baseline, generation, asset, and
  readiness evidence is complete.
- Connect the full safe sequence:
  `prepare-history-replay -> generate model run -> handoff/register -> publish/readiness`.

## Completed Implementation Slice: Release State Machine Safety Gate

Status: completed on 2026-06-21.

Purpose:

- Add a single backend state-machine service for release review and production
  promotion decisions.
- Keep the existing `reconcile` endpoint available for diagnostics, while
  making production promotion go through stricter gates than raw package/index
  consistency.
- Prevent staged `patch_only`, `quarantined_visual`, `degraded_visual`, and
  `non_visual` releases from being marked production-published by accident.

Success criteria:

- Add a `release_state_machine` domain module.
- Add a structured HTTP endpoint:
  `POST /api/model-version/releases/{release_id}/state-machine`.
- The endpoint accepts `action` values:
  `review`, `publish_if_ready`, and `fail_if_unusable`.
- `review` must be side-effect free and return current reconcile/readiness
  evidence.
- `publish_if_ready` may update lifecycle to `published` only when all blockers
  are absent:
  - package/reconcile evidence has no blocking problems;
  - release quality is `complete_visual`;
  - baseline state manifest path and hash exist;
  - generation job id exists unless explicitly disabled for legacy migration;
  - visual releases have release-local mesh asset manifest evidence;
  - component and mesh asset readiness evidence is complete.
- Unsafe promotion requests must return a structured report with
  `transition_allowed=false` and no status mutation.
- `fail_if_unusable` may mark a release `failed` only when blockers exist, and
  must write a status event reason.
- The response must include previous/current lifecycle/status, action,
  transition decision, blockers, warnings, reconcile report, and release events.
- HTTP validation must prove against DB1112 staged handoff evidence:
  - `review` returns no mutation;
  - `publish_if_ready` rejects a staged `patch_only` handoff release and keeps
    it staged;
  - startup route logging includes the endpoint.

Current constraints:

- This slice does not yet build the missing full replay-generated complete
  visual pair.
- This slice intentionally does not change existing read-only compare readiness
  semantics; it gives future orchestration a safer mutation entrypoint.

Validation evidence:

- Added domain module:
  `src/version_management/release_state_machine.rs`.
- Added route:
  `POST /api/model-version/releases/{release_id}/state-machine`.
- Startup route logging includes the new endpoint.
- `review` against DB1112 staged handoff release
  `codex-http-handoff-20260621052235` returned:
  - `transition_allowed=false`;
  - `applied=false`;
  - `current_lifecycle=staged`;
  - blockers include missing baseline state evidence, missing generation job id,
    missing mesh asset evidence, and `release quality is patch_only`.
- `publish_if_ready` against the same release returned:
  - `transition_allowed=false`;
  - `applied=false`;
  - `action_taken=none`;
  - release detail remained `release_lifecycle=staged`,
    `release_status=staged`, `release_quality=patch_only`.
- A new handoff release created after this slice,
  `codex-http-sm-handoff-20260621054048`, now carries
  `generation_job_id=incremental-db1112-896-to-897-20260620T155135644Z`.
- `publish_if_ready` against that new release still returned
  `transition_allowed=false` and kept the release staged; the generation blocker
  was gone, while baseline/asset/quality blockers remained.
- Invalid action `launch_into_orbit` returned HTTP `400`.
- `cargo fmt --check` passed.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed using `E:\codex-targets\plant-cli-ducklake-build`.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed using `E:\codex-targets\plant-cli-ducklake-build`.
- `git diff --check` passed for touched code/docs with existing CRLF warnings
  only.

Next implementation target:

- Wire the safe orchestration sequence so a backend task can run:
  `prepare-history-replay -> bounded generate -> handoff/register -> state-machine review`.
- Then build or restore the missing full baseline/target complete-visual
  releases required for a production two-pane comparison.

## Completed Implementation Slice: Execute History Replay Plan HTTP Runner

Status: completed on 2026-06-21.

Purpose:

- Convert the safe `prepare-history-replay` plan from advisory stdout JSON into
  a structured backend execution entrypoint.
- Prevent UI/backend callers from submitting arbitrary argv while still allowing
  the prepared `baseline_*`, `generate`, and `publish` phases to run through the
  bounded runner.

Success criteria:

- Add structured endpoint:
  `POST /api/model-version/runs/execute-history-replay-plan`.
- The endpoint must read a prior `prepare_history_replay` bounded run status and
  stdout JSON.
- It must reject non-succeeded prepare runs, changed source hashes, missing
  stdout, mismatched source file/project, and unsupported phases.
- It must support only whitelisted phases:
  `baseline_parse`, `baseline_generate`, `baseline_register`, `generate`, and
  `publish`.
- It must validate selected argv against the parsed plan:
  - `generate` must be `incremental-sesno --generate-model --json` with matching
    source DB and sesno range;
  - `publish` must be `model-version publish-history --json` with matching
    release, dbnum, source DB, sesno range, replay Parquet dir, baseline parent,
    and `--materialize-assets`.
- The spawned bounded run must carry the prepare run's source DB hash as the
  expected hash.
- HTTP validation must prove invalid phase, missing prepare run, and a real
  DB1112 `publish` phase launch from existing prepare evidence.

Validation evidence:

- Added route:
  `POST /api/model-version/runs/execute-history-replay-plan`.
- Startup route logging includes the endpoint.
- Invalid phase `bogus` returned HTTP `400`.
- Missing prepare run `missing-prepare-run` returned HTTP `404`.
- DB1112 prepare run `codex-http-history-snapshot-20260621041907` launched
  `phase=publish` as run `codex-http-exec-publish-20260621061200`.
- Launch response included:
  - `kind=history_replay_plan_publish`;
  - `launch_observed=true`;
  - prepare stdout hash
    `d016d4db922e5a0965f3ecf79866f232f5ab6b7175ea8076817fab73d4c335fa`;
  - expected source DB hash
    `5ea0c56bef3030f8a450ffd1c136948f1c1581b20b6f55de79ccf0410766e385`;
  - command argv copied from prepare stdout `publish_argv`.
- The launched bounded run finished with `status=failed`, `exit_code=1`, and
  stderr:
  `historical release source Parquet directory does not exist`.
  This is the expected safe failure because the replay-generated package has not
  been produced yet.
- The failed run still proved source hash protection:
  `source_db_hash_unchanged=true` and matching before/after SHA-256.
- `cargo fmt --check` passed.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed using `E:\codex-targets\plant-cli-ducklake-build`.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed using `E:\codex-targets\plant-cli-ducklake-build`.

Current constraints:

- This slice does not yet run the heavy DB1112 `generate` phase.
- The final two-pane production comparison is still incomplete until a full
  replay-generated target package and baseline/asset/index evidence exist.
- No `cargo test` was run, per repository policy.

Next implementation target:

- Use this endpoint to run the prepared `generate` phase when ready for the
  heavier DB1112 model generation job.
- Then consume the resulting handoff/register evidence through the existing
  incremental handoff and release state-machine endpoints.

## Active Implementation Slice: DB1112 History Replay Generate To Release

Status: active on 2026-06-21; first generate attempt was safely cancelled
because it produced no progress evidence for about 14 minutes.

Purpose:

- Complete the first real DB1112 `791 -> 897` history replay model-generation
  validation using the safe HTTP orchestration path.
- Preserve the architecture boundary proven by Oracle:
  `SourceObservation -> BaselineState -> IncrementEvidence -> GenerationJob -> ReleasePackage -> DuckLake read model`.
- Keep DuckLake out of generation and payload ownership; use it only after a
  release package exists.

Important correction:

- A physical baseline snapshot for `from_sesno=791` is not necessarily the DB
  file that can read history through `to_sesno=897`.
- `prepare-history-replay` now allows `snapshot_id` to prove the baseline while
  `source_db_file` points at the current/history DB file for incremental
  parsing.
- This prevents the backend from trying to read `to_sesno=897` from a
  replacement DB whose latest session is only `791`.

Success criteria:

- A prepare run using
  `snapshot_id=codex-ams1112-physical-791-reuse-20260620` and
  `source_db_file=D:\AVEVA\Projects\E3D2.1\AvevaMarineSample\ams000\ams1112_0001`
  succeeds with `source_mode=physical_snapshot_with_history_source`.
- A generate run launched via
  `POST /api/model-version/runs/execute-history-replay-plan` either:
  - produces non-empty replay Parquet/model output with source hash unchanged;
  - or fails with actionable stdout/stderr/evidence that identifies the next
    backend fix.
- Long-running generate jobs must write heartbeat/stage evidence to
  `task-metrics.json`; a process that consumes CPU without stdout, stderr,
  metrics, or output files is not operationally acceptable for the backend UI.
- Successful output is not auto-published. It must pass through
  `incremental/handoff`, `register`, release state-machine review, and explicit
  DuckLake indexing/materialization.
- Final production readiness requires two release-local model packages and a
  two-pane `/model-version/compare` validation with both sides loading their own
  GLB assets.

Validation rules:

- Do not run `cargo test`.
- Validate `web_server` through HTTP.
- Validate `aios-database` through CLI/JSON/build.
- Stop any isolated validation server or generation process before ending a
  work session unless it has been deliberately handed off as an active
  long-running run.

Current evidence:

- Prepare run `codex-http-history-targetsrc-20260621062500` succeeded with the
  corrected source split.
- Generate run `codex-http-exec-generate-targetsrc-20260621062600` launched with
  the expected argv and source hash guard.
- It was cancelled after about 14 minutes with no metrics file and no replay
  Parquet output.
- The run finalized as `cancelled`, and source DB SHA-256 remained unchanged:
  `70f18c70116f392eae533b75fb8f4043d031a5f049448531cc1dfc43faf7d3c2`.

## Completed Implementation Slice: Incremental Generate Metrics Heartbeat

Status: completed on 2026-06-21.

Purpose:

- Make `incremental-sesno --generate-model` observable from the moment the
  bounded runner starts the child process.
- Prevent another DB1112 history replay run from spending many minutes in an
  opaque phase with no `task-metrics.json`, no stage, and no stale-heartbeat
  enforcement.

Success criteria:

- The CLI writes `task-metrics.json` before entering any potentially long
  source collection, db-meta refresh, SurrealDB persist, tree evidence,
  generation, export, or handoff step.
- The metrics file exposes a useful `stages.generate.progress.stage` such as
  `incremental_sesno_collecting`, `incremental_sesno_persisting`,
  `incremental_sesno_generate_running`, or
  `incremental_sesno_exporting_parquet`.
- Long synchronous phases refresh the metrics file periodically so bounded
  runner stale-heartbeat checks can distinguish progress from a stuck child.
- Validation uses CLI/JSON/build only and does not run `cargo test`.

Evidence:

- Added a no-op-safe `GenerateHeartbeatGuard` in `src/perf_metrics.rs`.
- Instrumented `run_incremental_sesno_once` in `src/main.rs` so source
  collection, db-meta refresh, SurrealDB persist, tree evidence, model
  generation, post-generation export, and handoff each write structured
  generate progress.
- `cargo fmt --check` passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"` passed.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"` passed.
- CLI smoke with `from_sesno=897` / `to_sesno=897` produced
  `target\codex-metrics-smoke\incremental-negative-20260621065446.json` and
  final stage `incremental_sesno_handoff_built`.
- Long-stage smoke against the isolated replay config for DB1112 `791 -> 897`
  produced
  `target\codex-metrics-smoke\incremental-long-cancel-final-20260621065819.json`
  while the child was still running, with stage
  `incremental_sesno_collecting_file`; the process was then stopped by the
  validation script and did not remain running.
- HTTP bounded-run smoke
  `codex-http-exec-generate-metrics-smoke-clean-20260621070645` launched through
  `POST /api/model-version/runs/execute-history-replay-plan`, exposed
  `metrics.stage=incremental_sesno_collecting_file` through
  `GET /api/model-version/runs/{run_id}`, and was cancelled cleanly with
  `source_db_hash_unchanged=true`.

Next requirement:

- Let DB1112 `791 -> 897` generate run to normal completion through the HTTP
  bounded runner now that P0.8 has removed duplicate collection and proven
  per-session progress is exposed through run metrics.
- If the run still spends a long time in collection, use
  `incremental_sesno_collecting_file_progress` to identify the exact slow
  session and refno counts instead of treating collection as a black box.
- If generate passes, continue immediately to package handoff, release
  registration, DuckLake indexing, compare readiness, and two-pane 3D visual
  comparison.

## Active Implementation Slice: Single-Pass Increment Evidence Collection

Status: completed for single-pass collection and per-session progress on
2026-06-21. The broader DB1112 `791 -> 897` generate/release/compare chain is
still pending.

Implemented so far:

- The main `incremental-sesno` CLI/HTTP runner path collects pdms-io operations
  once per file/range and reuses them for persist.
- Public JSON-compatible collection and persist wrappers remain available.
- Build validation passed for `aios-database` and `web_server`.
- CLI DB1112 `896 -> 897` validation showed
  `collect_increment_eles_count_in_verbose_output=1`.
- `pdms-io-fork/src/io.rs` now exposes `IncrementCollectProgress` and
  `collect_increment_eles_with_progress`; the old `collect_increment_eles`
  remains a compatibility wrapper.
- `src/data_interface/sesno_increment.rs` records callback snapshots as
  `incremental_sesno_collecting_file_progress`, including current `sesno`,
  processed/total sessions, refno locations, unique/duplicate refnos, and
  operation count.
- CLI DB1112 `791 -> 897` long-range progress smoke observed precise progress
  before persist:
  `phase=session_locations_collected sesno=792 sessions=0/106 refno_locs=31`.
- HTTP bounded-run progress smoke
  `codex-http-exec-generate-progress-20260621081108` started the same DB1112
  `791 -> 897` generate through
  `POST /api/model-version/runs/execute-history-replay-plan`; the run record
  reached `metrics.stage=incremental_sesno_collecting_file_progress`, the
  metrics detail reported
  `phase=session_locations_collected sesno=793 sessions=1/106 refno_locs=31`,
  and cancellation left `source_db_hash_unchanged=true` with no
  `aios-database` child process remaining.
- `GET /api/model-version/runs/codex-http-exec-generate-progress-20260621081108?project=AvevaMarineSample`
  returned `success=true`, `data.run.status=cancelled`, and
  `data.run.metrics.stage=incremental_sesno_collecting_file_progress`.

Still pending for the larger product goal:

- Run DB1112 `791 -> 897` through the bounded HTTP runner to normal completion.
- Complete the downstream immutable release package, DuckLake read-model/index,
  compare readiness, and two-pane 3D visual comparison.

## Active Architecture Decision: Immutable Version Algebra

Status: confirmed by Oracle browser consult
`e3d-version-ducklake-compact-plan` on 2026-06-21 after MCP Oracle transport
returned `Transport closed`. No API/paid Oracle run was used.

The production model version contract is:

```text
SOID -> BSID -> IEVID -> GJID -> RID -> package_hash
```

Hard rules:

- User-facing version identity is `release_id + package_hash`.
- `sesno`, snapshot ids, DB file paths, mutable output directories, and
  DuckLake transaction/snapshot ids are evidence or storage details, not model
  version identity.
- SurrealDB is an ephemeral generation workspace/cache, not release truth.
- Immutable release packages are the payload truth.
- DuckLake is an append-only projection/read-model for derived graph, query
  acceleration, diff/impact, and audit events. It must be rebuildable from
  release packages and must not be the generation writer, baseline restore
  source, job-state truth, or UI version id.
- Before production publish, add projection freshness/falsification gates:
  same `package_hash` with changed DuckLake diff/index means
  `projection_inconsistent` and blocks production sign-off.

Purpose:

- Remove the duplicate pdms-io operation collection currently performed by
  `collect_pdms_increment_for_file` and `persist_pdms_increment_files`.
- Make DB1112 `791 -> 897` history replay generate operationally tractable
  before continuing to publish/index/compare.
- Keep the architecture boundary unchanged: this is an upstream increment
  evidence/generation preparation fix, not a DuckLake truth-model change.

Success criteria:

- The CLI/HTTP generate path calls `collect_increment_eles` only once for each
  source file and actual sesno range.
- Persist uses the already collected `EleOperationData` instead of reopening
  pdms-io and re-reading the same range.
- Public JSON output remains backward compatible; runtime grouped operations
  are transient or written as an explicit evidence artifact.
- Metrics expose collection progress beyond a single outer stage, preferably
  current session and processed/total session counts.
- DB1112 `791 -> 897` bounded run progresses past
  `incremental_sesno_collecting_file` or, if collection remains long, reports a
  precise per-session bottleneck.
- No `cargo test`; validation uses build, CLI/JSON, and web_server HTTP.

## Current Continuation Update: Oracle + DB1112 Evidence - 2026-06-21

Oracle MCP remains unreliable in this workspace: `mcp__oracle.sessions` returned
`Transport closed` for `e3d-version-ducklake-compact-plan`. The same Oracle
review was recovered through CLI/browser render, and the architecture decision
stands:

```text
SOID -> BSID -> IEVID -> GJID -> RID -> package_hash
```

Hard boundary:

- `release_id + package_hash` is the user-facing model version.
- Immutable Parquet/GLB release packages are payload truth.
- DuckLake is append-only projection/read-model/index/diff/audit only.
- DuckLake must not become generation writer, payload truth, baseline restore
  source, job truth, or UI version id.

New validation evidence:

- DB1112 `896 -> 897` generated successfully with `--generate-model` in about
  50 seconds and produced a handoff package:

```text
release_id=codex-smallrange-896-897-20260621092553-db1112-sesno897-pkg6e2bfaaafe09
package_hash=6e2bfaaafe091aa0ae178420c3e3953dcff1c5d8062f898eca478e4ff04d2c31
geo_instances=163
instances=106
glb_readable_count=6
```

- HTTP `incremental/handoff` registered it as staged `patch_only`.
- DuckLake component/unit/mesh asset indexing succeeded.
- `publish_if_ready` correctly refused to publish it because it is
  `patch_only` and lacks baseline manifest evidence.

Large-range status:

- DB1112 `791 -> 897` now enters the fast current-state path and emits useful
  per-session progress.
- The current bottleneck is session `892`, with `refno_locs=220296`.
- Caching owner children membership as a `HashSet` did not materially change
  completion time; the next optimization must target per-refno current-state
  parsing/checking.

Two-pane evidence:

- Existing published releases
  `codex-ams1112-physical-791-quarantine` and
  `codex-ams1112-physical-897-quarantine` load in `/model-version/compare`.
- Compare readiness is `quarantined_visual`, with both component indexes and
  mesh assets ready, but `production_ready=false`.
- Browser screenshot:

```text
.planning/2026-06-17-ducklake-valv-version-diff/model-version-compare-791-897-oracle-architecture-agent-browser.png
```

This satisfies the current diagnostic/demo goal of seeing two 3D model panes,
but not the final production goal. Production completion still requires a
`complete_visual` release pair and a successful DB1112 `791 -> 897` generation
chain through release package, DuckLake projection, readiness, and browser
validation.

## Current Continuation Update: P0.10 Collector Hardening - 2026-06-21

Status:

- The DB1112 `791 -> 897` pure collection bottleneck is now cleared for the
  diagnostic path.
- `incremental-sesno --no-persist` was added to isolate parse/classification
  cost from SurrealDB persistence.
- Large-session collection now dedupes by refno, parses current elements in
  physical offset order, batch-resolves owner offsets through a full index map
  when owner volume is large, caches owner children as `HashSet`, and avoids
  repeating B-tree lookup for missing owners during owner checking.

Validated evidence:

```text
DB1112 791 -> 897 --no-persist
metrics=target\codex-two-phase-smoke\longrange-no-persist-owner-nosearch-20260621110306.json
exit_code=0
duration_ms=153307
sessions=106
elements=224384
total_changes=223588
delete=220312
```

```text
DB1112 896 -> 897 default persist
metrics=target\codex-two-phase-smoke\smallrange-default-persist-final-20260621110654.json
exit_code=0
duration_ms=5612
pe=169
att=169
```

Current remaining production work:

- Run DB1112 `791 -> 897 --generate-model` on the real persisted path.
- If generation succeeds, process the handoff/register/index-assets/index-units
  flow and state-machine review.
- Produce or validate a `complete_visual` release pair; current 791/897 pair is
  still diagnostic `quarantined_visual`.
- Re-run `/model-version/compare` browser validation against the final pair.

## Current Continuation Update: Oracle MCP v3 Architecture Plan - 2026-06-21

Status:

- The requested Oracle MCP continuation was attempted.
- `mcp__oracle.consult` failed with `Transport closed`.
- A focused Oracle CLI bundle dry-run succeeded at about `150,686` tokens.
- A browser run was attempted as `e3d-ducklake-version-plan-2`, but failed due
  missing ChatGPT login/model selector state in Oracle's private Chrome profile.
- No paid/API Oracle call was started.
- The completed Oracle session `e3d-version-ducklake-compact-plan` was rendered
  and remains the active second-model evidence.

Architecture decision now documented:

```text
docs/plans/2026-06-21-e3d-model-version-ducklake-oracle-mcp-v3.md
```

Current contract:

- `ReleasePackage` is the model-data truth.
- DuckLake is append-only projection/read-model only.
- User-visible model version is `release_id + package_hash`.
- Formal lineage is `SOID -> BSID -> IEVID -> GJID -> RID`.
- `sesno`, source DB path, output directory and DuckLake snapshot id are anchors
  or implementation details, not version identity.

Next production step remains unchanged:

- resume DB1112 `791 -> 897 --generate-model` after the current
  `pe_transform` hardening, then publish/index/review and validate the final
  two-pane compare with production-ready releases.

## Current Continuation Update: pe_transform Hardening Validated - 2026-06-21

Validation:

```text
command=aios-database -c output\AvevaMarineSample\model_versions\replay_configs\codex-http-history-targetsrc-release-20260621062500\DbOption-replay --refresh-transform 1112
run_root=target\codex-transform-refresh\db1112-marker-20260621122842
stage=refresh_pe_transform_dbnums_done
processed=3655
primed=3552
```

Result:

- The previous DB1112 stall around `2280/3975` is cleared.
- The marker passthrough and chunked `pe_transform` write path are sufficient
  to proceed to the real `791 -> 897 --generate-model` validation.
- The full production goal is still active and not complete.

## Current Continuation Update: Full DB1112 Generate + Patch Release Handoff - 2026-06-21

Status:

- DB1112 `791 -> 897 --generate-model` completed on the real persisted path.
- The generated handoff was registered through real `web_server` HTTP/POST.
- The resulting release is staged `patch_only`, not production
  `complete_visual`.

Evidence:

```text
full_generate_metrics=target\codex-full-generate\full-791-897-marker-20260621123126\task-metrics.json
handoff=target\codex-full-generate\full-791-897-marker-20260621123126\handoffs\incremental-db1112-791-to-897-20260621T050159936Z.json
release_id=codex-fullrange-791-897-marker-20260621123126-db1112-sesno897-pkgb509906b4f83-http-fixed
package_hash=b509906b4f83f876cd874266366dcd3cc7237eb0e3312575648a9f72cf0069e5
http_logs=target\codex-web-server\full-generate-handoff-fixed3-202606211350
ducklake_metadata=output\AvevaMarineSample\model_versions_ducklake\metadata.ducklake
```

Validated:

- HTTP handoff registration succeeded with component index:
  `component_count=46469`.
- Unit index succeeded:
  `unit_count=1470`, `member_count=46469`, `unresolved_member_count=42565`.
- Mesh asset index succeeded in non-materializing mode:
  `geo_hash_count=59`, `present_count=58`, `missing_count=1`,
  `glb_readable_count=58`.
- State machine refused production publication, as intended, because baseline
  evidence, complete visual quality, and release-local asset manifest evidence
  are missing.

Implementation checkpoint:

- DuckLake remains a projection/read-model, not model truth.
- ReleasePackage remains truth.
- Long Windows paths are handled by absolute DuckDB paths, DuckLake
  `OVERRIDE_DATA_PATH true`, shortened release storage directories, and a short
  fallback DuckLake store root.

Remaining final goal:

- Build/restore `scene_tree/1112.tree`.
- Repair or formally quarantine the remaining missing mesh asset.
- Produce two `complete_visual` releases and validate `/model-version/compare`
  with two production-ready 3D model panes.

## Current Continuation: Production Completion Audit And Closeout

Status: active.

This continuation keeps the original production-grade target intact. The
validated DB1112 `791 -> 897` handoff is useful evidence, but it is only a
staged `patch_only` release and must not be treated as done.

Production completion still requires authoritative evidence for every gate:

- `scene_tree/1112.tree` and `db_meta_info.json` are present for the DB1112
  release workspace or the absence is explicitly and safely quarantined by the
  release policy.
- Mesh assets referenced by renderable release packages are all present and
  readable, or missing rows are formally quarantined and excluded from the
  visual package with manifest counts proving consistency.
- Two DB1112 releases intended for final comparison are `complete_visual` under
  the release state-machine policy, not merely `patch_only` or diagnostic
  `quarantined_visual`.
- DuckLake remains a rebuildable projection/read-model; immutable
  ReleasePackage artifacts remain the payload truth.
- HTTP validation proves release registration/index/readiness behavior, and a
  browser validation proves `/model-version/compare` loads two release-local 3D
  panes with production-ready releases.

Next steps for this continuation:

1. Inspect the current release catalog and package evidence for DB1112.
2. Close the `scene_tree` and missing mesh gaps through repair, materialization,
   or explicit quarantine logic that the state machine can verify.
3. Produce or promote a pair of release packages only when the evidence supports
   `complete_visual`.
4. Re-run CLI/HTTP/browser validation and update `progress.md` after each
   meaningful step.

## Current Continuation Update: Self-Intersect Mesh Classification

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `repair-missing-meshes` now reports `self_intersecting_inputs` and classifies
  source `PrimExtrusion` wire self-intersections as `self_intersecting_input`.
- Compare readiness treats `self_intersecting_input` and
  `self_intersecting_profile` validation flags as production blockers.
- DB1112 897 remaining missing meshes are classified as:
  `non_renderable_input=8`, `self_intersecting_input=9`,
  `still_missing_hashes=17`.
- DB1112 791 remaining missing meshes are classified as:
  `missing_inst_geo=1`, `non_renderable_input=6`,
  `self_intersecting_input=9`, `still_missing_hashes=16`.
- `cargo fmt --check`, `cargo build --bin aios-database`, and
  `cargo build --bin web_server` passed with only existing pdms-io warnings.

Meaning:

- The backend generation problem is no longer an opaque mesh-generation failure;
  it is a documented source-geometry/quality-contract blocker.
- Current 791/897 releases are still diagnostic `quarantined_visual`, not
  production `complete_visual`.
- Next production work is either exact profile repair, or explicit
  ReleasePackage-level non-visual/degraded sign-off that the state machine and
  compare readiness can enforce.

## Current Continuation Update: Metadata-Driven Quality Flags

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- Release registration/publish now infers validation flags from metadata objects
  named `missing_mesh_repair`, `mesh_repair`, or `repair_missing_meshes`.
- The mapping is intentionally small:
  `still_missing_hashes`, `degraded_fradius_fallback_rows`,
  `self_intersecting_inputs`, `non_renderable_inputs`, and `missing_inst_geo`.
- Compare readiness now blocks production on `non_renderable_input` and
  `missing_inst_geo`, in addition to the earlier self-intersect and degraded
  geometry flags.

Evidence:

- Temporary CLI registration with a DB1112 897 repair summary produced:
  `mesh_missing_rows_quarantined,self_intersecting_input,non_renderable_input`.
- Temporary readiness validation returned `production_ready=false` and listed
  non-renderable, self-intersecting, and quarantined missing mesh blockers.
- Targeted `rustfmt --check`, `aios-database` build, and `web_server` build
  passed. Global `cargo fmt --check` is currently blocked by an unrelated
  formatting diff in `src/web_api/mbd_pipe_api.rs`.

## Current Continuation Update: ReleasePackage Sidecar Quality Evidence

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- Release registration now writes a small `release.json` sidecar at the
  immutable release root.
- `publish-history` rewrites that sidecar after the final `Published` state is
  read back from DuckLake.
- `annotate` rewrites the sidecar after quality/flag changes.

Decision:

- DuckLake remains the release catalog/query projection.
- The immutable ReleasePackage now also carries enough wrapper evidence to be
  inspected outside DuckLake: lifecycle, quality, quality reason, validation
  flags, package hash, row counts, source/baseline/asset hashes.
- The sidecar does not change existing Parquet package hashing; it describes
  the release wrapper, while `package_hash` remains the viewer payload hash.

Evidence:

- `aios-database model-version register --json` against DB1112 release `897`
  wrote
  `target\codex-release-sidecar-smoke\20260621-release-json\releases\codex-sidecar-smoke-897\release.json`.
- The sidecar contains `release_quality=quarantined_visual` and
  `validation_flags=mesh_missing_rows_quarantined,self_intersecting_input,non_renderable_input`.
- Targeted `rustfmt --check`, `aios-database` build, and `web_server` build
  passed with only existing pdms-io warnings.

## Current Continuation Update: Reconcile Sidecar Gate

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `reconcile-release` now requires and validates the ReleasePackage
  `release.json` sidecar.
- The reconcile JSON report now includes `release_sidecar_path`,
  `release_sidecar_exists`, and `release_sidecar_hash`.
- Sidecar fields are checked against the DuckLake release record for identity,
  lifecycle/quality/status, package hash, validation flags, and row counts.

Evidence:

- CLI reconcile with sidecar present:
  `target\codex-release-sidecar-reconcile\20260621-sidecar-gate\reconcile-sidecar-present.out`
  reports `release_sidecar_exists=true` and no sidecar-specific problem.
- CLI reconcile after temporarily deleting the sidecar:
  `target\codex-release-sidecar-reconcile\20260621-sidecar-gate\reconcile-sidecar-missing.out`
  reports `release_sidecar_exists=false` and problem `release sidecar is missing`.
- Targeted `rustfmt --check`, `aios-database` build, and `web_server` build
  passed.

Known validation limitation:

- HTTP smoke was attempted with `web_server` on local ports `3217` and `3218`,
  but the endpoint did not become available within the readiness windows while
  startup was still doing SurrealDB init and E3D increment scanning. Captured
  logs show startup work but no panic.

## Current Continuation Update: Sidecar Sync After State Transitions

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `reconcile_model_release()` now writes `release.json` after an applied
  transition such as `--fail-if-unusable` or `--publish-if-complete`.
- `run_model_release_state_machine()` now writes `release.json` after an
  applied `publish_if_ready` or `fail_if_unusable` transition.

Evidence:

- Temporary CLI release:
  `target\codex-release-sidecar-reconcile\20260621-status-sync`.
- After `reconcile-release --fail-if-unusable`, DuckLake and sidecar both report
  `failed`.
- A second `reconcile-release` reports zero exact sidecar problems; only the
  expected missing mesh asset index problem remains.
- Targeted `rustfmt --check`, `aios-database` build, and `web_server` build
  passed.

Meaning:

- A valid state transition no longer creates a stale sidecar that would make
  the next health check fail.
- This keeps the ReleasePackage sidecar usable as production evidence without
  adding a separate repair command.

## Current Continuation Update: Oracle Architecture Review 2026-06-21

Status: completed and documented; production goal remains active.

Oracle evidence:

- `mcp__oracle.consult` was attempted first but the MCP transport returned
  `Transport closed`.
- Equivalent Oracle browser review completed through the Oracle CLI session
  `e3d-ducklake-architectu-20260621`.
- The rendered answer is saved at
  `target\oracle-e3d-ducklake-architecture-20260621.md`.
- The architecture document was updated:
  `docs\plans\2026-06-21-e3d-model-version-ducklake-oracle-mcp-v3.md`.

Decision reaffirmed:

- `ReleasePackage` is the immutable payload truth.
- DuckLake is a rebuildable projection/read-model for catalog, indexes, diff,
  impact, readiness, and audit.
- SurrealDB remains the generation workspace/cache.
- HTTP GET paths must stay read-only; publish/index/reconcile/repair are POST
  operations.
- User-facing model identity is `release_id + package_hash`, not sesno, output
  directory, or a DuckLake transaction id.

Development plan from the review:

- P0: prevent mistaken production publish by unifying release validation,
  sidecar gates, state-machine publish gates, affected-scope handoff quality
  flags, baseline evidence, and release-local asset evidence.
- P1: make DuckLake projection explicitly rebuildable and add projection
  hashes/events so deleting/rebuilding DuckLake from ReleasePackage produces
  invariant diff results.
- P2: add long-running watcher/replay queue, copy-on-write mesh asset reuse,
  retention policy, and large-scene runtime pagination/tile support.

Meaning:

- DB1112 `791 -> 897` remains the right validation case.
- Current 791/897 release artifacts are still diagnostic/quarantined, not final
  `complete_visual` production releases.
- The next implementation should close P0 gates before attempting the final
  two-pane production visual comparison.

## Current Continuation Update: Readiness Baseline Gate

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `release_readiness()` now treats missing baseline state manifest path/hash as
  a production problem for `complete_visual` releases.
- Non-`complete_visual` diagnostic releases still report the same missing
  baseline evidence as a warning.

Why:

- The state machine already requires baseline evidence by default before
  production publish.
- Pair readiness previously kept missing baseline evidence as a warning, so a
  wrongly annotated `complete_visual` release could get closer to
  `production_ready` than the state-machine policy allows.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Temporary CLI smoke:
  `target\codex-readiness-baseline-gate\20260621`.
- The smoke registered `codex-baseline-gate-complete-897` as
  `complete_visual` without baseline evidence and verified
  `validate-compare-readiness --json` returns `production_ready=false` with
  `release has no baseline state manifest evidence` in both release problem
  lists.

Self review:

- This is the smallest useful P0 fix: no new validator abstraction, just one
  existing readiness branch aligned with existing state-machine policy.
- Full HTTP smoke was not repeated because this slice changes the shared
  DuckLake readiness core and `web_server` build covers API integration; local
  web_server startup is still slow/noisy from unrelated initialization.

## Current Continuation Update: Index-Assets Sidecar Sync

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `index_model_release_mesh_assets()` now rewrites `release.json` after the
  DuckLake store updates `asset_manifest_path` and `asset_manifest_hash`.

Why:

- Explicit `model-version index-assets` and HTTP `POST /index-assets` are valid
  production evidence operations.
- Before this fix, they could update DuckLake release metadata while leaving
  the ReleasePackage sidecar with stale asset evidence.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\model_release.rs src\version_management\ducklake_store.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Temporary CLI smoke:
  `target\codex-index-assets-sidecar-sync\20260621`.
- The smoke registered `codex-index-assets-sidecar-sync-1112`, ran
  `model-version index-assets`, then `reconcile-release`.
- Reconcile reported zero sidecar-specific problems, and
  `release.json.asset_manifest_hash` matched the DuckLake release row.

Self review:

- The fix is intentionally one wrapper-level refresh, not a new validator
  layer.
- The remaining reconcile problem in the smoke is expected:
  non-materialized mesh assets are not release-local.

## Current Continuation Update: Compare Readiness Sidecar Gate

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `release_readiness()` now resolves the immutable ReleasePackage root and
  validates `release.json` before allowing pair readiness to move toward
  production-ready.
- Missing sidecar evidence is reported as a problem:
  `release sidecar is missing: <path>`.

Why:

- Oracle review reinforced that ReleasePackage is the durable truth and
  DuckLake is a rebuildable query projection.
- The compare-readiness GET endpoint is the gate behind the two-pane model
  comparison UI, so it must not accept a catalog row whose ReleasePackage
  sidecar is missing or drifted.
- This aligns readiness with `reconcile-release` and the state-machine publish
  policy without adding another validation layer.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Temporary CLI smoke:
  `target\codex-readiness-sidecar-gate\20260621`.
- The smoke registered `codex-readiness-sidecar-gate-1112`, deleted its
  generated `release.json`, and verified `validate-compare-readiness --json`
  returns `production_ready=false` with `release sidecar is missing:` in both
  release problem lists.

Self review:

- This is intentionally a tiny readiness-core change; it reuses the existing
  sidecar validator instead of introducing `release_validator.rs`.
- Full HTTP smoke is still pending for the final end-to-end two-pane visual
  run; this slice is covered through the shared DuckLake readiness core plus
  a successful `web_server` build.

## Current Continuation Update: Sidecar Evidence Field Gate

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `validate_release_sidecar()` now checks the ReleasePackage evidence fields
  that `release.json` already writes:
  `derivation_type`, `generation_job_id`, package dirs, source manifest
  path/hash, baseline manifest path/hash, and asset manifest path/hash.

Why:

- ReleasePackage is the truth, and DuckLake is a projection.
- The previous sidecar validation caught missing sidecars, package hash drift,
  lifecycle/quality drift, flags, and row-count drift, but did not verify the
  exact source/baseline/asset evidence fields used for production readiness.
- This closes that drift path without adding a new validator abstraction.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `git diff --check -- src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- Temporary CLI smoke:
  `target\codex-sidecar-evidence-gate\20260621`.
- The smoke registered `codex-sidecar-evidence-gate-1112`, tampered
  `release.json.source_manifest_hash`, and verified
  `validate-compare-readiness --json` returns `production_ready=false` with
  `release sidecar source_manifest_hash mismatch:` in both release problem
  lists.

Self review:

- This is the smallest useful evidence fix: compare fields already present in
  `release.json`.
- Full DuckLake projection rebuild remains a larger P1 item; this step only
  prevents stale sidecar evidence from being accepted as production-ready.

## Current Continuation Update: Evidence File Hash Gate

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `validate_release_sidecar()` now verifies the actual evidence files declared
  by the release row:
  `source_manifest`, `baseline_state_manifest`, and `asset_manifest`.
- If a path exists without a hash, a hash exists without a path, the file is
  missing, or the file sha256 differs from the catalog hash, readiness/reconcile
  records a problem.

Why:

- The previous slice ensured sidecar fields match the DuckLake release row.
- That still allowed a weaker state where the row and sidecar agree, but the
  evidence file they point to is gone or has changed.
- Two-pane production compare needs evidence that can be re-read, not only
  remembered strings.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Temporary CLI smoke:
  `target\codex-evidence-file-gate\20260621`.
- The smoke registered `codex-evidence-file-gate-1112`, deleted the
  release-local `manifest.json`, and verified
  `validate-compare-readiness --json` returns `production_ready=false` with
  `release evidence source_manifest is missing:` in both release problem lists.

Self review:

- This is a small reusable helper inside the existing validator; no new table,
  endpoint, or command was added.
- Full browser/two-pane validation remains pending until a complete_visual pair
  has intact package, asset, baseline, and projection evidence.

## Current Continuation Update: Release-Local Source Manifest Evidence

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `register_model_release()` now stores `source_manifest_path/hash` from the
  immutable ReleasePackage `manifest.json`.
- `source_package_dir` still records the original source parquet directory for
  traceability.

Why:

- After adding evidence-file validation, source manifest evidence became a
  production gate.
- If that evidence points at the original export directory, a normal cleanup
  can break an otherwise self-contained ReleasePackage.
- The durable evidence must live under the release package itself.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Temporary CLI smoke:
  `target\codex-release-local-source-manifest\20260621`.
- The smoke registered `codex-release-local-source-manifest-1112` from a copied
  source package, verified `release.json.source_manifest_path` points to the
  release-local `parquet\1112\manifest.json`, deleted the copied source
  directory, and confirmed readiness did not report any `source_manifest`
  problem.
- The same smoke then deleted the release-local manifest and confirmed
  readiness reports `release evidence source_manifest is missing:`.

Self review:

- This is one-line production hardening in registration; no migration was added
  for old releases.
- Existing old releases with external `source_manifest_path` should be
  re-registered or reconciled after rewriting sidecar/catalog evidence before
  final production sign-off.

## Current Continuation Update: Source Manifest Release-Local Gate

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `validate_release_sidecar()` now rejects `source_manifest_path` evidence
  outside the release's `immutable_package_dir`.
- This closes the remaining gap where an old or hand-mutated release could point
  to an external manifest file that exists and has the expected hash.

Why:

- Production visual compare must be reproducible from the immutable
  ReleasePackage.
- A correct hash on an external source manifest is not enough, because that
  evidence can disappear or drift independently of the release package.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Temporary CLI smoke:
  `target\codex-source-manifest-release-local-gate\20260621`.
- The smoke registered `codex-source-manifest-local-gate-1112`, copied the
  release-local manifest to `external-source`, then rewrote both sidecar and
  DuckLake catalog `source_manifest_path` to the external file. Readiness
  returned `production_ready=false` and reported
  `release evidence source_manifest is not release-local:` for both sides,
  without a missing-file or hash-mismatch problem.

Self review:

- This is deliberately narrower than a migration. Existing releases with
  external manifest evidence now fail readiness until they are re-registered or
  have release-local source manifest evidence restored.

## Current Continuation Update: Index-Assets Repair Hint Alignment

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `compare-readiness` now recommends
  `aios-database model-version index-assets --release-id <id> --materialize`.
- The message previously used `--materialize-assets`, which belongs to
  `publish-history`, not `index-assets`.

Why:

- This is small, but it is directly on the operator repair path for missing or
  non-release-local GLB assets before two-pane runtime-scene validation.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `aios-database model-version index-assets --help` shows `--materialize` and
  no `--materialize-assets`.
- `validate-compare-readiness` on
  `target\codex-source-manifest-release-local-gate\20260621` returned
  recommended actions containing `index-assets --materialize` twice and
  `index-assets --materialize-assets` zero times.

Self review:

- This does not repair DB1112 missing meshes by itself; it removes a bad
  instruction from the production repair loop.

## Current Continuation Update: Reconcile Evidence Repair

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `reconcile-release` now restores a missing `release.json` sidecar even when no
  lifecycle status transition is needed.
- `reconcile-release` now repairs old `source_manifest_path/hash` evidence to
  the release-local `immutable_package_dir\manifest.json`.
- `index-assets` now stores `asset_manifest_hash` as the sha256 of
  `mesh_assets_manifest.json`; `asset_index_hash` remains in the mesh asset
  index stats.

Why:

- New readiness gates correctly require evidence files to be readable and
  hashed, but old DB1112 releases were registered before those fields were
  release-local.
- The explicit repair command should make old releases auditable without
  pretending quarantined visual quality is production-ready.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs src\version_management\model_release.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- CLI `reconcile-release` repaired both
  `codex-ams1112-physical-791-quarantine` and
  `codex-ams1112-physical-897-quarantine`; both now have release-local
  `source_manifest_path`, existing sidecars, and zero reconcile problems.
- CLI and HTTP `compare-readiness` now agree for 791 -> 897:
  `mesh_assets_ready=true`, `both_published=true`, `production_ready=false`.
  Remaining blockers are only `quarantined_visual` / missing mesh quarantine
  quality flags and 791 `spec_info_fallback`.

Self review:

- This is an evidence repair, not a quality promotion. The pair still must not
  be marked `complete_visual` until quarantined source geometry/spec fallback is
  repaired or signed off by an explicit visual contract.

## Current Continuation Update: Oracle MCP Follow-up and Fresh Compare Link Validation

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- The compare page now accepts canonical query parameters
  `from_release_id` / `to_release_id` as well as legacy `from` / `to`.
- This fixes fresh links to
  `/model-version/compare?from_release_id=...&to_release_id=...` so the two
  release selectors open directly on the requested pair.
- The architecture plan now records the Oracle MCP attempt, DuckLake boundary,
  edge cases, and development phases.

Oracle MCP:

- `mcp__oracle.consult` dry-run passed with browser mode, `gpt-5.5-pro`, five
  attachments, and an estimated 124,339 tokens.
- The real browser consult failed because Oracle's private Chrome profile at
  `C:\Users\dpc\.oracle\browser-profile` is not signed in to ChatGPT.
- No API-mode Oracle run was started, because API mode may incur usage cost.

Architecture conclusion:

- DuckLake/DuckDB-backed metadata is a good fit for the query catalog, snapshot
  inspection, and delta analysis layer.
- The canonical production boundary remains the immutable ReleasePackage, not
  the DuckLake catalog alone.
- Raw E3D DB files are provenance, parsed parquet/tree snapshots are data state,
  mesh/GLB files are derived visual assets, and the release package manifest is
  the version commitment.

Evidence:

- `rustfmt --edition 2024 --check src\web_api\model_version_api.rs` passed.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build`
  passed with only existing pdms-io warnings.
- Temporary `web_server` on port 3997 loaded the fresh canonical compare URL:
  `from_release_id=codex-ams1112-physical-791-quarantine` and
  `to_release_id=codex-ams1112-physical-897-quarantine`.
- Browser state:
  `from=codex-ams1112-physical-791-quarantine`,
  `to=codex-ams1112-physical-897-quarantine`,
  `readiness=quarantined_visual`, `not production ready`,
  `mesh_missing_rows_quarantined`, `spec_info_fallback`, `iframes=2`.
- Iframe evidence:
  791 loaded `components 2000/26117`, `geometries 2288/2288`, three visible
  canvases; 897 loaded `components 2000/28651`, `geometries 2041/2041`, three
  visible canvases.
- Screenshot:
  `.planning\2026-06-17-ducklake-valv-version-diff\model-version-compare-after-evidence-repair-agent-browser.png`.

Self review:

- The UI now satisfies the current two-pane visual comparison requirement for
  the DB1112 quarantine pair.
- The releases remain diagnostic/quarantined, not production-ready, until the
  true visual-quality blockers are resolved or explicitly signed off.

## Current Continuation Update: Missing Mesh Repair Immutable Boundary

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `repair-missing-meshes` now refuses non-dry-run writes under
  `model_versions/releases/<release-id>/...` unless
  `AIOS_ALLOW_RELEASE_PACKAGE_MESH_REPAIR=1` is set.
- This preserves the immutable ReleasePackage boundary. Repair output should go
  to a scratch mesh root and then into a newly registered/published release, not
  into an existing release package.

DB1112 findings:

- Existing missing mesh reports:
  - 791: `requested=22`, `missing_inst_geo=19`, `non_renderable=1`.
  - 897: `requested=23`, `missing_inst_geo=20`, `non_renderable=1`.
- Plain repair against the existing release mesh roots attempted two hashes per
  release but generated `0`; both attempted hashes ended as
  `generation_failed_bad`.
- Scratch repair with `AIOS_CSG_ALLOW_DEGRADED_PROFILE_FALLBACK=1` generated two
  GLBs per release and recorded `degraded_fradius_fallback_rows=2`.
- Therefore these two rows are only suitable for a future `degraded_visual`
  package with `degraded_geometry_fallback` evidence, not for `complete_visual`.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\missing_mesh_repair.rs`
  passed after formatting.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build`
  passed with only existing pdms-io warnings.
- Guard smoke:
  release-local non-dry-run returned `exit_code=1` with
  `refusing to write missing-mesh repair into immutable ReleasePackage path`.
- Scratch smoke:
  non-dry-run returned `exit_code=0` and kept the normal classification path.

Self review:

- This does not make 791/897 production-ready; it prevents a more dangerous
  failure mode where repair tooling silently mutates the very ReleasePackage
  that readiness is supposed to audit.

## Current Continuation Update: Spec Info Fallback Readiness Evidence

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `compare-readiness` now distinguishes quantified `spec_info_fallback_count`
  evidence from unquantified fallback risk.
- A release with `spec_info_fallback_count=null` and
  `spec_info_fallback_unquantified` now reports:
  `release has unquantified spec_info fallback risk; quantify or regenerate before complete_visual production comparison`.

Evidence:

- `rustfmt --edition 2024 --check src\version_management\ducklake_store.rs`
  passed.
- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save" --target-dir E:\codex-targets\plant-cli-ducklake-build`
  passed with only existing pdms-io warnings.
- CLI readiness evidence:
  `target\codex-db1112-791-897-readiness-after-spec-message-20260621.json`.
- HTTP readiness evidence:
  `target\codex-spec-message-web-20260621\compare-readiness.json`.
- HTTP result:
  `classification=quarantined_visual`, `production_ready=false`,
  `mesh_assets_ready=true`, `from_problem_count=2`, `to_problem_count=1`,
  `has_unquantified_spec_info_message=true`.

Self review:

- This improves operator evidence only. It intentionally does not promote the
  DB1112 791/897 pair to `complete_visual` or `production_ready`.

## Current Continuation Update: Generated Spec Info Fallback Count

Status: implemented and validated on 2026-06-21; production goal remains active.

What changed:

- `export_dbnum_instances_parquet` now writes `spec_info_fallback_count` and
  `spec_info_validation` into both package manifests.
- `register_model_release` now reads fallback count from the generated package
  manifest when CLI/API input and metadata do not provide it.
- When the count is greater than zero, release registration appends
  `spec_info_fallback`.
- The parquet export CLI summary now prints the fallback count.

Evidence:

- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- `cargo build --bin web_server --features "web_server,model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Real DB1112 export wrote:
  `target\codex-db1112-spec-info-manifest-export-20260621\1112\manifest.json`.
- Export assertion:
  `spec_info_fallback_count=40072`,
  `instance_fallback_rows=40072`, `tubing_fallback_rows=0`,
  `instances_rows=47490`, `geo_instances_rows=163`.
- Real DB1112 register smoke wrote:
  `target\codex-db1112-spec-info-real-register-20260621\assertion.json`,
  with sidecar count `40072` and flag `spec_info_fallback`.

Self review:

- This closes the forward path for generated package evidence. It does not
  mutate existing DB1112 quarantine releases; those need re-registration from
  matching historical evidence before their unquantified flag can be removed.

## Current Continuation Update: Legacy Spec Info Audit Gate

Status: implemented and validated on 2026-06-22; production goal remains active.

What changed:

- Added `model-version audit-spec-info`.
- The command reads an existing release package without mutating sidecar/catalog
  and reports:
  - manifest `spec_info_fallback_count`
  - manifest `spec_info_validation.fallback_count`
  - `instances.parquet` rows and zero `spec_value` rows
  - `tubings.parquet` rows and zero `spec_value` rows
  - combined legacy zero count

Evidence:

- `cargo build --bin aios-database --features "model-version-ducklake,surreal-save"`
  passed with only existing pdms-io warnings.
- Audit output:
  `target\codex-db1112-spec-info-legacy-audit-20260622\summary.json`.
- 791 result:
  `manifest_count=null`, `legacy_zero=26173`.
- 897 result:
  `manifest_count=null`, `legacy_zero=28693`.

Self review:

- This does not promote either release. It exposes that 897 also lacks clean
  spec-info evidence, even though its sidecar did not previously carry the
  `spec_info_fallback` flag.

## Current Continuation Update: Manifest-Level Spec Info Evidence Gate

Status: implemented and validated on 2026-06-22; production goal remains active.

What changed:

- `compare-readiness` now reads the immutable package `manifest.json` for
  generated spec-info evidence.
- Readiness evidence exposes:
  - `spec_info_manifest_evidence_present`
  - `spec_info_manifest_fallback_count`
- A `complete_visual` release whose package manifest lacks generated
  spec-info evidence is now blocked with a readiness problem.
- Quarantined/degraded releases with the same missing evidence keep their
  current classification but surface an operator warning.

DB1112 result:

- `codex-ams1112-physical-791-quarantine` and
  `codex-ams1112-physical-897-quarantine` both lack generated manifest
  evidence, so both remain `quarantined_visual`.
- This is intentional: legacy parquet audit showed large zero-spec counts in
  both packages, and HTTP readiness should not scan parquet on page load.

Evidence:

- CLI readiness:
  `target\codex-db1112-readiness-spec-manifest-gate-20260622.json`.
- HTTP readiness:
  `target\codex-readiness-spec-manifest-web-20260622\compare-readiness.json`.
- Both paths report `classification=quarantined_visual`,
  `production_ready=false`, and manifest evidence absent on both sides.

Self review:

- The architecture line is now clearer: DuckLake indexes release state and
  metadata; immutable ReleasePackage manifests carry generated model-data
  quality evidence. This avoids expensive UI-time parquet scans and avoids
  silently mutating historical packages.

## Current Continuation Update: ReleasePackage File Integrity Gate

Status: implemented and validated on 2026-06-22; production goal remains active.

What changed:

- `reconcile-release` and `compare-readiness` now verify the registered
  `model_release_files` catalog against the immutable ReleasePackage payload.
- The gate resolves file paths from `immutable_package_dir + relative_path`,
  rejects unsafe relative paths, checks required file presence, checks byte
  count, checks SHA-256, and verifies the catalog file set still matches
  `release.package_hash`.

Evidence:

- Temporary isolated DuckLake/release root:
  `target\codex-package-file-gate-20260622`.
- A smoke release was registered, then its release-local
  `instances.parquet` was modified by one byte.
- Both `reconcile-release --json` and
  `validate-compare-readiness --json` reported `release file bytes mismatch`
  and `release file sha256 mismatch`.
- Real DB1112 791/897 readiness remained clean for this new gate:
  `target\codex-package-file-gate-real-readiness-20260622.json`.
- HTTP readiness through a rebuilt `web_server` also remained clean:
  `target\codex-package-file-gate-http-20260622b\compare-readiness.json`.

Self review:

- This is a small production hardening step, not a quality promotion. It keeps
  DuckLake as rebuildable projection/read-model and makes the immutable package
  payload harder to silently drift.

## Current Continuation Update: CompleteVisual Validation Flag Publish Gate

Status: implemented and validated on 2026-06-22; production goal remains active.

Oracle checkpoint:

- Reused completed Oracle session `e3d-ducklake-architectu-20260621`.
- The advisory conclusion remains aligned with the implementation boundary:
  immutable ReleasePackage is truth; DuckLake is a rebuildable projection; publish
  must be centralized behind state-machine/reconcile gates.

What changed:

- `reconcile-release --publish-if-complete` now requires
  `release_quality=complete_visual`.
- Any `complete_visual` release, or any reconcile run with
  `publish_if_complete=true`, now turns release validation flags into blocking
  problems.
- This closes the manual-annotation bypass where a release carrying quarantine
  flags could be marked `complete_visual` and then treated as publishable by a
  reconcile/state-machine path.

Evidence:

- Isolated DuckLake smoke:
  `target\codex-complete-visual-flag-gate-20260622-005325`.
- Complete-visual-with-flag release stayed staged:
  `flagged-reconcile.json`, `publishable=false`, `applied=false`.
- Non-complete release under `publish_if_complete` stayed staged:
  `quality-reconcile.json`, `publishable=false`, `applied=false`.
- Real DB1112 CLI regression:
  `target\codex-complete-visual-flag-gate-real-20260622`.
- Real DB1112 HTTP regression through `web_server`:
  `target\codex-complete-visual-flag-gate-http-20260622`.
- Isolated HTTP state-machine smoke:
  `target\codex-state-machine-flag-gate-http-20260622-010754`.
  It registered a temporary `complete_visual` release with
  `mesh_missing_rows_quarantined`, then `publish_if_ready` returned
  `transition_allowed=false`, `applied=false`, and left the release staged.

Self review:

- This is a release safety gate only. It does not promote 791/897. They remain
  `quarantined_visual`, with `production_ready=false`, until the underlying
  model quality blockers are repaired and new complete visual packages are
  generated.

## Current Continuation Update: Current Two-Pane 3D Compare Regression

Status: validated on 2026-06-22; production goal remains active.

What was verified:

- A temporary `web_server` was started on `http://127.0.0.1:4026` with
  `db_options\DbOption-codex-live-view.toml`.
- The 791/897 compare-readiness endpoint returned
  `classification=quarantined_visual`, `both_published=true`,
  `both_complete_visual=false`, `component_indexes_ready=true`,
  `mesh_assets_ready=true`, and `production_ready=false`.
- The compare page rendered the two selected releases, two model panes, the
  readiness warning, and the diff table.
- The 791 standalone viewer reported `canvasCount=3`,
  `geometries=2288/2288`, `failed=0`.
- The 897 standalone viewer reported `canvasCount=3`,
  `geometries=2041/2041`, `failed=0`.

Evidence:

- API output:
  `target\codex-current-compare-ui-20260622\compare-readiness.json`.
- Runtime scene samples:
  `target\codex-current-compare-ui-20260622\runtime-scene-791-sample.json`,
  `target\codex-current-compare-ui-20260622\runtime-scene-897-sample.json`.
- Browser screenshots:
  `.planning\2026-06-17-ducklake-valv-version-diff\model-version-compare-current-791-897-20260622-agent-browser.png`,
  `.planning\2026-06-17-ducklake-valv-version-diff\release-viewer-791-20260622.png`,
  `.planning\2026-06-17-ducklake-valv-version-diff\release-viewer-897-20260622.png`.

Self review:

- The diagnostic UI path is healthy enough for investigation and demo.
- This does not satisfy the final Done definition: the releases are still
  `quarantined_visual`, so production comparison remains blocked until
  complete visual packages are regenerated and pass readiness.

## Current Continuation Update: Builtin/Sentinel Geo Hash Mesh-Gate Fix

Status: implemented and validated on 2026-06-22; production goal remains active.

What changed:

- External mesh checks now treat `geo_hash` `0`, `1`, `2`, and `3` as
  builtin/sentinel geometry rather than missing external GLB files.
- The fix covers Parquet missing-mesh export, DuckLake mesh asset indexing,
  web_server deploy validation mesh sampling, and repair-missing-meshes report
  ingestion.

Evidence:

- 897 historical report had `geo_hash=0` for 169 rows. After applying the new
  external-mesh filter, the actionable missing set is `22 hashes / 39 rows`
  instead of `23 hashes / 208 rows`.
- CLI dry-run:
  `target\codex-builtin-geo-hash-fix-20260622\repair-897-dry-run.json`
  reports `requested_hashes=23`, `row_count=22`, `has_zero_row=false`.
- HTTP/POST web_server validation:
  `target\codex-builtin-geo-hash-fix-20260622\web-auth\deploy-validation-quicktest-7997-8080.json`
  reports `success=true` and `mesh_refs_sample_7997=pass`.
- Readiness regression:
  `target\codex-builtin-geo-hash-fix-20260622\compare-readiness-791-897.json`
  remains `classification=quarantined_visual`, `production_ready=false`.

Self review:

- This removes a real false-positive blocker from future regeneration without
  weakening the release gate or mutating historical packages.
- Final Done still requires regenerating complete visual 791/897 packages after
  repairing or explicitly classifying the remaining source-geometry blockers.

## Current Continuation Update: Spec Info Fallback Quantification

Status: implemented and validated on 2026-06-22; production goal remains active.

What changed:

- `annotate --spec-info-fallback-count` now turns an explicit count into
  consistent release flags: it removes `spec_info_fallback_unquantified` and
  keeps `spec_info_fallback` when the count is non-zero.
- 791/897 were annotated with read-only audit counts from their immutable
  package parquet files. Release quality remains `quarantined_visual`.

Evidence:

- `audit-spec-info` for 791:
  `legacy_zero_spec_value_count=26173`, `instances=26117/26117`,
  `tubings=56/56`.
- `audit-spec-info` for 897:
  `legacy_zero_spec_value_count=28693`, `instances=28651/28651`,
  `tubings=42/42`.
- Annotation JSON:
  `target\codex-builtin-geo-hash-fix-20260622\annotate-spec-info-791.json`,
  `target\codex-builtin-geo-hash-fix-20260622\annotate-spec-info-897.json`.
- Readiness regression:
  `target\codex-builtin-geo-hash-fix-20260622\compare-readiness-791-897-after-spec-annotation.json`
  remains `classification=quarantined_visual`, `production_ready=false`, with
  quantified spec fallback blockers.

Self review:

- This is evidence hygiene, not a production promotion.
- The audit proves both historical packages have all instance/tubing
  `spec_value` rows at zero, so the next real fix is regenerating spec_info in
  the model package path.

## Current Continuation Update: Spec Info Generation Repair

Status: implemented and validated on 2026-06-22; production goal remains active.

What changed:

- SITE name classification now covers AvevaMarineSample DB1112 tokens:
  `CIVI/CIVIL/ARCH`, `STRU/STRUCT`, `ELEC`, and `DIANQI/电气`, while preserving
  existing `PIPE/ELEC/INST/HVAC` codes.
- `spec_info_1112.parquet` now includes SITE rows, so deeper descendants can
  inherit a SITE-level professional code through the owner chain.
- Parquet export resolves zero raw `spec_value` by checking self, direct owner,
  and TreeIndex ancestors up to depth 64.

Evidence:

- Scratch E2E export:
  `target\codex-spec-info-site-ancestor-fix-20260622\parquet\1112`.
- Distribution report:
  `target\codex-spec-info-site-ancestor-fix-20260622\spec-distribution.txt`.
- `spec_info_1112.parquet` now has `917` rows, with `635` non-zero rows.
- `instances.parquet` now has `35258/47490` non-zero `spec_value` rows.
- Manifest fallback is now `12232`, down from an all-zero historical 791 package
  and from `38975` after SITE-token-only mapping.

Self review:

- This is the first direct fix to the spec_info generation path.
- Remaining fallback is tied to unmapped/non-production SITE names such as
  metadata, missing-elements, and issue-summary sites; those should stay explicit
  fallback until a real business code table or UDA major source is connected.
- Historical 791/897 packages remain immutable and quarantined; regenerate new
  candidate packages before attempting complete_visual publication.

## Current Continuation Update: Oracle MCP and DuckLake Architecture Review

Status: documented on 2026-06-22; production goal remains active.

What was attempted:

- Oracle skill instructions were loaded and `npx -y @steipete/oracle --help`
  was executed.
- Oracle MCP dry-run was reduced from about `384,976` tokens to about
  `92,934` tokens with the focused version-management context.
- The live browser Oracle consult failed on a ChatGPT/Cloudflare challenge.
- No Oracle API run was started, to avoid silently switching to a metered path.

Architecture decision:

- Continue using DuckLake/DuckDB/Parquet for model-version catalog and query
  projection.
- Keep immutable `ReleasePackage` as the source of truth.
- Keep release lifecycle/quality gates in project code, not as implicit
  DuckLake snapshot semantics.
- Treat DuckLake indexes (`model_releases`, `component_snapshots`,
  `model_release_mesh_assets`, `unit_versions`) as rebuildable from release
  packages and sidecars.

Documented:

- `docs/plans/2026-06-21-e3d-model-version-ducklake-oracle-mcp-v3.md`
  now includes section `13.31 Oracle MCP Follow-up and DuckLake Version
  Architecture`.
- The section covers architecture, file structure, version data model, edge
  cases, development sequence, validation strategy, and review conclusion.

Next implementation step:

- Use the repaired spec_info and builtin/sentinel mesh handling to regenerate
  DB1112 791/897 candidate packages in isolated replay/baseline workspaces, then
  rerun publish/index/readiness and two-pane UI validation.

## Current Continuation Update: DB1112 791/897 Candidate Regeneration Audit

Status: validated on 2026-06-22; production goal remains active.

What was done:

- Existing 791->897 replay package was validated with
  `validate-history-replay --allow-patch-only --json`.
- DB1112 physical 791 and 897 baselines were re-exported through the fixed
  Parquet/spec_info path.
- 791 baseline was missing `scene_tree/1112.tree`; `--gen-indextree 1112`
  generated it from the isolated physical baseline workspace.
- 791 was re-exported after the tree repair.
- Missing mesh repair was dry-run using each baseline's own Surreal namespace.

Evidence:

- `target\codex-791-897-candidate-audit-20260622\validate-history-replay-existing.json`
- `target\codex-regenerate-791-897-20260622\summary.json`
- `target\codex-regenerate-791-897-20260622\gen-indextree-791.log`
- `target\codex-regenerate-791-897-20260622\export-791-after-tree.log`
- `target\codex-regenerate-791-897-20260622\export-897.log`
- `target\codex-regenerate-791-897-20260622\repair-791-after-tree-baseline-dry-run.json`
- `target\codex-regenerate-791-897-20260622\repair-897-baseline-dry-run.json`

Results:

- Existing replay workdir remains not publishable:
  `classification=missing_mesh_assets`, `ready_for_publish=false`.
- 791 after tree repair:
  `spec_info_rows=847`, `instances=47698`, `geo_instances=31292`,
  `spec_info_fallback_count=11601`, `render_missing_geo_hashes=16`.
- 897 regenerated:
  `spec_info_rows=917`, `instances=52020`, `geo_instances=28704`,
  `spec_info_fallback_count=15032`, `render_missing_geo_hashes=16`.
- Mesh dry-run with correct namespaces:
  791 has `6` non-renderable, `9` self-intersecting, `1` dry-run eligible hash.
  897 has `7` non-renderable and `9` self-intersecting hashes.

Self review:

- This made the requested model-generation path more true: 791 had a real
  backend evidence bug, the missing tree artifact, and it is now repaired in
  the isolated baseline workspace.
- The new packages are not registered or published because they still have
  render-required missing mesh hashes and non-zero spec_info fallback.
- Final Done remains open.
