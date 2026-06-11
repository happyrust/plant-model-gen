# Tasks

## Strand A — dbnum precheck (W-1)

- [x] Add `dbnums` (serde default) to `SiteProject`; fix the stale doc comment in `models.rs`.
- [x] Implement `precheck_dbnum_conflicts` (multi-owner dbnum → 409 with details; per-project dedupe).
- [x] Forward `dbnums` from scan results in admin UI payloads (`types/site.ts` + `SiteDrawer.vue`).
- [ ] Manually verify: conflict 409, legacy-data skip path.

## Strand B — IP utility (NEW-2/3)

- [x] Create `src/shared/net_util.rs`; move `get_local_ip_via_udp` + `is_loopback_or_unspecified_host`.
- [x] Re-export from `web_server/mod.rs`; delete the copy in `platform_api/config.rs`.
- [x] Add `local_ip_or_loopback()` helper; replace display-URL `unwrap_or_default()` / `0.0.0.0` fallbacks
      (handlers ×3, site_registry, db_startup_manager ×2, database_status_handlers,
      database_diagnostics, models, wizard_handlers, managed_project_sites `default_access_host`,
      platform_api `access_host_from_bind_host`).
- [x] Verify single definition via `rg` (only `src/shared/net_util.rs`).

## Closeout

- [x] Format changed Rust files.
- [ ] Rebuild `ui/admin` — blocked by pre-existing breakage: `data-browser-surreal.ts`
      imports `surrealdb` which is not declared in `package.json` (ghost dependency,
      unrelated to this spec). Type-level changes verified clean via vue-tsc language server.
- [x] Update CHANGELOG.
