# Feature Specification: Site Input Validation and Local-IP Utility Consolidation

## User Need

Three findings from `docs/code-review-2026-06-10/INCREMENTAL_REVIEW.md`:

- **W-1**: `precheck_dbnum_conflicts` has been an empty stub through two review rounds;
  `models.rs:943` even documents a safety net that does not exist. Root cause: the
  persisted `SiteProject` carries no `dbnums`, so the check cannot be implemented
  against current data.
- **NEW-2**: `get_local_ip_via_udp` + `is_loopback_or_unspecified_host` were copied
  wholesale into `src/web_api/platform_api/config.rs` because `web_api` must not
  depend on `web_server`. Two implementations will drift.
- **NEW-3**: on IP-inference failure (offline host without default route) call sites
  diverge into four behaviors, two of which produce broken URLs (`http://:5173`,
  `http://0.0.0.0:port`).

## Scope

- `SiteProject` persisted model and the create/update validation path.
- A shared local-IP utility module under `src/shared/`.
- All `get_local_ip_via_udp` call sites' failure fallbacks.

## Requirements

1. `SiteProject` gains an optional `dbnums: Vec<u32>` field (serde default, backward
   compatible with existing `projects_json` rows); the admin UI forwards `dbnums`
   from scan results when composing create/update requests.
2. `precheck_dbnum_conflicts` reports HTTP 409 with the conflicting dbnum → project
   list when the same dbnum appears in more than one project of the site, matching
   the sidecar scan's conflict definition.
3. Projects lacking `dbnums` data (legacy rows, hand-entered paths) are skipped
   without error — validation tightens progressively, never blocks old data.
4. The stale comment in `models.rs` is corrected to describe the real behavior.
5. One canonical implementation of `get_local_ip_via_udp` /
   `is_loopback_or_unspecified_host` lives in `src/shared/`; `web_server` and
   `web_api` both consume it; the copy in `platform_api/config.rs` is deleted.
6. Failure fallbacks unify by call-site class:
   - display URLs (access address, viewer URL): fall back to `127.0.0.1` + one
     `tracing::warn!` advising `AIOS_PUBLIC_HOST`; never emit empty-host or
     `0.0.0.0` URLs;
   - liveness probes: probe `127.0.0.1`;
   - explicit IP-query APIs: keep returning 503;
   - bind-address paths: keep falling back to the configured bind IP.

## Non-Goals

- No re-scan or sidecar round-trip inside the create/update transaction (validation
  uses the dbnums snapshot provided by the client).
- No parse-time conflict gate (sidecar preview/plan already surfaces
  `DBNUM_CONFLICT` downstream).
- No IPv6 support changes in the IP utility.
- Do not run Rust tests or compile test targets.

## Acceptance Criteria

- Creating a site whose two projects share a dbnum returns 409 naming the dbnum and
  both projects; removing the overlap lets creation succeed.
- A legacy site (no dbnums in `projects_json`) updates without validation errors.
- `rg "fn get_local_ip_via_udp" src/` yields exactly one definition (in `src/shared/`).
- With IP inference forced to fail, no rendered URL contains `http://:` or
  `http://0.0.0.0`; display URLs show `127.0.0.1` and the warn log appears once.
