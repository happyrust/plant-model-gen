# Implementation Plan

## Approach

Two independent strands sharing one spec: (a) make the dbnum precheck implementable by
persisting the scan-provided dbnums on `SiteProject`, then implement it with the same
"one dbnum, many owners" rule the sidecar scanner uses; (b) extract the IP utility to
`src/shared/net_util.rs` and normalize failure fallbacks by call-site class.

## Files

### Strand A — dbnum precheck (W-1)

- `src/web_server/models.rs`
  - `SiteProject`: add `#[serde(default)] pub dbnums: Vec<u32>`.
  - Fix the `ScanProjectsResult::conflicts` doc comment (line ~943) to describe real
    behavior once the precheck exists.
- `src/web_server/managed_project_sites.rs`
  - Implement `precheck_dbnum_conflicts`: build `HashMap<u32, Vec<&str>>` over
    projects with non-empty `dbnums`; bail with a 409-mapped error listing each
    dbnum and its owning projects (message format mirrors sidecar scan conflicts).
  - Confirm both call sites (create ~L3282, update ~L4053) pass through unchanged.
- `ui/admin/src/components/sites/*` (site create/edit forms)
  - Forward `dbnums` from `ScannedProject` into the `SiteProject` payload.
- `ui/admin/src/types/site.ts`
  - Add optional `dbnums?: number[]` to the site project type.

### Strand B — IP utility consolidation (NEW-2/3)

- `src/shared/net_util.rs` (new)
  - Move `get_local_ip_via_udp` (env-override list + UDP probe) and
    `is_loopback_or_unspecified_host` here; export from `src/shared/mod.rs`.
- `src/web_server/mod.rs`
  - Re-export from shared (keep `pub use` so existing `super::get_local_ip_via_udp`
    call sites compile unchanged).
- `src/web_api/platform_api/config.rs`
  - Delete the copied block (~L68-160); import from `crate::shared::net_util`.
- Fallback normalization (display-URL class):
  - `src/web_server/managed_project_sites.rs:505` (`default_access_host`): return
    `127.0.0.1` instead of `0.0.0.0`, keep the warn.
  - `src/web_server/handlers.rs:2463/2773/2973`, `database_diagnostics.rs:344`,
    `db_startup_manager.rs:381/452`, `database_status_handlers.rs:427`,
    `models.rs:284`, `wizard_handlers.rs:484`: replace `unwrap_or_default()` with
    a `local_ip_or_loopback()` helper from `net_util` (returns `127.0.0.1` + warn).
  - Keep 503 paths (`site_config_handlers.rs:73`, handlers SERVICE_UNAVAILABLE
    sites) and `bin/web_server.rs` bind-ip fallback as-is.

## Risks

- Frontend may have other payload assembly points (presets, quick deploy) that build
  projects without dbnums — they fall into the skip path by design (Req 3).
- `web_server/mod.rs` re-export keeps the diff small; a later cleanup can migrate
  call sites to `crate::shared::net_util` directly.

## Validation

- Static inspection + `rg` for remaining `unwrap_or_default()` on the IP helper and
  for duplicate definitions.
- Manual: create conflicting-dbnum site → 409; legacy site update → OK; offline-mode
  display URLs contain `127.0.0.1`.
- `cargo fmt`; `npm run build` under `ui/admin` for the type change. No Rust tests,
  per repository rule.
