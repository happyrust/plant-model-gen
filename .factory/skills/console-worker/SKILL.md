---
name: console-worker
description: Implements plant-model-gen web console migration (Vue 3 + Vuetify SPA under /console/*), including Rust web_server + frontend changes and redirect cleanup.
---

# Console Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for any feature that touches the **web console** surface:
- `web_console/` (Vue/Vite/Vuetify components, router, build output)
- `src/web_server/*` (serving `dist`, SPA fallback, legacy-route redirects, API compatibility for console pages)

## Required Skills

- `agent-browser`: mandatory for UI validation of `/console/*` routes (screenshots, console errors, network checks).

## Work Procedure

1. Read the assigned feature (description + expectedBehavior + verificationSteps) and the mission constraints in mission `AGENTS.md`.
2. Constraints (must follow):
   - Do **NOT** run tests or compile tests (`cargo test`, `cargo check --tests` are forbidden).
   - Validate `web_server` by **running it** + `curl`/`post` + `agent-browser`.
3. Tooling-only exception:
   - If the feature is **tooling/config-only** (e.g. `.factory/services.yaml`, `web_console/tsconfig*`, dependency version alignment) and does not change runtime console behavior, you may skip runtime `web_server` + `agent-browser` steps.
   - In that case, validation must instead include the relevant commands (e.g. `vue-tsc --noEmit`, `npm run build`, `cargo check`) with captured exit codes and observations.
4. Codebase discovery:
   - Use `ace-tool` for initial semantic search; use `rg` only for exact-match follow-ups.
5. Implement changes (as needed):
   - Frontend: update Vue/Vuetify components + router; keep route structure consistent with the validation-contract mapping.
   - Backend: update axum routing, redirects, API envelopes, and static serving under `/console` and `/console/assets`.
6. Build/format/check (no tests):
   - `npm --prefix web_console install` (only if needed)
   - `npm --prefix web_console run build`
   - `cargo fmt`
   - `cargo check --features web_server --bin web_server`
7. Run/verify runtime behavior:
   - Start/ensure `web_server` is up on `http://127.0.0.1:3100` (see `.factory/services.yaml`).
   - If start fails due to \"Address already in use\", run the service healthcheck (`curl -sf http://127.0.0.1:3100/api/status`) and **reuse the existing instance if healthy**.
   - `curl -sI http://127.0.0.1:3100/console | head` and any feature-specific `/api/*`.
   - Validate redirects with `curl -I` when in scope.
8. UI validation with `agent-browser`:
   - Use an isolated session name per feature (e.g. `--session feat-<feature-id>`).
   - Always capture:
     - `agent-browser errors --clear` before flow
     - screenshots for key states
     - `agent-browser errors` after flow (must be empty of uncaught errors)
   - Note: screenshot syntax is `agent-browser screenshot <path>` (no `-o` flag).
9. Commit changes with a clear message (no secrets).
   - Run `git status --porcelain` first.
   - Stage **only** the files relevant to this feature (avoid `git add -A` if the repo has unrelated modifications).
   - If there are large unrelated local changes you cannot safely isolate, return to orchestrator instead of committing them.
   - Note: repo has a global `*.json` ignore; `web_console/tsconfig*.json` is explicitly unignored, but if you add other required JSON configs, confirm they are not ignored (`git check-ignore -v <path>`).

## Example Handoff

```json
{
  "salientSummary": "Integrated Vuetify and rebuilt the console shell using v-app + navigation drawer + app bar; set Vite base to /console/ so built assets load under /console/assets without post-build patching. Verified /console deep links load and refresh, and sidebar navigation stays under /console/* with no uncaught browser errors.",
  "whatWasImplemented": "Updated web_console to install Vuetify and created a standard dashboard layout component; adjusted Vite config for base '/console/' and updated router/nav structure to include placeholder routes for Deployment/Sync/DB/Settings groups. Updated Rust web_server routes to serve dist assets and handle SPA history fallback + legacy redirects as required.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "npm --prefix web_console run build",
        "exitCode": 0,
        "observation": "Vite build succeeded; dist/index.html references /console/assets/*"
      },
      {
        "command": "cargo check --features web_server --bin web_server",
        "exitCode": 0,
        "observation": "web_server compiled successfully"
      },
      {
        "command": "curl -sI http://127.0.0.1:3100/console | head",
        "exitCode": 0,
        "observation": "200 OK text/html"
      }
    ],
    "interactiveChecks": [
      {
        "action": "agent-browser open /console and click each top-level nav group item",
        "observed": "URL stayed under /console/*; screenshots saved; agent-browser errors output empty"
      }
    ]
  },
  "tests": { "added": [] },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The required config file for `web_server` startup is missing/unknown (e.g. no suitable `db_options/DbOption-*.toml`) and you need a decision.
- You cannot make a validation assertion pass without changing scope/contract.
- You discover that a required legacy page has no backend API equivalent and needs product direction.
