# Contract: Managed Site Room Compute Auto Trigger

This contract describes observable behavior for managed-site deploy/generate flows after room-compute auto trigger is implemented.

## Trigger Contract

### Full Deploy

**Entry point**: Existing managed-site deploy API/task.

Expected lifecycle:

```text
deploy_site(site_id)
  -> run_deploy_pipeline(site_id)
    -> run_generation_pipeline(site_id, parse_first=true)
      -> parse if needed
      -> generate sidecar job
      -> room compute sidecar job when policy enabled
    -> run_start_pipeline_for_deploy(site_id)
    -> deploy validation
```

### Generation Then Start

**Entry point**: Existing generation admin task or `generate_site(site_id, parse_first)`.

Expected lifecycle:

```text
generate_site(site_id, parse_first)
  -> run_generation_then_start_pipeline(site_id, parse_first)
    -> run_generation_pipeline(site_id, parse_first)
      -> generate sidecar job
      -> room compute sidecar job when policy enabled
    -> run_start_pipeline(site_id)
```

### Parse Only

Parse-only operations MUST NOT trigger room compute.

### Shared Generation Callers

Because `run_generation_pipeline()` is shared, implementations MUST preserve this matrix:

| Caller | Auto room compute |
|---|---|
| Local full deploy | Required when policy enabled. |
| Generate then start | Required when policy enabled. |
| Quick deploy with start | Required through local full deploy behavior. |
| Quick deploy generate-only | Required after generation when policy enabled. |
| Remote deploy local generation | Required for local generated artifacts when policy enabled; remote-side recompute is out of MVP. |
| Parse-only | Forbidden. |
| Start-only | Forbidden. |

## Policy Contract

Room compute MUST be triggered when all are true:

- The lifecycle operation runs generation.
- `generation_enabled(site)` is true.
- `site.gen_spatial_tree == true`.
- Generation succeeds.

Room compute MUST be skipped when any are true:

- The lifecycle operation is parse-only.
- Generation is disabled.
- `site.gen_spatial_tree == false`.
- Generation fails or is cancelled.

Skip events MUST be logged with a reason.

## Sidecar Job Contract

### New Job Kind

`SidecarCliJobKind` MUST include:

```rust
RoomCompute
```

Expected mappings:

| Method | Value |
|---|---|
| `label()` | `房间计算` |
| `key()` | `room-compute` |
| `log_path(site_id)` | dedicated room-compute log path |

### CLI Command

The sidecar job MUST invoke the existing CLI command:

```text
room compute
```

Arguments:

- Use the managed site's generated config path via `config_no_ext`.
- Include `--db-nums` when a derived db scope exists.
- Omit `--db-nums` for full-scope generation.
- Omit `--keywords` in MVP so the CLI uses the site DbOption room keywords.
- Do not pass `--generate-models` by default; generation has already completed.

Recommended examples:

```text
room compute --db-nums 24383
room compute --db-nums 24381 24383
room compute
```

If a future persisted site-level room keyword policy chooses explicit room keywords:

```text
room compute --keywords -RM,-ROOM --db-nums 24383
```

### Capability Contract

If the sidecar binary cannot execute `room compute` with the required spatial-index-backed implementation, the job MUST fail with a capability error. The deploy pipeline MUST treat that as a blocking room-compute failure.

## Status Contract

While the room-compute sidecar job is active:

- Active job tracking MUST report `kind=RoomCompute`.
- Admin task progress SHOULD display `房间计算中`.
- Runtime stage MUST be distinguishable from parse/generate/start, for example `room_computing`.
- Logs MUST include sidecar submitted/running/terminal status lines.
- `tail_log(site_id, "room-compute", limit)` MUST work if tail-log APIs are exposed for existing log kinds.

Suggested progress range:

| Stage | Progress |
|---|---:|
| generation running | 40-70 |
| room compute running | 70-85 |
| start running | 85-95 |
| deploy validation | 95-100 |

## Failure Contract

If room compute returns `success=false`:

- Full deploy MUST fail.
- Generation-then-start MUST fail before start is considered complete.
- `last_error` MUST include a room-compute-specific message.
- Admin task failure MUST identify room compute as the failed stage.
- The site MUST NOT be marked as successful deploy due to a later start/validation step.

If sidecar HTTP polling fails but terminal websocket event is available, existing terminal-event fallback behavior MAY be reused.

## Completion Marker Contract

Every automatic room-compute run MUST write a completion marker under the site runtime directory.

Minimum marker fields:

```json
{
  "site_id": "example",
  "started_at": "2026-06-15T00:00:00Z",
  "finished_at": "2026-06-15T00:00:30Z",
  "success": true,
  "sidecar_job_id": "job-id",
  "scope_source": "generate_db_nums",
  "db_nums": [24383],
  "config_no_ext": "runtime/admin_sites/example/DbOption-generate",
  "report_json_path": "runtime/admin_sites/example/room-compute-report.json",
  "error": null
}
```

Deploy validation MUST fail when auto room compute was expected but the marker is missing or records `success=false`.

Deploy validation MUST NOT require non-zero relation row counts because a valid scope can contain no matching rooms.

## Cancellation Contract

If a user stops/cancels a site while room compute is active:

- Existing active sidecar job cancellation MUST include `RoomCompute`.
- The room-compute job MUST be unregistered after terminal status or cancellation timeout.
- The lifecycle operation MUST not continue to start after cancellation.

## Log Contract

Site log snapshots SHOULD include:

```text
room-compute: 房间计算日志
```

The room-compute log should contain:

- command/scope summary
- sidecar job id/status events
- CLI stdout/stderr
- final success/failure line

## Idempotency Contract

Automatic room compute MUST be safe to run repeatedly for the same site and scope.

At minimum, implementation MUST verify one of these before enabling auto trigger:

- Existing `room compute` deletes/replaces `room_relate` and `room_panel_relate` for the computed scope.
- Or auto trigger performs scoped cleanup before invoking `room compute`.
- Or auto trigger is blocked for scopes where stale relation cleanup cannot be guaranteed.

## Backward Compatibility Contract

- Existing `aios-database room compute` CLI behavior remains unchanged.
- Existing `/api/room/compute` behavior remains unchanged.
- Existing parse-only tasks remain unchanged.
- Existing sites with `gen_spatial_tree=false` do not unexpectedly run room compute.
