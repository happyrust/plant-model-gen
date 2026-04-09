---
name: admin-integration-worker
description: Aligns `/admin` browser flows with `/api/admin/*` runtime behavior, including create/edit hydration, polling, log visibility, and end-to-end orchestration validation.
---

# admin-integration-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that cross the static `/admin` UI and the Rust admin/runtime backend, especially when the proof requires browser interaction plus API/runtime polling.

## Work Procedure

1. Read the feature, mission `AGENTS.md`, `.factory/services.yaml`, and the feature's `fulfills` assertions.
2. Use `ace-tool` first to gather the relevant UI + API + runtime files.
3. Identify the end-to-end seam under change:
   - create -> refresh -> auto-select -> hydrate
   - edit -> refresh -> draft reset
   - parse/start/stop/delete -> runtime/log polling -> visible UI state
   - auto-refresh cadence and conflict/error surfacing
4. Make the minimum cross-surface changes required.
5. Required validation:
   - run or reuse the mission `web_server` on `127.0.0.1:3333`
   - use `agent-browser` for the browser flow
   - use `curl` / POST requests to capture the matching backend evidence
   - when a feature involves async actions, collect both the triggering response and the eventual runtime/log convergence
6. Capture at least one success-path observation and, when the feature claims resilience/guardrails, one denial/conflict observation.
7. Clean up validation data when practical (for example delete temporary test sites created only for proof).

## Validation Expectations

- Browser-only proof is insufficient for integration features.
- API-only proof is insufficient for operator-facing orchestration features.
- Do not use Rust tests for this mission.

## Return to Orchestrator When

- The feature would require mission boundary changes (different primary port, new services, console revival).
- The environment cannot support the required `/admin` browser + API proof on the isolated instance.
- You uncover a high-risk mismatch between UI expectations and backend contract that needs scope clarification.
