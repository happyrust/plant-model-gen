---
name: admin-ui-worker
description: Implements `/admin` static workbench UI changes in plant-model-gen, including shell/layout, form/editor UX, status strip, logs panel, and action guardrails.
---

# admin-ui-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that primarily modify the static admin UI in `plant-model-gen`:
- `src/web_server/static/admin/index.html`
- `src/web_server/static/admin/admin.css`
- `src/web_server/static/admin/admin.js`
- light route/asset touch-ups only when required to make the static UI reachable

## Work Procedure

1. Read the assigned feature, mission `AGENTS.md`, and the assertions listed in the feature's `fulfills` set.
2. Before exact-match searches, use `ace-tool` for the first code retrieval pass.
3. Confirm the current `/admin` structure and identify the smallest set of HTML/CSS/JS changes needed.
4. Implement the UI changes while preserving `/admin` as a server-served static page. Do **not** migrate work into `/console/*`.
5. If you touch Rust route wiring incidentally, keep it minimal and validate the runtime path afterward.
6. Validation requirements:
   - If Rust files changed: run `cargo fmt --all` and `cargo check --features web_server --bin web_server`
   - Start or reuse the mission `web_server` on `127.0.0.1:3333`
   - Use `agent-browser` to validate the assigned `/admin` UI flow
   - Use `curl` for any supporting `/api/admin/*` reads needed to prove UI correctness
7. Capture screenshots for the key states relevant to the feature.
8. Stage only the files relevant to the feature if asked to commit.

## Required Validation Style

- Browser validation is mandatory for UI features.
- Do not claim completion from static code inspection alone.
- Do not use Rust tests for this mission.

## Example Handoff Expectations

Include:
- exact files changed
- commands run and exit codes
- screenshots or browser observations for the required UI states
- any discovered UX/data-contract mismatch that should return to the orchestrator

## Return to Orchestrator When

- The feature needs backend API semantics that are not yet implemented or are ambiguous.
- `/admin` would need to become a console/Vue page to proceed.
- The isolated mission instance on `3333` cannot be made reachable without changing mission boundaries.
