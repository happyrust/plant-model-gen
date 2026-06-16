# Feature Specification: Site Deploy Room Compute Auto Trigger

**Feature Branch**: `015-site-deploy-room-compute-auto-trigger`

**Created**: 2026-06-15

**Status**: Draft

**Input**: User description: "使用 grill-me skill 分析站点部署时如何自动触发房间计算，并编写 spec kit。"

## User Scenarios & Testing

### User Story 1 - Automatically Compute Rooms During Deploy (Priority: P1)

As an admin deploying a managed site, I need room relations to be computed automatically after model generation and before the site is started, so a deployed site is immediately ready for room-based queries without a separate manual CLI/API step.

**Why this priority**: Deployment currently completes parse/generate/start, but room relations are only built by explicit `room compute` flows. That creates a hidden post-deploy prerequisite and allows a site to look deployed while room queries are empty or stale.

**Independent Test**: Deploy a site with generation enabled and `gen_spatial_tree=true`; verify generation completes, `room compute` runs using that site's generated config, room relation tables are populated, and only then does the deployment proceed to start and validation.

**Acceptance Scenarios**:

1. **Given** a site with `gen_model` or `gen_mesh` or `gen_spatial_tree` enabled and `gen_spatial_tree=true`, **When** a full deploy is submitted, **Then** the backend runs model generation, runs room computation, then starts the site.
2. **Given** model generation fails, **When** deploy is running, **Then** room computation is not started and the deploy fails with the generation error.
3. **Given** room computation fails, **When** deploy is running, **Then** the site is not started as a successful deploy and the admin task exposes the room-compute failure.
4. **Given** a site is parse-only with generation disabled, **When** deploy is submitted, **Then** room computation is not triggered.
5. **Given** the sidecar binary does not support `room compute` or was built without the required SQLite spatial-index capability, **When** auto room compute is expected, **Then** deploy fails fast with a capability error instead of silently skipping room readiness.

---

### User Story 2 - Use The Managed Site's Generation Scope (Priority: P1)

As an operator using scoped generation, I need automatic room computation to use the same database scope and configuration as the just-completed generation job, so it does not compute unrelated projects or connect to the wrong database.

**Why this priority**: The existing web API room compute path uses global `DbOption` state and is not site-scoped. Deployment must use the managed site's generated `DbOption-generate.toml` and the same db-number scope as generation.

**Independent Test**: Configure a site with `generate_db_nums=[24383]` and deploy it; verify the submitted room-compute sidecar command includes `--db-nums 24383` and does not use global/default DbOption paths.

**Acceptance Scenarios**:

1. **Given** `generate_db_nums` is non-empty, **When** auto room compute is triggered, **Then** room compute uses those db nums.
2. **Given** `generate_db_nums` is empty and `manual_db_nums` is non-empty, **When** auto room compute is triggered, **Then** room compute falls back to `manual_db_nums`.
3. **Given** both generation and manual db scopes are empty, **When** the site generation is full-scope, **Then** room compute is submitted without `--db-nums` and computes the full generated scope.
4. **Given** `manual_refnos` is used for scoped root-model generation, **When** auto room compute runs, **Then** the db-number scope is still derived from the site db scope; refno-root room compute remains a future explicit enhancement unless implemented safely.
5. **Given** the same site is generated multiple times with different scopes, **When** room compute completes, **Then** persisted room relations for the computed scope are consistent with the latest generation and do not retain stale conflicting relations from previous runs.

---

### User Story 3 - Make Room Compute Progress Observable (Priority: P1)

As an admin watching deployment progress, I need the UI/task/log surfaces to show that deployment is computing rooms, so I can distinguish a legitimate long room-compute stage from a stuck start.

**Why this priority**: Room computation can take long enough that hiding it under "generating" or "starting" makes operations ambiguous and difficult to diagnose.

**Independent Test**: Start a deploy and observe admin task status/logs while the sidecar room-compute job is active; verify the runtime stage and logs identify "room compute" rather than only "generation".

**Acceptance Scenarios**:

1. **Given** room compute is running, **When** admin task status is polled, **Then** the task progress message says room computation is in progress.
2. **Given** room compute emits sidecar job events, **When** the site log view is opened, **Then** the room-compute log contains submitted/running/succeeded/failed events.
3. **Given** deployment fails during room compute, **When** the task fails, **Then** the failure message includes the room-compute exit status or error summary.
4. **Given** the site runtime status is queried during room compute, **When** an active room-compute sidecar job exists, **Then** `current_stage`, `current_stage_label`, `sidecar_job_kind`, and recent activity identify room compute rather than generic start/generate.
5. **Given** a user requests tail logs for the room-compute kind, **When** the log exists, **Then** the API returns the same tail-log shape as existing parse/generate logs.

---

### User Story 4 - Preserve Manual Room Compute Flows (Priority: P2)

As an engineer validating room logic, I need existing CLI and API room compute entry points to remain available, so automatic deployment behavior does not remove explicit recompute and validation workflows.

**Why this priority**: Existing `aios-database room compute`, `room compute-panel`, validation scripts, and room API endpoints are still needed for debugging, performance work, and targeted recompute.

**Independent Test**: Run existing documented CLI validation after implementing auto-trigger and verify commands still behave the same outside managed-site deploy.

**Acceptance Scenarios**:

1. **Given** a user runs `aios-database room compute` manually, **When** the command is outside a managed deploy, **Then** existing CLI semantics are unchanged.
2. **Given** `/api/room/compute` is called manually, **When** it uses the current app state, **Then** the API keeps its existing behavior and is not required for deploy auto-trigger.

## Edge Cases

- `gen_spatial_tree=false` while `gen_model=true`: room compute should be skipped or blocked with a clear reason because the prebuilt AABB/spatial-index prerequisites are not guaranteed.
- `inst_relate_aabb` is empty after a generation job that returned success.
- Sidecar job HTTP polling fails but websocket terminal event reports success or failure.
- Scoped deploy has `manual_refnos` but no `generate_db_nums`.
- User cancels/stops a site while room compute is active.
- The site DB was started for generation and needs cleanup before or after room compute.
- Room compute succeeds but restoring a full spatial index after scoped compute fails.
- Existing deployments should not auto-run room compute on parse-only tasks.
- Quick deploy, local generate/start, and remote deploy all reuse parts of the generation pipeline and need explicit trigger policy.
- Room compute may finish successfully but produce zero relation rows because no rooms match; success must not be judged only by row count.
- Existing relation rows may be stale if a later scoped generation excludes rooms/components that were previously related.

## Grill-Me Decision Record

| Question | Recommended Answer | Status | Reasoning |
|---|---|---|---|
| Q1: Where should auto room compute be triggered? | Inside backend managed-site generation/deploy pipeline after `spawn_generation_process` succeeds and before start. | Resolved by code exploration | `run_deploy_pipeline` calls `run_generation_pipeline` before `run_start_pipeline_for_deploy`; generation produces the room prerequisites. |
| Q2: Should the frontend call `/api/room/compute` after deploy? | No. | Resolved by code exploration | The web API path is not clearly site-scoped and can use global DbOption state. |
| Q3: Should room compute run through sidecar CLI or direct Rust function calls? | Use sidecar CLI job infrastructure for MVP. | Recommended | Parse/generate already use `run_sidecar_cli_job_with_site_events`; it gives logs, cancellation, and site-scoped config. |
| Q4: Should room compute failure block deploy? | Yes for full deploy. | Recommended | A "successful" deploy with missing room relations hides data incompleteness. |
| Q5: Should there be a new UI checkbox? | Not for MVP; derive from generation enabled plus `gen_spatial_tree=true`. | Recommended | The user asked for automatic trigger; adding a new option delays the core behavior. |
| Q6: What scope should room compute use? | Same db-number scope as generation: `generate_db_nums`, else `manual_db_nums`, else full generated scope. | Recommended | Prevents computing unrelated databases while preserving full-site deploy behavior. |
| Q7: Should parse-only deploy trigger room compute? | No. | Resolved | Room compute needs generated AABB/mesh/spatial prerequisites. |
| Q8: Should `DataGeneration` and `FullGeneration` admin tasks also trigger it? | Yes when they call generation-then-start flows and `gen_spatial_tree=true`. | Recommended | The same hidden prerequisite exists when users run generate/start instead of deploy. |
| Q9: Should remote deploy local generation trigger room compute? | Yes for the local artifact package, but remote runtime readiness remains a separate future concern unless remote room compute is explicitly added. | Needs confirmation | `remote_deploy_site_with_task_id` calls `run_generation_pipeline` for local generation before upload; this can prepare local data, but does not prove remote-side room readiness after upload/start. |
| Q10: How should room-compute success be proven? | Use sidecar success plus an explicit report/marker, not only relation row count. | Recommended | Some valid scopes may contain no matching rooms; row count alone creates false failures. |

## Trigger Matrix

| Flow | Existing entry point | Auto room compute policy |
|---|---|---|
| Full local deploy | `deploy_site` / `run_deploy_pipeline` | Trigger after successful generation when `gen_spatial_tree=true`; block start/deploy success on failure. |
| Data generation then start | `generate_site` / `run_generation_then_start_pipeline` | Trigger after successful generation when `gen_spatial_tree=true`; block start on failure. |
| Quick deploy with `start_site=true` | quick deploy path calling `run_deploy_pipeline` | Same as full local deploy. |
| Quick deploy with generate-only | quick deploy path calling `run_generation_pipeline` | Trigger after successful generation when `gen_spatial_tree=true`; return failure in pipeline result if room compute fails. |
| Remote deploy local generation | `remote_deploy_site_with_task_id` calling `run_generation_pipeline` | Trigger for local generated data when `gen_spatial_tree=true`; remote-side room compute is out of MVP unless implemented explicitly. |
| Parse-only | `parse_site` / `run_parse_pipeline` | Never trigger. |
| Start-only | `start_site` / `run_start_pipeline` | Never trigger; start consumes existing generated/room data. |

## Requirements

### Functional Requirements

- **FR-001**: The managed-site backend MUST automatically trigger room computation after a successful generation job and before any deploy/start completion that depends on generated room data.
- **FR-002**: Auto room compute MUST run only when generation is enabled and `gen_spatial_tree=true`.
- **FR-003**: Auto room compute MUST NOT run for parse-only operations.
- **FR-004**: Auto room compute MUST use the managed site's generated configuration file, not global/default room API state.
- **FR-005**: Auto room compute MUST be submitted via the existing sidecar CLI job infrastructure unless a future design explicitly replaces sidecar orchestration.
- **FR-006**: The sidecar command MUST invoke `room compute` with room keywords from site DbOption defaults or explicit `--keywords -RM,-ROOM` if the deploy policy chooses a fixed default.
- **FR-007**: The sidecar command MUST pass `--db-nums` from `site.generate_db_nums` when non-empty.
- **FR-008**: If `site.generate_db_nums` is empty, the sidecar command MUST pass `--db-nums` from `site.manual_db_nums` when non-empty.
- **FR-009**: If both db-number scopes are empty, the sidecar command MUST omit `--db-nums` and allow full-scope room compute.
- **FR-010**: The backend MUST create a distinct sidecar job kind and log surface for room compute.
- **FR-011**: Active sidecar job tracking MUST be able to represent room-compute jobs.
- **FR-012**: Runtime/admin task progress MUST expose a room-compute stage while the room-compute sidecar job is active.
- **FR-013**: If room compute fails during full deploy or generation-then-start, the pipeline MUST fail before marking the site as successfully running.
- **FR-014**: If room compute is skipped because prerequisites are disabled, the skip MUST be logged with the site id and reason.
- **FR-015**: Existing manual room compute CLI/API flows MUST keep their current behavior.
- **FR-016**: Cancellation/stop flows MUST cancel or wait for active room-compute sidecar jobs the same way they handle parse/generate sidecar jobs.
- **FR-017**: Deployment validation SHOULD include a non-blocking or blocking check that room-compute output exists when auto room compute was expected.
- **FR-018**: Runtime stage computation MUST explicitly account for active `RoomCompute` sidecar jobs so room-compute progress is not mislabeled as starting or generating.
- **FR-019**: Tail-log APIs and log snapshots MUST support a room-compute log kind.
- **FR-020**: Auto room compute MUST keep the site DB available for the complete generation-plus-room-compute sequence, or start and clean up a dedicated DB process for room compute.
- **FR-021**: Auto room compute MUST write a completion marker or JSON report containing sidecar job id, scope, success/failure, started/finished time, and any CLI report path.
- **FR-022**: Deploy validation MUST read the room-compute completion marker when auto room compute was expected and fail deploy if the marker is missing or records failure.
- **FR-023**: Auto room compute MUST fail fast when the sidecar binary cannot run `room compute` with required spatial-index support.
- **FR-024**: Auto room compute MUST preserve or enforce idempotent relation persistence for the computed scope; it MUST NOT leave stale conflicting room relations after repeated generation/compute runs.

### Key Entities

- **Auto Room Compute Policy**: Derived policy that decides whether the current managed-site lifecycle should run room compute.
- **Room Compute Sidecar Job**: A site-scoped CLI job submitted through the existing sidecar job infrastructure.
- **Room Compute Scope**: The db-number scope used for room compute, derived from generation scope.
- **Room Compute Runtime Stage**: Observable status/log state for admin task progress and diagnosis.
- **Room Compute Completion Marker**: A runtime artifact recording the latest automatic room-compute result for deploy validation.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A full managed-site deploy with generation enabled and `gen_spatial_tree=true` produces room relation data before reporting deploy success.
- **SC-002**: A scoped deploy with `generate_db_nums=[N]` submits room compute with `--db-nums N`.
- **SC-003**: A room-compute failure causes deploy to fail before successful start/validation is reported.
- **SC-004**: Admin task status and site logs visibly show a room-compute stage.
- **SC-005**: Existing documented `room compute` CLI validation commands continue to run without managed-site deploy.
- **SC-006**: Runtime logs and completion marker show room-compute success even when the valid result for a scope contains zero matching room relations.
- **SC-007**: A repeated scoped generation/room-compute run does not leave stale conflicting room relations from an earlier run.

## Assumptions

- Room compute relies on generation-produced `inst_relate_aabb` and spatial-index prerequisites.
- `gen_spatial_tree=true` remains the best available managed-site signal that room compute prerequisites should exist.
- MVP does not add a frontend opt-out switch; a later feature can add `auto_room_compute_after_generate`.
- The generated DbOption path used by generation is acceptable for room compute.
- Full-scope generation implies full-scope room compute, even if that can be slower.
- Existing room compute persistence is expected to be idempotent for its scope; if code review disproves this, implementation must add cleanup/overwrite behavior before enabling auto trigger.

## Non-Goals

- No redesign of the room compute algorithm.
- No replacement of the sidecar job system.
- No automatic room compute after parse-only jobs.
- No new room compute UI dashboard beyond status/log exposure required for deploy observability.
- No special refno-root room-compute policy for `manual_refnos` in MVP unless implementation proves it can be derived safely.
- No remote-side room compute after SSH upload/start in MVP.
