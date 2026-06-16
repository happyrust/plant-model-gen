# Feature Specification: Deployment Project Identity Over E3D Collection

**Feature Branch**: `[018-deployment-project-identity]`

**Created**: 2026-06-16

**Status**: Draft

**Input**: User description: "我的站点部署项目是包含多个 E3D 项目的集合，我的这个项目名称决定了数据库的名称、目录的名称、提供给外部访问的名称等等，和 E3D 的项目名应该是不冲突的。"

## Purpose

Establish the authoritative identity model for managed deployments: a deployment is a collection of one or more E3D source projects, and the deployment project name is the single outward identity that determines the database name, runtime/output directory names, and external (viewer) access name. The deployment project name and the E3D source project names are two independent namespaces and must never conflict or be substituted for one another.

This spec formalizes the model and adds the independence invariant plus a regression guard. It cross-references prior specs and does not re-implement what they already cover:

- `014-site-project-identity-config`: deployment name uniqueness across sites, deployment name as editable DB/business identity, rename workflow, parse-config visibility.
- `016-custom-project-output-namespace`: output namespace (scene_tree/parquet/CATA manifest), DbOption project_name, parquet/viewer access all use the deployment name; `included_projects`/`project_dirs` keep E3D source names.

## Grill-Me Analysis Decisions

| Decision Branch | Recommended Answer | Rationale |
|---|---|---|
| New spec vs amend 014/016 | New spec 018 as the authoritative identity model, cross-referencing 014/016. | 014/016 stay focused; 018 is the single source of truth for the model and adds the independence invariant + guard. |
| Deployment name equal to an E3D source name | Allowed (independent fields), but surfaced with a warning; deployment-name uniqueness across sites is enforced (reuse 014 FR-003). | They are different fields with different purposes; coincidence is not a conflict but can confuse operators. |
| Regression guard form | A static guard that asserts outward-identity consumers (DB name, runtime dir, output dir, viewer output_project, parquet root) derive from the deployment name and never from the E3D source name. | Prevents silent regressions back to using an E3D source name for outward identity. |
| Collection semantics | Reuse `site.projects` with `is_primary` and `role` (design/library); deployment name is independent of the primary E3D project name. | The multi-E3D collection already exists; the deployment name must not be tied to any single member. |
| Validation surfaces | Enforce uniqueness and surface independence in create, edit, clone, quick-deploy, preview; details view distinguishes deployment name from the E3D collection. | Operators configure deployments through all these flows and must see a consistent model. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Deployment Name Is The Sole Outward Identity (Priority: P1)

As an operator deploying a collection of E3D projects under one site, I need the deployment project name I choose to be the single identity used for the database name, the runtime/output directories, and the external access name, so the deployment is addressed consistently regardless of which E3D projects it contains.

**Why this priority**: This is the core model the operator relies on to reason about and access a deployment.

**Independent Test**: Create a deployment named `9002` containing E3D projects `AvevaPlantSample` + `AvevaCatalogue`; verify the database name, runtime/output directories, and viewer access name all use `9002`.

**Acceptance Scenarios**:

1. **Given** a deployment named `9002` with multiple E3D source projects, **When** its configuration is generated, **Then** the database name, runtime directory, output directory, and viewer `output_project` all use `9002`.
2. **Given** the deployment contains several E3D projects, **When** outward identity is resolved anywhere in the system, **Then** it uses the deployment name and never an E3D source project name.
3. **Given** the deployment name changes via the rename workflow, **When** configuration is regenerated, **Then** all outward-identity surfaces reflect the new deployment name.

---

### User Story 2 - E3D Names Are Source-Only And Non-Conflicting (Priority: P1)

As an operator, I need the E3D source project names to be used only for locating source data, never for the deployment's outward identity, so a deployment name can be chosen freely without colliding with E3D project names.

**Why this priority**: Without this guarantee, choosing a deployment name could clash with an E3D project folder or be silently overridden.

**Independent Test**: Generate a deployment whose name differs from all its E3D project names; verify `included_projects`/`project_dirs` carry the E3D names/paths while outward identity uses the deployment name. Then set the deployment name equal to one E3D name and verify it still works with a warning.

**Acceptance Scenarios**:

1. **Given** a deployment with E3D projects `AvevaPlantSample` + `AvevaCatalogue`, **When** configuration is generated, **Then** `included_projects` contains the E3D names and `project_dirs` the E3D paths, independent of the deployment name.
2. **Given** a deployment name set equal to an E3D source project name, **When** the deployment is created/edited, **Then** the operation succeeds and a warning notes the coincidence, with no functional conflict.
3. **Given** any outward-identity path (DB/dir/external access), **When** it is computed, **Then** it does not read an E3D source project name.

---

### User Story 3 - Independence Regression Guard (Priority: P2)

As a maintainer, I need an automated guard that fails when outward-identity code starts using an E3D source name, so the independence invariant cannot silently regress.

**Why this priority**: The bug class (e.g., the diagnosed CATA manifest path using a source name) is easy to reintroduce.

**Independent Test**: Run the guard against the codebase; it passes on the current correct state and fails when an outward-identity path is switched to a source-name helper.

**Acceptance Scenarios**:

1. **Given** the current codebase, **When** the guard runs, **Then** it passes.
2. **Given** an outward-identity consumer is changed to use the E3D source name, **When** the guard runs, **Then** it fails with a clear message identifying the offending location.

---

### User Story 4 - Consistent Model Across Configuration Surfaces (Priority: P3)

As an operator, I need create, edit, clone, quick-deploy, preview, and details views to present the deployment-name-vs-E3D-collection model consistently, so the identity model is unambiguous everywhere.

**Why this priority**: Operators configure deployments across multiple surfaces; inconsistency causes mistakes.

**Independent Test**: Open each surface for a multi-E3D deployment and verify the deployment name and the E3D project collection are both shown and consistent.

**Acceptance Scenarios**:

1. **Given** a multi-E3D deployment, **When** the details view renders, **Then** it shows the deployment name as the outward identity and lists the E3D source projects as the collection.
2. **Given** create/edit/clone/quick-deploy/preview flows, **When** a deployment name is entered, **Then** uniqueness is enforced and independence from E3D names is preserved.

---

### Edge Cases

- Deployment name equals an E3D source project name (allowed, warned).
- Deployment name differs only by case from another deployment name (uniqueness normalization).
- A deployment contains a single E3D project (collection of one).
- A deployment contains multiple E3D projects across different root directories.
- An E3D project name would be a valid folder but the deployment name is not an existing folder (must not break source discovery).
- Legacy sites created before this model where outward identity may have used a source name (covered by 016 regeneration; not migrated).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST model a deployment as a collection of one or more E3D source projects represented by `site.projects`.
- **FR-002**: System MUST use the deployment project name as the sole outward identity for the database name, runtime directory, output directory, and external (viewer) access name.
- **FR-003**: System MUST use E3D source project names and paths only for source data discovery (`included_projects`, `project_dirs`, source roots), never for outward identity.
- **FR-004**: System MUST treat the deployment project name and E3D source project names as independent namespaces; a coincidental equal value MUST NOT cause functional conflict.
- **FR-005**: System MUST surface a warning when a deployment name equals one of its E3D source project names.
- **FR-006**: System MUST enforce deployment-name uniqueness across managed sites (consistent with 014 FR-003).
- **FR-007**: System MUST provide a regression guard that fails when an outward-identity consumer uses an E3D source project name.
- **FR-008**: System MUST keep outward-identity resolution consistent after a deployment rename (consistent with 014/016).
- **FR-009**: System MUST present the deployment-name-vs-E3D-collection model consistently in create, edit, clone, quick-deploy, preview, and details surfaces.
- **FR-010**: System MUST NOT require migration of legacy outward-identity artifacts; regeneration adopts the deployment-name identity (consistent with 016).

### Key Entities

- **Deployment Project**: A managed site identified by immutable `site_id` and an editable deployment `project_name`; the single outward identity.
- **E3D Source Project**: A member of the deployment's collection, with its own name, path, and role; used only for source discovery.
- **Project Collection**: The `site.projects` set with `is_primary` and `role` (design/library).
- **Outward Identity Surfaces**: Database name, runtime directory, output directory, viewer access name.
- **Independence Guard**: An automated check asserting outward-identity surfaces never use an E3D source name.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For a deployment `9002` over E3D projects `AvevaPlantSample` + `AvevaCatalogue`, 100% of outward-identity surfaces use `9002`.
- **SC-002**: `included_projects`/`project_dirs` carry the E3D names/paths in 100% of generated configs, independent of the deployment name.
- **SC-003**: Setting the deployment name equal to an E3D name succeeds with a warning and zero functional conflicts.
- **SC-004**: The independence guard passes on the current codebase and fails when an outward-identity path is switched to a source name.
- **SC-005**: Deployment-name uniqueness is enforced across all configuration surfaces.

## Assumptions

- `site_id` remains the immutable technical identity; `project_name` is the editable outward identity.
- The multi-E3D collection is represented by the existing `site.projects` structure.
- Outward-identity resolution centralizes on the deployment-name helper introduced in 016 (`site_deployment_project_name`).
- Validation uses running web_server HTTP/POST flows, CLI/json, generated config inspection, and a static guard; repository rules forbid adding/running `cargo test` for `web_server`.
- Legacy artifacts are not migrated; regeneration adopts the deployment identity.
