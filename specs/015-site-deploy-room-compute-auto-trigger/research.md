# Research: Site Deploy Room Compute Auto Trigger

## Existing Lifecycle Findings

- Admin deploy tasks dispatch through `src/web_server/admin_task_handlers.rs` as `TaskType::DeployManagedSite`, then call `managed_project_sites::deploy_site(site_id)`.
- `deploy_site()` eventually runs `run_deploy_pipeline(site_id)`.
- `run_deploy_pipeline()` calls `run_generation_pipeline(site_id, true)` when generation is enabled, otherwise it only parses if needed, then calls `run_start_pipeline_for_deploy(site_id)`.
- `run_generation_pipeline()` starts a DB process, runs `spawn_generation_process(site_id)`, then cleans up the DB process.
- `spawn_generation_process()` writes site config and submits a sidecar CLI job with `SidecarCliJobKind::Generate`.
- Room compute is currently exposed as `aios-database room compute` and HTTP room APIs, but deploy does not call either.
- `run_generation_pipeline()` is a shared helper used by local deploy, generate/start, quick deploy, and remote deploy local generation.
- `runtime_status()` currently computes `current_stage` before exposing `active_sidecar_job`, so room-compute stage visibility needs an explicit code change.
- Existing tail-log comments and routing expect a limited set of log kinds; a new `room-compute` kind must be added where log file paths are resolved.

## Decision: Hook Point

**Decision**: Trigger automatic room compute after `spawn_generation_process(site_id.clone()).await` returns success and before the deploy/start pipeline reports success or starts the Viewer.

**Rationale**:

- Room compute depends on generated model data and `inst_relate_aabb`.
- Deploy start should not be marked successful while required room relation tables are missing.
- The backend pipeline is the only place that reliably knows generation success and site-scoped config.

**Alternatives considered**:

- Frontend calls `/api/room/compute` after deploy: rejected because it is not a robust lifecycle guarantee and can use global state.
- Deploy validation triggers room compute lazily: rejected because validation should observe readiness, not mutate core data.
- Run room compute before generation: rejected because prerequisites are not ready.

## Decision: Trigger Matrix

**Decision**: Treat `run_generation_pipeline()` as the technical hook, but document every caller's intended behavior.

| Flow | Decision | Rationale |
|---|---|---|
| Local full deploy | Trigger and block success on failure | This is the primary user request. |
| `generate_site()` / generation-then-start | Trigger and block start on failure | It reaches the same ready-for-viewer state as deploy. |
| Quick deploy with start | Trigger through full deploy path | Quick deploy should not create a less-ready site. |
| Quick deploy generate-only | Trigger after generation, return pipeline error on failure | It still produces generated artifacts expected to be room-ready. |
| Remote deploy local generation | Trigger for local generated package; do not claim remote-side compute readiness | Current remote deploy uploads local artifacts after local generation, but remote-side recompute is a separate concern. |
| Parse-only | Never trigger | No generated prerequisites. |
| Start-only | Never trigger | Start should not mutate generated/room data. |

## Decision: Execution Mechanism

**Decision**: Reuse sidecar CLI job orchestration and submit `room compute`.

**Rationale**:

- Parse and generate already use `run_sidecar_cli_job_with_site_events()`.
- Sidecar jobs provide status polling, websocket events, active job tracking, log lines, and cancellation integration.
- Existing `room compute` CLI is the documented behavior and already handles AABB checks, relation writes, and scoped spatial index restoration.

**Implementation note**: Add `SidecarCliJobKind::RoomCompute` with label "房间计算", key "room-compute", and a dedicated log path.

## Decision: Scope

**Decision**: Use the same managed-site db-number scope as generation:

1. `site.generate_db_nums` if non-empty.
2. Else `site.manual_db_nums` if non-empty.
3. Else omit `--db-nums` and compute full scope.

**Rationale**:

- Scoped generation should not trigger unrelated room computation.
- Full generation should still produce full room readiness.
- The CLI already treats missing `--db-nums` as full scope.

**Open point**: `manual_refnos` root-model generation may deserve `--refno-root`, but this needs a safe mapping from site root-model strings to room-compute root semantics. Leave out of MVP unless implementation confirms the mapping.

## Decision: Trigger Policy

**Decision**: Auto room compute runs when generation is enabled and `site.gen_spatial_tree == true`.

**Rationale**:

- `room compute` checks `inst_relate_aabb` and relies on generated spatial/AABB data.
- Existing admin UI already treats `gen_spatial_tree` as the option for spatial queries, room tree, and Viewer loading.
- No new UI switch is needed for the MVP.

**Skipped cases**:

- Parse-only operations.
- Generation-disabled deploy.
- `gen_spatial_tree=false`, unless a future feature defines a safe fallback.

## Decision: Failure Policy

**Decision**: Room compute failure blocks full deploy and generation-then-start completion.

**Rationale**:

- A successful deploy should mean the site is ready for room-aware workflows.
- Silent failure would shift the problem to later room queries, where diagnosis is harder.
- The user asked for deployment-time automatic trigger, which implies lifecycle readiness rather than best-effort background work.

**Future option**: Add an explicit "warn only" mode if operations prefer Viewer availability over room readiness.

## Decision: DB Process Lifecycle

**Decision**: The implementation must ensure the site DB is running for room compute with the same site config used for generation.

**Rationale**:

- Current `run_generation_pipeline()` starts DB for generation and cleans it up immediately after generation.
- `room compute` calls `init_surreal()` and reads/writes SurrealDB tables.
- If generation cleanup happens before room compute, room compute may connect to the wrong DB or fail.

**Implementation options**:

- Preferred: keep the generation-started DB alive until both generation and room compute complete, then clean up once.
- Alternative: after generation cleanup, start a new DB process for room compute and clean it up separately.

**Hard requirement**: The chosen option must use cleanup-on-all-paths semantics so success, failure, cancellation, and panics do not leave the pipeline-owned DB process registered as running.

## Decision: Observability

**Decision**: Add explicit room-compute sidecar kind, log, active-job status, and admin task progress message.

**Rationale**:

- Room compute may be long-running.
- Operators need to distinguish generation, room compute, and startup.
- Existing sidecar event logging can be reused with minimal change.

**Implementation detail**: `current_stage()` must receive or otherwise inspect `active_sidecar_job`. A `RoomCompute` active job should override generic `starting`/`parsed-db-ready` labels with `room_computing` / `房间计算中`.

## Decision: Success Marker

**Decision**: Write an explicit room-compute completion marker or JSON report after every automatic run.

**Rationale**:

- A valid room-compute scope can have zero matching rooms, so relation row count alone is not a reliable success signal.
- Deploy validation needs a stable artifact to confirm the automatic stage actually ran.
- The marker can record scope, sidecar job id, status, timing, report path, and error message without changing SurrealDB schema.

Suggested path:

```text
runtime/admin_sites/<site_id>/room-compute-result.json
```

## Decision: Keywords

**Decision**: Prefer not passing `--keywords` in MVP, so the CLI uses the managed site's DbOption defaults.

**Rationale**:

- Hard-coding `-RM,-ROOM` may override site-specific room keyword configuration.
- Hyphen-prefixed values are more error-prone when manually constructing CLI args.
- Existing `room_compute_mode` already falls back to `db_option_ext.get_room_key_word()`.

## Decision: Idempotency

**Decision**: Auto room compute must rely on or enforce idempotent relation persistence for the computed scope.

**Rationale**:

- Deployment can run repeatedly for the same site.
- Scoped generation may change the set of generated rooms/components.
- Stale `room_relate` or `room_panel_relate` rows would be worse than a visible failure because deploy would appear successful with mixed old/new relationships.

**Implementation check**: Before enabling auto trigger, verify existing `room compute` deletes/replaces relations for its scope. If it does not, add cleanup/overwrite behavior or block auto trigger for unsafe scopes.

## Risks

- DB cleanup ordering can break room compute if not adjusted.
- Scoped room compute restores full spatial index after a scoped run; deploy should surface restore failures clearly.
- Full-scope room compute can be expensive when no db-number scope is configured.
- If `gen_spatial_tree=true` generation returns success but `inst_relate_aabb` is empty, room compute should fail fast with its existing preflight message.
- Sidecar binaries built without `sqlite-index`/room compute support must produce a clear capability failure.
- Remote deploy may need separate remote-side room compute in a future feature if uploaded local artifacts are insufficient.
