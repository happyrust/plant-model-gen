# Feature Specification: System-Library MDB Dependency Discovery

**Feature Branch**: `[019-system-mdb-dependency-discovery]`

**Created**: 2026-06-16

**Status**: Draft

**Input**: User description: "快速站点部署按 MDB 名称定位依赖项目路径时，必须先解析 SYST 和 GLOB 等系统文件，才能快速可靠地知道依赖工程路径。"

## Purpose

Make MDB-name quick deploy resolve dependent E3D project paths from parsed system-library facts instead of directory-name guesses. Operators should be able to provide an MDB name and a search root, and the system should discover the E3D source project collection, target DB, and dependency completeness from SYST/GLOB/GLB system-library content before creating a deployment configuration.

## Grill-Me Analysis Decisions

| Decision Branch | Recommended Answer | Rationale |
|---|---|---|
| Can the system infer dependencies from directory names alone? | No. Directory names are only candidate roots; dependency authority comes from parsed system-library content. | E3D project folders can be renamed, duplicated, or incomplete. MDB membership is semantic data, not a folder convention. |
| Which system libraries define the quick-deploy discovery basis? | Parse SYST first, then use GLOB/GLB as supported system-library sources and evidence. | Current behavior only read SYST; the user clarified that GLOB and related system files must participate before dependency paths are trusted. |
| Should quick deploy proceed if MDB dependencies are incomplete? | No. It must fail with a clear missing/ambiguous dependency report. | Silent fallback to partial or guessed project paths creates broken parse/generate plans and hides the real operator action needed. |
| Should the site configuration drawer do the same full discovery? | No. Drawer path fill remains a convenience helper; full dependency discovery belongs to scan/MDB-candidates/quick-deploy flows. | The drawer can build `root + name` quickly, but only backend system-library discovery can certify dependencies. |
| Should already discovered `projects[]` bypass scanning? | Yes, but MDB candidate validation still parses system libraries for those projects. | Advanced callers can provide an explicit project collection; semantic validation remains mandatory. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Quick Deploy Finds Dependencies From MDB Name (Priority: P1)

As a deployment operator, I need to enter an MDB name and a parent search root so the system can discover the required E3D source projects and target DB from system-library content, allowing quick site creation without manually assembling project paths.

**Why this priority**: This is the primary workflow requested by the user and removes error-prone manual dependency path entry.

**Independent Test**: Provide an MDB name such as `/ALL` and a search root such as `D:\AVEVA\Projects\E3D2.1`; verify that the quick-deploy response resolves a target DB, primary project path, and full project collection using parsed system-library facts.

**Acceptance Scenarios**:

1. **Given** a search root containing multiple E3D project folders with valid system libraries, **When** the operator submits an MDB name, **Then** the system scans candidate projects, parses system-library facts, and resolves the MDB to exactly one deployable target DB.
2. **Given** the MDB has members across design and catalogue/library projects, **When** quick deploy resolves the request, **Then** the generated project collection includes all dependency projects required by the MDB.
3. **Given** a caller directly supplies `projects[]`, **When** MDB quick deploy runs, **Then** the system uses that collection as candidate roots but still validates MDB membership by parsing system libraries.

---

### User Story 2 - Incomplete Or Ambiguous Dependencies Fail Early (Priority: P1)

As an operator, I need quick deploy to stop before creating a misleading deployment when an MDB dependency is missing or ambiguous, so I can correct the search root or project collection first.

**Why this priority**: A partial dependency set is the root cause of downstream parse/generate failures such as missing CATA or BRAN outputs.

**Independent Test**: Remove or duplicate a dependency DB file in a controlled fixture; verify quick deploy reports missing or ambiguous DB members and does not proceed as if the MDB were deployable.

**Acceptance Scenarios**:

1. **Given** an MDB member DB cannot be located under the discovered projects, **When** quick deploy resolves the MDB, **Then** it fails with a missing dependency message naming the MDB and member DB.
2. **Given** a member DB maps to multiple candidate files, **When** quick deploy resolves the MDB, **Then** it fails with an ambiguity message listing enough candidate context for the operator to narrow the search root.
3. **Given** no SYST/GLOB/GLB system-library files are found, **When** MDB discovery runs, **Then** it returns no deployable candidates and explains that system-library parsing is unavailable.

---

### User Story 3 - Operators Can Inspect Discovery Evidence (Priority: P2)

As a maintainer or advanced operator, I need the MDB candidate result to show which system library proved the MDB and which file supplied each member DB, so failed or surprising discovery can be diagnosed quickly.

**Why this priority**: Dependency discovery is opaque without evidence. Evidence reduces debugging time when project roots contain duplicate, stale, or incomplete files.

**Independent Test**: Call the MDB candidates endpoint and verify each candidate exposes source evidence and each member row includes locate status, source project, file name/path, and candidates when ambiguous.

**Acceptance Scenarios**:

1. **Given** a valid MDB candidate, **When** candidates are returned, **Then** the candidate identifies the source system-library file and source DB type used to enumerate the MDB.
2. **Given** a member DB is available, **When** candidates are returned, **Then** the member row identifies the source project and file path.
3. **Given** a member DB is ambiguous, **When** candidates are returned, **Then** the member row includes all candidate file paths.

---

### User Story 4 - Drawer Path Fill Remains A Convenience Only (Priority: P3)

As an operator editing a site, I need the site configuration helper to fill likely project paths quickly while understanding that dependency validation happens through backend discovery, so the UI is useful but not misleading.

**Why this priority**: The helper improves speed but must not imply semantic validation has already happened.

**Independent Test**: Use the drawer helper to build `D:\AVEVA\Projects\E3D2.1\AvevaCatalogue`; verify it fills path fields and scan root, but dependency completeness is still reported only by scan/MDB-candidates/preview/quick-deploy flows.

**Acceptance Scenarios**:

1. **Given** a root and project/MDB folder name, **When** the operator clicks fill path, **Then** the drawer fills `project_path`, source project name, and scan root without claiming dependencies are verified.
2. **Given** the operator runs dependency discovery afterward, **When** system-library parsing finds issues, **Then** the UI must surface those issues rather than relying on the filled path.

---

### Edge Cases

- Search root points directly at a single E3D project rather than a parent containing many projects.
- Multiple projects contain the same dbnum, causing ambiguous member DB resolution.
- MDB name casing or missing leading slash differs from stored MDB names.
- GLOB/GLB files parse successfully but contain no MDB elements.
- SYST fails to parse but GLOB/GLB contains a matching MDB candidate.
- `projects[]` is supplied explicitly and `search_roots` is empty.
- Legacy quick-deploy callers still provide only `db_file`; existing dbfile mode must remain unchanged.
- Search root contains hidden, dotted, non-DB, or extremely large directory trees.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST support quick deploy requests that provide `mbd_name` plus either `search_roots`, `project_path`, or explicit `projects[]`.
- **FR-002**: System MUST treat directory scanning as candidate discovery only; MDB dependency authority MUST come from parsed system-library facts.
- **FR-003**: System MUST parse supported system-library files (`SYST`, `GLOB`, `GLB`) when enumerating MDB candidates.
- **FR-004**: System MUST normalize MDB names consistently before matching, including leading slash and case-insensitive comparison.
- **FR-005**: System MUST scan candidate project roots and build a DB file inventory from file headers before locating MDB member DB files.
- **FR-006**: System MUST enumerate MDB candidates from parsed system-library content and list their member dbnums in declared order when available.
- **FR-007**: System MUST locate each MDB member dbnum across the discovered project collection and classify it as available, missing, or ambiguous.
- **FR-008**: System MUST reject quick deploy when the requested MDB has missing or ambiguous members; it MUST NOT silently fall back to partial or guessed dependencies.
- **FR-009**: System MUST resolve exactly one target DB for quick deploy using this priority: explicit `dbnum`, explicit `db_file`, otherwise a single deployable design member.
- **FR-010**: System MUST populate the quick-deploy request with resolved `projects`, primary `project_path`, target `dbnum`, target `db_file`, and a default project name when needed.
- **FR-011**: System MUST expose discovery evidence for each MDB candidate, including source system-library file, source DB type, member status, source project, and candidate paths when ambiguous.
- **FR-012**: System MUST preserve legacy dbfile quick deploy behavior when `mbd_name` is absent.
- **FR-013**: Site configuration path fill MUST remain a non-authoritative convenience helper and MUST NOT mark dependencies as verified.
- **FR-014**: Discovery warnings MUST be included in responses so operators can understand parse failures, missing system libraries, and ambiguity.

### Key Entities

- **Search Root**: A parent directory or project directory supplied by the operator to locate candidate E3D projects.
- **E3D Source Project**: A discovered or explicitly supplied project path with name, role, dbnums, and db types.
- **System Library Source**: A parsed SYST/GLOB/GLB DB file used to enumerate MDB candidates.
- **MDB Candidate**: A discovered MDB name, source evidence, member dbnums, member DB file statuses, and deployability summary.
- **Member DB File Status**: Locate result for one MDB member dbnum: available, missing, or ambiguous.
- **Quick Deploy Resolution**: The normalized request after MDB discovery fills project collection, primary path, target dbnum, and target db file.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For a valid MDB under a correct search root, 100% of required member DBs are classified before quick deploy creates a site configuration.
- **SC-002**: Missing or ambiguous member DB cases fail before site creation/generation in 100% of quick-deploy attempts.
- **SC-003**: Operators can identify the system-library source file and source DB type for every returned MDB candidate.
- **SC-004**: Legacy dbfile quick deploy still succeeds without requiring `mbd_name`.
- **SC-005**: For typical local project roots under `D:\AVEVA\Projects\E3D2.1`, MDB candidate discovery completes quickly enough for interactive admin use and returns actionable warnings instead of hanging.

## Assumptions

- `SYST`, `GLOB`, and `GLB` are the supported system-library DB types for MDB candidate discovery in this feature.
- Existing file-header parsing is sufficient to build the DB inventory; full parsing is required only for supported system-library sources.
- Existing sidecar isolation remains the boundary for reading E3D DB files; `web_server` does not read them directly.
- Validation follows repository rules: no `cargo test` for `web_server`; use `cargo check`, running-service HTTP/POST, CLI/json, and generated artifact inspection.
- This feature does not replace full parse/index generation; it only makes pre-deploy dependency discovery authoritative enough to create a correct quick-deploy configuration.
