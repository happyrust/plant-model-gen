# Quickstart: Validate Deployment Project Identity Over E3D Collection

## Goal

Verify the deployment name is the sole outward identity over a multi-E3D collection, E3D names stay source-only and independent, a coincidence warning appears when names match, and a guard prevents regressions.

## Prerequisites

- Runnable admin web_server with its own working directory.
- A deployment can be created with deployment name `9002` and E3D projects `AvevaPlantSample` (design, primary) + `AvevaCatalogue` (library).
- Admin token for protected endpoints.
- Do NOT use `cargo test` / Rust test-target for web_server validation.

## Scenario 1: Deployment name is the sole outward identity

1. Create the deployment named `9002` with the two E3D projects.
2. Trigger config generation (preview or deploy).
3. Inspect generated DbOption and runtime layout.

Expected:

```toml
project_name = "9002"
included_projects = ["AvevaPlantSample", "AvevaCatalogue"]
```

- runtime/output directories and viewer `output_project` all use `9002`.

## Scenario 2: E3D names are source-only and independent

1. Inspect `included_projects`/`project_dirs` from Scenario 1.

Expected: E3D names/paths present and independent of the deployment name; no outward-identity surface uses an E3D name.

## Scenario 3: Coincidence warning

1. Create or edit a deployment whose name equals an E3D source name (for example `AvevaPlantSample`).

Expected: operation succeeds and the response/UI shows a non-blocking warning about the name coincidence; no functional conflict.

## Scenario 4: Uniqueness

1. Attempt to create a second deployment with a `project_name` that normalizes to an existing deployment's name.

Expected: rejected with a clear uniqueness error, consistent across create/edit/clone/quick-deploy.

## Scenario 5: Regression guard

1. Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/guard/deployment_identity_guard.ps1
```

Expected: passes on the current codebase.

2. Temporarily switch an outward-identity path (e.g., `build_viewer_url`) to the source-name helper and re-run.

Expected: guard fails and names the offending location. Revert the change.

## Scenario 6: Presentation consistency

1. Open details and preview for the multi-E3D deployment.

Expected: deployment name shown as outward identity; E3D projects listed as the collection; consistent across surfaces.

## Evidence To Record

- Generated `project_name` / `included_projects` / `project_dirs`.
- Active output namespace path.
- Warning text when names coincide.
- Uniqueness rejection message.
- Guard pass/fail output.

## Recorded Validation Evidence

### Scenario 5: Regression guard (2026-06-16)

- `scripts/guard/deployment_identity_guard.ps1` on current code: `[PASS] deployment identity guard passed (3 outward-identity functions verified)`, exit 0.
- Negative test: temporarily switched `build_viewer_url` to `site_source_project_name(site)`; guard output: `FAILED: outward-identity function uses E3D source name. build_viewer_url references site_source_project_name at ...managed_project_sites.rs:10133`, exit 1. Change reverted; guard PASS again, exit 0.

Scenarios 1-4 and 6 (HTTP/UI flows) still require a running web_server and are pending live validation.
