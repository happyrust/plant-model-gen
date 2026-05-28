# Remote Offline Deploy Plan

## Solution Approach

Turn the existing remote deploy prototype into a reliable Linux MVP instead of rebuilding it from scratch. The implementation extends `managed_project_sites` remote deployment, keeps the overwrite deployment model for MVP, adds explicit root/sudo/user-mode execution paths, makes `/offline-deploy` the main UI flow, and introduces a thin site-agent status surface for deployed remote `web_server` instances.

Windows, release-directory deployment, upgrade orchestration, and full data snapshot sync are planned follow-up phases, not MVP completion requirements.

## Ordered Steps

### 1. Baseline And Schema Audit

Touch:
- `src/web_server/models.rs`
- `src/web_server/managed_project_sites.rs`
- `src/web_server/admin_handlers.rs`
- `src/web_server/admin_task_handlers.rs`
- `ui/admin/src/api/sites.ts`
- `ui/admin/src/views/OfflineDeployView.vue`
- `ui/admin/src/views/SiteDetailView.vue`

Work:
- Document the current remote target, remote deploy status, admin task, and offline deploy UI flow.
- Add or confirm fields for plain-text saved SSH password, deploy task association, deploy id, deploy mode, degraded flag, site token, and agent status metadata.
- Keep Windows selectable only as a non-MVP future adapter.

Verification:
- Inspect SQLite schema migration behavior through a running `web_server`.
- Use admin HTTP endpoints to create/list remote targets and confirm saved password and status fields round-trip as intended.

### 2. Remote Execution Modes

Touch:
- `src/web_server/managed_project_sites.rs`

Work:
- Introduce a small remote execution abstraction that classifies targets as `root`, `sudo`, or `user`.
- For root or sudo-capable users, keep the full systemd install path.
- For ordinary users without sudo, skip system-level steps and use generated `start.sh`, `stop.sh`, and `status.sh` scripts under the remote site directory.
- Store user-mode pids and logs under `<remote_site_dir>/runtime/pids` and `<remote_site_dir>/runtime/logs`.
- Ensure repeated deployment stops only previously managed systemd units or pid-file processes.

Verification:
- HTTP preflight reports root/sudo/user mode correctly.
- A root/sudo target reaches full deployment status.
- A non-sudo user target reaches degraded deployment status and exposes clear skipped-step messages.

### 3. Payload Resolution And Preflight

Touch:
- `src/web_server/managed_project_sites.rs`
- `scripts/package/build-windows-bundle.ps1` only if package artifact paths need alignment.

Work:
- Require payload sources for the managed site's SurrealDB/RocksDB data directory, `web_server`, SurrealDB, `resource/surreal`, and viewer/admin static assets.
- Detect mesh, asset, parquet, and cache directories from DbOption/runtime paths and surface them as warnings or requirements without making them automatic payloads yet.
- Keep remote ports fixed by user input; never silently rewrite them.
- Improve preflight checks for port occupancy, disk availability, remote privilege mode, local payload availability, and unsupported Windows execution.

Verification:
- POST remote preflight returns blocking errors for missing required payloads.
- POST remote preflight returns warnings for optional model asset/cache paths.
- Occupied remote ports block deployment with a clear message.

### 4. Remote Deployment Flow

Touch:
- `src/web_server/managed_project_sites.rs`
- `src/web_server/admin_task_handlers.rs`

Work:
- Keep MVP overwrite deployment.
- Upload required payloads by SFTP.
- Generate remote DbOption with remote ports, paths, site identity, deploy id, and site token.
- Install systemd units for full mode or scripts for user-mode degraded mode.
- Start remote services/processes.
- Validate remote HTTP endpoints and classify completion as full or degraded.
- Associate status with the submitted admin task id or deploy id.

Verification:
- POST remote deploy submits an admin task and remote status references the task or deploy id.
- Polling the admin task and remote status shows the expected sequence: preflight, prepare, upload, config, start, validation, completed.
- Redeploying the same site stops only managed services/processes and does not kill unknown port owners.

### 5. UI Consolidation

Touch:
- `ui/admin/src/router/index.ts`
- `ui/admin/src/components/layout/AppHeader.vue`
- `ui/admin/src/views/OfflineDeployView.vue`
- `ui/admin/src/views/SiteDetailView.vue`
- New shared component under `ui/admin/src/components/sites/` or `ui/admin/src/components/deploy/`.

Work:
- Make `/offline-deploy` the default admin landing path.
- Extract the remote deploy form/progress/check list into a shared wizard panel.
- Keep `OfflineDeployView` as the main workflow with site selection.
- Make `SiteDetailView` reuse the shared panel or link into `/offline-deploy` with a selected site.
- Show saved password behavior, full vs degraded completion, skipped privileged steps, and remote health status clearly.

Verification:
- Opening admin after login lands on offline deploy.
- No-site state displays a clear path to create/parse a managed site.
- Site detail and offline deploy use consistent fields and status wording.

### 6. Site Agent Status Surface

Touch:
- `src/web_server/web_listen.rs`
- `src/web_server/admin_handlers.rs`
- New module such as `src/web_server/remote_agent_handlers.rs` or `src/web_server/site_agent_handlers.rs`.
- `src/web_server/mod.rs`
- `ui/admin/src/api/sites.ts` or a new agent API module.

Work:
- Add a thin remote/site agent API separate from remote target configuration.
- Expose a remote status endpoint that reports site identity, deploy id, site token identity, version metadata, runtime mode, degraded flag, ports, database health, viewer health, and coarse disk resources.
- Add local pull-based monitoring from the admin UI or backend to remote web_server URLs.
- Reserve center heartbeat fields such as `center_url`, `heartbeat_interval`, and `site_token`, but do not implement upgrade/sync task dispatch in MVP.

Verification:
- Remote `web_server` returns agent status after deployment.
- Local admin can fetch and display the deployed site's status.
- Agent status includes enough metadata to support later upgrade and full snapshot sync planning.

### 7. Real Remote Smoke

Touch:
- Documentation under `goals/remote-offline-deploy/` or `docs/plans/`.
- Optional smoke script under `runtime/` or `scripts/` if useful.

Work:
- Run local `web_server` and use HTTP/POST API calls instead of tests.
- Verify Ubuntu22 full deployment.
- Verify user-mode degraded deployment with an ordinary non-sudo user.
- Verify CentOS7.9 only when a real CentOS7.9 target is available; do not claim CentOS completion before that smoke passes.
- Record request bodies, response JSON, remote URLs, remote health results, and known failures.

Verification:
- Remote `/api/status` passes.
- Remote database connection check passes.
- Remote `/api/site/identity` matches the deployed site id.
- Remote viewer/admin static resources are reachable.
- No `cargo test` is run.

## Risks And Open Questions

- Plain-text SSH password storage is accepted for the current test phase but is not production-safe.
- CentOS7.9 binary compatibility may fail because of older glibc/OpenSSL or SurrealDB builds.
- User-mode degraded deployment may be externally unreachable if firewall changes require privilege.
- Existing site-scoped remote status can still overwrite history; full task-scoped history is a later hardening step.
- Artifact/release deployment is required after the MVP to support clean rollback and upgrades.
