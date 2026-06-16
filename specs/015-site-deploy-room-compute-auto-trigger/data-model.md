# Data Model: Site Deploy Room Compute Auto Trigger

## Existing Entities Reused

### ManagedProjectSite

Existing site record used as the source of truth for trigger and scope.

Relevant fields:

- `site_id`: Stable managed-site identity.
- `project_name`: Human-facing project identity; not directly used for room-compute policy.
- `manual_db_nums`: Parse/manual db scope fallback.
- `generate_db_nums`: Preferred generation db scope for room compute.
- `manual_refnos`: Root-model generation hints; not mapped to room-compute root in MVP.
- `gen_model`: Generation option.
- `gen_mesh`: Generation option.
- `gen_spatial_tree`: Spatial generation option and MVP auto-room-compute prerequisite.
- `pipeline_db_mode`: DB mode used for parse/generate pipeline.
- `config_path`, `runtime_dir`, `db_data_path`: Generated config/runtime paths.
- `status`, `parse_status`, `last_error`: Runtime status surfaces.

No mandatory new persisted field is required for MVP.

## Derived Entities

### AutoRoomComputePolicy

Derived at runtime from `ManagedProjectSite`.

| Field | Type | Source | Description |
|---|---|---|---|
| `enabled` | bool | `generation_enabled(site) && site.gen_spatial_tree` | Whether auto room compute should run after generation. |
| `skip_reason` | Option<String> | derived | Human-readable reason when skipped. |
| `failure_blocks_deploy` | bool | constant true for MVP | Whether room-compute failure fails the lifecycle operation. |
| `config_no_ext` | String | `generation_config_path(site_id)` without `.toml` | Site-scoped DbOption path passed to sidecar CLI. |
| `cwd` | String | repository root | CLI working directory used by sidecar. |
| `requires_sqlite_index` | bool | constant true for MVP | Whether the sidecar binary must support spatial-index-backed room compute. |
| `caller_flow` | enum | pipeline caller | Flow classification used for audit/logging: local deploy, generate-start, quick deploy, remote local-generation, etc. |

Suggested helper:

```rust
fn auto_room_compute_policy(site: &ManagedProjectSite) -> AutoRoomComputePolicy
```

### RoomComputeScope

Derived db-number scope for `room compute`.

| Field | Type | Source | Description |
|---|---|---|---|
| `db_nums` | Option<Vec<u32>> | `generate_db_nums`, else `manual_db_nums` | `None` means full scope and omits `--db-nums`. |
| `source` | enum | derived | `GenerateDbNums`, `ManualDbNums`, or `FullScope`. |
| `manual_refnos_present` | bool | `!manual_refnos.is_empty()` | Logged for diagnosis; not mapped to `--refno-root` in MVP. |

Suggested helper:

```rust
fn room_compute_scope(site: &ManagedProjectSite) -> RoomComputeScope
```

CLI argument mapping:

- Always start with `["room", "compute"]`.
- Prefer omitting `--keywords` in MVP so CLI uses the site DbOption room keywords.
- Add `["--keywords", ...]` only after a future persisted site-level room keyword policy exists.
- If `db_nums=Some(nums)`, append `["--db-nums", nums_as_comma_or_repeated_args]` according to existing Clap parsing support.
- Optionally append `["--report-json", <runtime report path>]` if implementation adds a machine-readable report.

### RoomComputeSidecarJob

Runtime representation layered on existing `ActiveSidecarJob`.

| Field | Type | Source | Description |
|---|---|---|---|
| `kind` | `SidecarCliJobKind::RoomCompute` | new enum variant | Distinguishes room-compute jobs from parse/generate. |
| `key` | String | `room-compute:<site_id>` or equivalent | Sidecar job key. |
| `job_id` | String | sidecar response | Active sidecar job id. |
| `status` | String | sidecar status/events | `submitted`, `queued`, `running`, `succeeded`, `failed`, `cancelled`. |
| `log_path` | PathBuf | `room_compute_log_path(site_id)` | Dedicated room-compute log. |

### RoomComputeRuntimeStage

Observable status derived from active sidecar job.

| Field | Type | Description |
|---|---|---|
| `current_stage` | String | Suggested value: `room_computing`. |
| `progress_label` | String | Suggested user-facing label: `房间计算中`. |
| `progress_percent` | f32 | Suggested range: 70-85 before startup. |

### RoomComputeCompletionMarker

Runtime JSON artifact written for automatic room-compute runs.

Suggested path:

```text
runtime/admin_sites/<site_id>/room-compute-result.json
```

| Field | Type | Description |
|---|---|---|
| `site_id` | String | Managed site id. |
| `started_at` | String | RFC3339 start time. |
| `finished_at` | String | RFC3339 finish time. |
| `success` | bool | Sidecar/CLI result. |
| `sidecar_job_id` | String | Submitted sidecar job id. |
| `scope_source` | String | `generate_db_nums`, `manual_db_nums`, or `full_scope`. |
| `db_nums` | Option<Vec<u32>> | Actual db nums passed to CLI. |
| `config_no_ext` | String | Site-scoped config used by CLI. |
| `report_json_path` | Option<String> | Optional CLI report path. |
| `error` | Option<String> | Failure summary when unsuccessful. |
| `relation_stats` | Option<Object> | Optional parsed stats from CLI report. Zero rows can still be successful. |

## State Transitions

```text
Parsed or Draft
  -> Generating
  -> Generated/Parsed
  -> RoomComputing
  -> Parsed ready for start
  -> Starting
  -> Running
```

Failure transitions:

```text
Generating failed -> Failed
RoomComputing failed -> Failed
Starting failed -> Failed
```

Skipped transition:

```text
Generated + auto policy disabled -> Starting
```

## Validation Rules

- Auto room compute is enabled only when generation is enabled and `gen_spatial_tree=true`.
- Room compute must use a site-scoped config path.
- Room compute must fail the lifecycle operation if sidecar returns `success=false`.
- Skips must be logged.
- Active room-compute jobs must be cancelable through existing sidecar active-job cancellation paths.
- Deployment validation must use `RoomComputeCompletionMarker` when auto room compute was expected.
- Marker success must not require non-zero room relation counts.
- Existing relation persistence must be idempotent for the computed scope; if not, implementation must clean the scope before compute.
- Capability failure from a sidecar binary without room compute/spatial-index support is a blocking failure.

## Future Persisted Fields

Not required for MVP, but reserved for future requirements:

- `auto_room_compute_after_generate: bool`
- `room_compute_failure_policy: "block" | "warn"`
- `room_compute_keywords: Vec<String>`
- `last_room_compute_started_at`
- `last_room_compute_finished_at`
- `last_room_compute_duration_ms`
