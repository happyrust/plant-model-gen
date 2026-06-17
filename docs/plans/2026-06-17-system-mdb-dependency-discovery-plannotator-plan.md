# System-Library MDB Dependency Discovery Development Plan

Date: 2026-06-17
Scope: `plant-model-gen-cata-closure` feature `019-system-mdb-dependency-discovery`
Planner: Plannotator-assisted draft

## Plannotator Status

`plannotator annotate specs/019-system-mdb-dependency-discovery/plan.md --json` was started from the repository root and resolved the target plan file, but it did not produce JSON review output before entering a long wait. Treat this document as a Plannotator-assisted working plan, not as an approved Plannotator gate result.

## Goal

Make MDB-name quick deploy resolve E3D dependency project paths from parsed system-library facts before site creation. A deploy request that provides `mbd_name` plus `search_roots`, `project_path`, or explicit `projects[]` should discover candidate projects, parse supported system libraries, locate every MDB member DB, and proceed only when the requested MDB is complete and unambiguous.

## Facts

- Current feature docs live under `specs/019-system-mdb-dependency-discovery/`.
- System-library sources for MDB discovery are `SYST`, `GLOB`, and `GLB`.
- Directory scanning only finds candidate roots and DB file inventory; it is not dependency proof.
- `src/data_interface/mdb_candidates.rs` owns system-library parsing, MDB candidate enumeration, member DB locate status, and source evidence.
- `src/web_server/managed_project_sites.rs` owns quick-deploy request normalization, candidate selection, target DB selection, and fail-fast behavior before site creation.
- `ui/admin/src/views/SitesView.vue` already has MDB quick deploy mode that submits `mbd_name` and `search_roots`.
- Repository validation rules prohibit `cargo test` for `web_server`; use static checks, running-service HTTP/POST, CLI/json, and artifact inspection.

## Suspected Failure Modes

1. Quick deploy falls back to a directory-name guess when system libraries are missing or fail to parse.
2. Matching MDB members are incomplete, but the request still creates a partial deployment config.
3. Ambiguous member DB files are summarized too coarsely for an operator to fix the search root.
4. Target DB selection is accidental when more than one deployable design member exists and no explicit `dbnum` or `db_file` is supplied.
5. UI path-fill helpers imply dependencies are verified even though only backend discovery can certify them.
6. Legacy `db_file` quick deploy behavior regresses because MDB resolution is applied when `mbd_name` is absent.

## Development Strategy

### Phase 1: Freeze The Contract

- Keep `specs/019-system-mdb-dependency-discovery/contracts/system-mdb-discovery-contract.md` as the source of truth for request/response shape.
- Confirm `syst_file` remains a compatibility alias while new consumers prefer `source_file` and `source_db_type`.
- Preserve MDB name normalization: trim, add leading slash for comparison, and compare case-insensitively.
- Document that at least one of `search_roots`, `project_path`, or `projects[]` is required when `mbd_name` is present.

### Phase 2: Candidate Discovery And Inventory

- In `src/data_interface/mdb_candidates.rs`, keep candidate project roots separate from dependency proof.
- Build DB file inventory from headers for all DB files under discovered roots.
- Parse only supported system-library types for MDB enumeration, using `SYST`, `GLOB`, then `GLB` priority.
- Deduplicate same-project same-MDB candidates after source-priority ordering.

### Phase 3: Member Status And Evidence

- For each MDB member DB number, classify locate status as `available`, `missing`, or `ambiguous`.
- Include `source_project`, `file_path`, and `candidates` so ambiguous results can be debugged from the candidates endpoint.
- Keep aggregate fields (`available_count`, `missing_count`, `ambiguous_count`, `ready_to_deploy`) for quick UI checks.
- Ensure warnings report system-library parse failures and missing system libraries rather than silently hiding them.

### Phase 4: Quick Deploy Resolution

- In `resolve_quick_deploy_mbd_request`, call the sidecar MDB candidates path only when `mbd_name` is non-empty.
- Reject no-match, missing-member, ambiguous-member, and multi-target cases before site creation/generation.
- Select target DB by priority: explicit `dbnum`, explicit `db_file`, otherwise exactly one deployable design member.
- Fill resolved `projects`, primary `project_path`, `dbnum`, `db_file`, and stable `project_name` before handing off to existing create-only or full quick deploy flows.
- Keep legacy `db_file` mode unchanged when `mbd_name` is absent.

### Phase 5: Admin UI Semantics

- Keep `ui/admin/src/views/SitesView.vue` MDB mode focused on collecting `mbd_name` and `search_roots`.
- Surface backend warnings and resolved target information in success/error messages.
- Keep `SiteDrawer.vue` path fill as a convenience helper only; it must not claim dependency validation.
- If ambiguity detail is too large for the quick deploy error, point users to the MDB candidates/preview flow where candidate paths are exposed.

### Phase 6: Verification And Evidence

- Run formatting on changed Rust files:

  ```powershell
  cargo fmt -- src/data_interface/mdb_candidates.rs src/parse_sidecar.rs src/web_server/models.rs src/web_server/admin_handlers.rs
  ```

- Run static check:

  ```powershell
  cargo check --bin web_server --no-default-features --features "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,rvm-import"
  ```

- Start `web_server` and verify MDB candidates through HTTP/POST with known local roots.
- Verify success-path quick deploy with a deployable MDB, then inspect the returned site id, `resolved_db_file`, warnings, and stored project collection.
- Verify missing dependency failure with a narrowed search root.
- Verify ambiguous dependency failure with a broad root that includes duplicate member DB files.
- Verify legacy `db_file` quick deploy without `mbd_name`.
- Record exact commands, payloads, response summaries, and artifact/config paths in `specs/019-system-mdb-dependency-discovery/quickstart.md` or the implementation progress log.

## Files To Inspect Or Modify

Current repo:

- `src/data_interface/mdb_candidates.rs`
- `src/parse_sidecar.rs`
- `src/web_server/managed_project_sites.rs`
- `src/web_server/models.rs`
- `src/web_server/admin_handlers.rs`
- `ui/admin/src/views/SitesView.vue`
- `ui/admin/src/components/sites/SiteDrawer.vue`
- `ui/admin/src/types/site.ts`

Feature docs:

- `specs/019-system-mdb-dependency-discovery/spec.md`
- `specs/019-system-mdb-dependency-discovery/plan.md`
- `specs/019-system-mdb-dependency-discovery/tasks.md`
- `specs/019-system-mdb-dependency-discovery/contracts/system-mdb-discovery-contract.md`
- `specs/019-system-mdb-dependency-discovery/quickstart.md`

## Acceptance Criteria

- MDB-name quick deploy resolves a complete dependency project collection from parsed `SYST`/`GLOB`/`GLB` facts before site creation.
- Missing and ambiguous member DBs fail early and do not create misleading deployments.
- Candidate discovery responses include source evidence and member locate evidence suitable for operator debugging.
- Target DB selection is deterministic and refuses multi-target ambiguity without explicit input.
- Admin drawer path fill remains non-authoritative.
- Legacy `db_file` quick deploy still works when `mbd_name` is absent.
- Verification uses no `cargo test` and includes concrete HTTP/POST plus static-check evidence.

## Stop And Ask

- Before creating a git commit, pushing, or opening a PR.
- Before deleting, moving, or batch-renaming existing files.
- Before changing public API fields incompatibly.
- Before adding new dependencies or enabling heavy feature combinations.
- Before running remote-server or credential-dependent validation.
