# Quickstart: Validate Site Deploy Room Compute Auto Trigger

This quickstart validates behavior from the running admin service and sidecar logs. Adjust host/port/site id to your local environment.

## Prerequisites

- The web server/admin service is running.
- The sidecar can run `aios-database` CLI jobs.
- The managed site has generation enabled.
- `gen_spatial_tree=true`.
- The site has valid parse/generate inputs.

## 1. Create Or Select A Test Site

Use an existing managed site or create one with:

- `gen_model=true`
- `gen_spatial_tree=true`
- optional scoped `generate_db_nums`

Expected result:

- The site appears in the admin site list.
- Its generated DbOption files can be written.

## 2. Run Full Deploy

Submit the existing deploy action for the site.

Expected lifecycle:

```text
parse if needed
generate
room compute
start
deploy validation
```

Expected logs:

- parse log only if parsing was needed
- generate log with generation sidecar job status
- room-compute log with room sidecar job status
- web/viewer logs only after room compute succeeds

Expected marker:

```text
runtime/admin_sites/<site_id>/room-compute-result.json
```

The marker should record `success=true`, sidecar job id, scope source, db nums, config path, and timing.

## 3. Verify Room Compute Scope

For a scoped site:

- If `generate_db_nums` is non-empty, inspect room-compute log for the same db nums.
- If `generate_db_nums` is empty and `manual_db_nums` is non-empty, inspect room-compute log for `manual_db_nums`.
- If both are empty, verify room compute ran without `--db-nums`.

Expected result:

- The room-compute command is site-scoped and uses the same generated config path as the generation job.
- If no matching rooms exist, the room-compute marker can still be successful; do not fail validation only because relation row count is zero.

## 4. Verify Room Relation Output

After deploy success, verify room relation data exists using the project team's preferred SurrealDB query or room validation flow.

When the chosen test fixture is expected to contain rooms, verify `room_relate` / `room_panel_relate` output. When the fixture may contain no room matches, verify the room-compute marker and CLI report instead.

CLI validation references:

```powershell
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room compute --keywords -RM,-ROOM --db-nums 24383
cargo run --bin aios-database --features ws,sqlite-index,web_server -- room verify-json --input verification/room_compute_validation.json
```

For managed deploy validation, prefer inspecting the deployed site's DB/config/logs rather than recomputing manually.

## 5. Verify Failure Blocks Deploy

Use a test site where room compute prerequisites are missing or intentionally break room compute.

Expected result:

- Deploy task fails during room-compute stage.
- The site is not reported as successful deploy.
- `last_error` and task status identify room compute as the failed stage.

## 6. Verify Runtime And Logs

Poll runtime while room compute is active.

Expected result:

- `current_stage` is `room_computing` or equivalent.
- `current_stage_label` says `房间计算中`.
- `sidecar_job_kind` is `room-compute`.
- `tail_log(site_id, "room-compute", limit)` or the equivalent log API returns room-compute log lines.

## 7. Verify Skip Behavior

Set `gen_spatial_tree=false` or use a parse-only operation.

Expected result:

- Room compute is not triggered.
- Logs include a skip reason for generation flows where applicable.
- Parse-only operations do not mention room compute.

## 8. Verify Shared Caller Behavior

Run or inspect these flows:

- Full local deploy.
- Generate then start.
- Quick deploy with start.
- Quick deploy generate-only.
- Remote deploy local generation.

Expected result:

- Trigger behavior matches the trigger matrix in `spec.md`.
- Remote deploy does not claim remote-side room compute readiness unless a separate remote implementation exists.

## 9. Verify Repeated Scoped Runs

Run generation and room compute twice for the same site with a changed db-number scope.

Expected result:

- Room relations for the computed scope match the latest generated data.
- Stale conflicting relations from the earlier run are not left behind.

## 10. Verify Existing Manual Flows

Run existing documented room CLI/API flows outside managed deploy.

Expected result:

- Existing `room compute`, `room compute-panel`, and `/api/room/compute` behavior remains unchanged.
