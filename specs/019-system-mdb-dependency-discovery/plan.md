# Implementation Plan: System-Library MDB Dependency Discovery

**Branch**: `[019-system-mdb-dependency-discovery]` | **Date**: 2026-06-16 | **Spec**: `specs/019-system-mdb-dependency-discovery/spec.md`

**Input**: Feature specification from `/specs/019-system-mdb-dependency-discovery/spec.md`

## Summary

Upgrade MDB-name quick deploy so dependent E3D project paths are resolved from parsed system-library facts rather than directory-name guesses. The discovery path remains sidecar-owned: scan roots produce candidate E3D projects and DB inventories, then SYST/GLOB/GLB files are parsed to enumerate MDB candidates and member DBs. Quick deploy proceeds only when the requested MDB resolves to a complete, unambiguous dependency set.

## Technical Context

**Language/Version**: Rust backend with Axum web server and aios-database sidecar; Vue 3 + TypeScript admin UI for request entry.

**Primary Dependencies**: `src/data_interface/mdb_candidates.rs`, `src/parse_sidecar.rs`, `src/web_server/managed_project_sites.rs`, `src/web_server/models.rs`, admin site UI quick deploy form, existing parse DB header parsing, and `parse_pdms_db::parse_file`.

**Storage**: No persistent schema change required. Discovery is read-only and returns transient candidate/resolution data.

**Testing**: No `cargo test` or Rust test targets for `web_server`. Validate with `cargo fmt`, `cargo check`, running-service HTTP/POST to quick deploy / MDB candidates, CLI/json where needed, and generated response/artifact inspection.

**Target Platform**: Windows local managed site deployment, with sidecar behavior compatible across development and packaged runtime layouts.

**Project Type**: Rust web service + sidecar + Vue admin frontend.

**Performance Goals**: Candidate discovery must remain responsive for interactive admin use. It should parse only supported system libraries for MDB enumeration and use file-header inventory for all other DB files.

**Constraints**: `web_server` must not directly read E3D DB files; all DB file reads stay in sidecar/data-interface code. Legacy dbfile quick deploy must remain unchanged. Failure must be explicit, not a fallback to partial deployment.

**Scale/Scope**: One MDB name per quick deploy request; multiple E3D project roots; member DB count determined by system-library MDB membership.

## Constitution Check

Repository rules require no `cargo test` for `web_server`; this plan uses `cargo check` and running-service/HTTP validation. aios-database validation, if needed, uses CLI/json and generated artifacts. `.specify/memory/constitution.md` is still placeholder-only, so there are no additional gates.

## Project Structure

### Documentation (this feature)

```text
specs/019-system-mdb-dependency-discovery/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── system-mdb-discovery-contract.md
└── checklists/
    └── requirements.md
```

### Source Code (repository root)

```text
src/
├── data_interface/
│   └── mdb_candidates.rs        # system-library parsing, MDB candidate enumeration, member DB locate status
├── parse_sidecar.rs             # sidecar endpoint orchestration and request normalization
└── web_server/
    ├── managed_project_sites.rs # quick-deploy request resolution and failure handling
    ├── models.rs                # request/response contracts and documentation
    └── admin_handlers.rs        # admin endpoint documentation/surface

ui/admin/src/
├── views/SitesView.vue          # quick deploy mode and MDB/search-root request payload
└── components/sites/
    └── SiteDrawer.vue           # convenience path fill, not authoritative dependency validation
```

**Structure Decision**: Keep DB-reading logic in sidecar/data-interface modules and keep quick-deploy orchestration in `managed_project_sites.rs`. The admin UI only collects inputs and displays results; it does not certify dependency completeness.

## Technical Approach

### 1. Candidate project discovery

- Accept `projects[]` when explicitly provided.
- Otherwise collect candidate E3D projects by scanning each `search_root` and optional `project_path`.
- Treat directory scan output as candidate roots only; do not mark dependencies as verified at this stage.

### 2. DB inventory

- Walk candidate project roots under existing safety limits.
- Parse DB file headers to map dbnum, db type, latest session page, file name, file path, and source project.
- Keep all DB types in the locate inventory, but only supported system-library types participate in full parse for MDB enumeration.

### 3. System-library MDB enumeration

- Parse supported system libraries in priority order: SYST, GLOB, GLB.
- Enumerate MDB elements and member DB records from parsed content.
- Deduplicate same-project same-MDB candidates after priority ordering.
- Preserve source evidence: source system-library file and source DB type.

### 4. Member DB locate status

- For each MDB member dbnum, resolve against the DB inventory.
- Mark each member as available, missing, or ambiguous.
- Compute deployability: an MDB is deployable only if every member is available and none are ambiguous.

### 5. Quick-deploy resolution

- Normalize user-provided MDB names before matching.
- Select target DB by explicit `dbnum`, explicit `db_file`, or a single deployable design member.
- Fill resolved `projects`, primary `project_path`, target `dbnum`, target `db_file`, and default `project_name` before creating the site.
- Fail early with warnings/errors when no matching MDB, multiple target candidates, missing members, or ambiguous members exist.

### 6. UI semantics

- Quick deploy UI submits `mbd_name` and `search_roots`.
- Site drawer path fill remains a convenience helper that writes a likely `project_path` and scan root; dependency verification is still performed by backend discovery.

## Validation Plan

- Run `cargo fmt` on changed Rust files.
- Run `cargo check --bin web_server --no-default-features --features "ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,rvm-import"`.
- Start web_server and POST to the admin quick deploy endpoint with `mbd_name` and `search_roots`; verify resolved `projects`, `dbnum`, `db_file`, and warnings.
- Call the MDB candidates endpoint for the same project collection; verify `source_file`, `source_db_type`, member statuses, missing/ambiguous reporting, and deployability counts.
- Validate legacy `db_file` quick deploy still works without `mbd_name`.
- Inspect generated site config to confirm the dependency project collection matches the MDB members.

## Post-Design Constitution Check

- **No cargo tests for web_server**: PASS. Validation uses `cargo check` plus running-service HTTP/POST.
- **aios-database validation via CLI/json**: PASS. Any sidecar/CLI validation uses command output and JSON/artifact inspection.
- **Sidecar DB-read boundary**: PASS. DB file reads remain outside `web_server`.
- **No silent partial deploy**: PASS. Missing/ambiguous dependencies fail before site creation/generation.

## Complexity Tracking

No constitution violations or complexity exceptions.
