# Implementation Plan

## Approach

Keep the existing review DB helper API stable while changing its behavior from per-call physical WebSocket creation to per-call cloned SurrealDB session. Remove the repeated middleware context switch and start the existing `aios_core` heartbeat once after the main SurrealDB connection succeeds.

## Files

- `src/web_api/review_db.rs`
  - Convert review primary DB initialization to async `OnceCell`.
  - Add `review_db_session()` as the canonical session helper.
  - Make `fresh_review_db()` delegate to `review_db_session()` for compatibility.
  - Make `ensure_review_primary_db_context()` initialization-only.
- `src/web_server/mod.rs`
  - Add a process-level heartbeat guard.
  - Start `aios_core::spawn_heartbeat` once when WebSocket SurrealDB connection succeeds.

## Validation

- Static compile check for `web_server` without test targets.
- Optional HTTP smoke through the running web server if local SurrealDB is available.
- Verify logs no longer include `operation=review.ensure_context` for review request entry.
