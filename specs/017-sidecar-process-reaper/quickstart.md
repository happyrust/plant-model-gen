# Quickstart: Validate Sidecar Process Lifecycle Reaper

## Goal

Verify that sidecars no longer accumulate: owned sidecars are terminated on parent exit (graceful, crash, hard kill), stale sidecars are reaped on startup, all key kinds are covered, idle sidecars self-exit, and reaping never touches other instances.

## Prerequisites

- A runnable web_server (source or packaged) with its own working directory and admin_sidecars root.
- Ability to enumerate processes (PowerShell `Get-CimInstance Win32_Process`).
- Do NOT use `cargo test` / Rust test-target for web_server validation.

## Helper: count owned sidecars

```powershell
$root = (Resolve-Path ".\runtime\admin_sidecars").Path
Get-CimInstance Win32_Process -Filter "Name='aios-database.exe'" |
  Where-Object { $_.CommandLine -match 'serve' -and $_.CommandLine -like "*${root}*" } |
  Select-Object ProcessId, CommandLine
```

## Scenario 1: Graceful exit reaps owned sidecars

1. Start the web_server instance.
2. Trigger operations that spawn sidecars (e.g., site preview / db-index rebuild / resolve).
3. Confirm owned sidecars exist (helper above).
4. Stop the web_server gracefully.
5. Expected: owned sidecar count drops to 0.

## Scenario 2: Hard kill is covered by OS binding

1. Start the instance and spawn sidecars.
2. Hard-kill the web_server: `Stop-Process -Id <web_server pid> -Force`.
3. Expected: owned sidecars are terminated by the Job Object (Windows) without manual cleanup.

## Scenario 3: Startup reaper clears prior-run orphans

1. Spawn sidecars, then hard-kill the web_server while (for the test) the OS binding is disabled or unavailable, leaving orphans.
2. Start the same instance again.
3. Expected: startup reaper logs `phase=startup` with a non-zero `killed`, and owned sidecar count returns to a clean baseline.
4. Repeat start/kill/start several times.
5. Expected: steady-state owned sidecar count does not grow across restarts.

## Scenario 4: Cross-instance safety

1. Start instance A (cwd A) and instance B (cwd B), each spawning sidecars.
2. Restart or stop instance A.
3. Expected: instance B's sidecars remain untouched; A's reaper logs only A's `scope_root`.

## Scenario 5: All key kinds reaped

1. Trigger sidecars of kinds `site`, `job`, `db-index`, `resolve`, `scan`, `preview`, `mdb`.
2. Stop the instance.
3. Expected: shutdown reaper `by_kind` shows all kinds terminated; helper count is 0.

## Scenario 6: Idle self-shutdown

1. Spawn a serve sidecar with a short `--idle-timeout-secs` (e.g., 30) for the test.
2. Leave it idle past the timeout.
3. Expected: the sidecar exits on its own.

## Scenario 7: Maintenance sweep of pre-existing orphans

1. With orphans present under a known root, run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/cleanup_orphan_sidecars.ps1 -Root .\runtime\admin_sidecars
```

2. Expected: only orphans under that root are terminated; the script prints pid/key/runtime_dir and a final count.

## Evidence To Record

- Owned sidecar counts before/after each scenario.
- Reaper log lines (`phase`, `scope_root`, `scanned`, `killed`, `by_kind`).
- Confirmation that a second instance's sidecars were untouched.
- Idle sidecar exit observation.
- Maintenance script output.
