# Implementation Plan: Site Deploy Room Compute Auto Trigger

**Branch**: `015-site-deploy-room-compute-auto-trigger` | **Date**: 2026-06-15 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/015-site-deploy-room-compute-auto-trigger/spec.md`

## Summary

Add a managed-site lifecycle stage that runs `aios-database room compute` automatically after successful model generation and before site start/deploy validation. The trigger is backend-owned, site-scoped, sidecar-orchestrated, and derived from existing generation settings: generation must be enabled and `gen_spatial_tree=true`. The implementation must also make room compute visible in runtime/task/log surfaces and record a completion marker for deploy validation.

## Technical Context

**Language/Version**: Rust backend with Axum web server, Tokio async tasks, sidecar CLI orchestration, Vue 3 admin frontend for observability.

**Primary Dependencies**: `src/web_server/managed_project_sites.rs`, `src/web_server/admin_task_handlers.rs`, `src/web_server/parse_sidecar_client.rs`, `src/web_server/models.rs`, `src/cli_modes.rs`, `src/main.rs`, `docs/guides/ROOM_COMPUTE_CLI_VALIDATION.md`.

**Storage**: SurrealDB stores generated model and room relation data; SQLite `output/spatial_index.sqlite` and `inst_relate_aabb` are room-compute prerequisites. Admin registry remains unchanged unless a future option is added.

**Testing**: Follow repository practice for web_server changes: validate through running service HTTP/admin task flows and sidecar logs. CLI-level validation can use documented `aios-database room compute` commands and JSON/log inspection. Add targeted Rust tests only where existing local unit tests already cover pure helper behavior.

**Target Platform**: Windows local managed-site deployment first; keep sidecar CLI args portable.

**Performance Goals**: Automatic room compute should add no extra parse/generate work. It should reuse generated artifacts and only compute room relations for the generation scope.

**Constraints**: Do not call the global `/api/room/compute` endpoint from deploy. Do not run room compute when prerequisites are disabled. Do not hide the room-compute stage under generic "starting". Do not rely on relation row count alone as success proof.

**Scale/Scope**: One room-compute sidecar job per managed site lifecycle operation; db scope is `generate_db_nums`, else `manual_db_nums`, else full.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Backend-owned lifecycle**: PASS. Trigger lives in managed-site pipeline, not frontend follow-up calls.
- **Site-scoped configuration**: PASS. Use generated site DbOption path instead of global room API state.
- **Observability**: PASS. Add room-compute sidecar kind, log/tail support, runtime stage visibility, and completion marker.
- **Minimal new configuration**: PASS. MVP derives behavior from existing generation fields.
- **Preserve manual workflows**: PASS. Existing CLI/API room compute paths remain unchanged.

No gate violations.

## Project Structure

### Documentation (this feature)

```text
specs/015-site-deploy-room-compute-auto-trigger/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── managed-site-room-compute-auto-trigger-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/web_server/
├── managed_project_sites.rs      # lifecycle hook, sidecar kind/logs, room-compute helper
├── admin_task_handlers.rs        # task progress messages for room-compute stage
├── parse_sidecar_client.rs       # existing job submit/status path reused
└── models.rs                     # only if response/status schema needs explicit fields

src/
├── main.rs                       # existing room compute CLI arg contract
└── cli_modes.rs                  # existing room compute behavior, no semantic rewrite

ui/admin/src/
└── views/components for task/log display if backend exposes a new stage label
```

**Structure Decision**: Keep orchestration in `managed_project_sites.rs` because parse/generate/start pipeline sequencing already lives there. Avoid a new service layer unless room-compute policy grows beyond a small helper.

## Phase 0: Research

Output: [research.md](./research.md)

Resolved decisions:

- Hook after `spawn_generation_process(site_id.clone()).await` succeeds.
- Prefer sidecar CLI job over `/api/room/compute`.
- Add `SidecarCliJobKind::RoomCompute`.
- Default failure policy blocks full deploy/generation-then-start.
- Scope uses `generate_db_nums`, then `manual_db_nums`, then full.
- Runtime status must account for active room-compute sidecar jobs.
- Room-compute success should be recorded as a marker/report, not inferred only from non-empty relation tables.

Open decisions requiring user confirmation:

- Whether a future UI option should allow "warn only" instead of blocking deploy on room-compute failure.
- Whether `manual_refnos` should be translated into `--refno-root` for room compute in a later scoped-root enhancement.
- Whether remote deploy should also run room compute on the remote host after upload/start; MVP only covers local generated data through the shared local generation pipeline.

## Phase 1: Design & Contracts

Output:

- [data-model.md](./data-model.md)
- [contracts/managed-site-room-compute-auto-trigger-contract.md](./contracts/managed-site-room-compute-auto-trigger-contract.md)
- [quickstart.md](./quickstart.md)

## Implementation Approach

1. Add a room-compute log path helper, for example `room_compute_log_path(site_id)`.
2. Extend `SidecarCliJobKind` with `RoomCompute`, including label/key/log path.
3. Add `should_auto_compute_rooms_after_generation(site)` and `room_compute_db_nums(site)` helpers.
4. Add `run_room_compute_pipeline(site_id)` that reloads site/config, writes site files, builds sidecar CLI args, submits `room compute`, and maps failure to `last_error`.
5. Call `run_room_compute_pipeline(site_id.clone()).await?` after successful generation inside `run_generation_pipeline`, before cleanup/start completion semantics are observed by deploy.
6. Ensure the generation-started DB remains available for room compute or deliberately start/cleanup a DB process around the combined generation+room-compute sequence.
7. Write a room-compute completion marker/report under the site runtime directory.
8. Update runtime status, active sidecar job status, and admin task progress to display room computation.
9. Add deploy validation/log snapshot/tail-log support for room-compute artifacts.
10. Validate full deploy, scoped deploy, quick deploy, remote local-generation behavior, skipped parse-only deploy, and failure behavior.

## Post-Design Constitution Check

- **Backend-owned lifecycle**: PASS. Hook is in `run_generation_pipeline`.
- **Site-scoped configuration**: PASS. Helper uses `generation_config_path(site_id)` and `config_path_without_toml`.
- **Observability**: PASS. Dedicated sidecar kind/log, tail-log support, task status, and completion marker.
- **No algorithm rewrite**: PASS. Existing `room compute` CLI remains source of truth.

## Complexity Tracking

No constitution violations. Main risks:

- DB process lifecycle: current generation cleanup occurs immediately after `spawn_generation_process`. The implementation must either keep the same DB process alive through room compute or start a new scoped DB process before room compute and clean it afterward.
- Shared generation callers: quick deploy, generate/start, and remote deploy reuse `run_generation_pipeline`, so implementation must intentionally document and test which flows trigger room compute.
- Success semantics: zero relation rows can be valid; deploy validation must use a marker/report plus sidecar success rather than row count alone.
