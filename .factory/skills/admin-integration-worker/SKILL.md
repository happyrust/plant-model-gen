---
name: admin-integration-worker
description: Aligns `/admin` browser flows with `/api/admin/*` runtime behavior, including create/edit hydration, polling, log visibility, and end-to-end orchestration validation.
---

# admin-integration-worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that cross the static `/admin` UI and the Rust admin/runtime backend, especially when the proof requires browser interaction plus API/runtime polling.

## Required Skills

- `agent-browser` — mandatory for the browser side of each end-to-end flow.
- `verification-before-completion` — invoke before finishing so browser and API evidence match the final implementation.
- `systematic-debugging` — invoke when browser, runtime, and logs disagree during convergence checks.

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

## Example Handoff

```json
{
  "salientSummary": "Closed the create/edit and lifecycle seams between the `/admin` browser shell and the admin runtime APIs on the isolated 3333 instance.",
  "whatWasImplemented": "Adjusted the browser/API integration so create and edit preserve selection while rehydrating list/detail/runtime/log surfaces from real follow-up reads, and parse/start/stop/delete now converge with the correct busy, success, conflict, and failure states across the status strip, runtime panel, and logs panel.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "curl -X POST http://127.0.0.1:3333/api/admin/sites/{id}/start",
        "exitCode": 0,
        "observation": "Received the expected 202 envelope and confirmed later convergence through runtime/log polling."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Used agent-browser to create a site, edit it, start it, stop it, and exercise delete cancel/success/conflict flows.",
        "observed": "Browser-visible state stayed aligned with `/api/admin/*` payloads and preserved operator context during refresh and diagnostics."
      }
    ]
  },
  "tests": {
    "added": []
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature would require mission boundary changes (different primary port, new services, console revival).
- The environment cannot support the required `/admin` browser + API proof on the isolated instance.
- You uncover a high-risk mismatch between UI expectations and backend contract that needs scope clarification.
