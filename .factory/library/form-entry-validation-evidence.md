# Form Entry Validation Evidence

- `frontend_test` should use `npm --prefix /Volumes/DPC/work/plant-code/plant3d-web test` without `--runInBand`; Vitest rejects that Jest-only flag with `CACError: Unknown option --runInBand`.
- `frontend_lint` now points at the real lint script: `npm --prefix /Volumes/DPC/work/plant-code/plant3d-web run lint`.
- Current lint state is still degraded but explicit: the command runs ESLint 9 and reports repo-local violations, including generated bundle `doc/xeokit-sdk.es.js` and several existing `no-explicit-any` issues; this is no longer masqueraded as type-check.
- Cross-repo 3101 -> 3100 proxy evidence already exists in mission handoff `2026-03-09T20-16-40-876Z__align-web-server-port-and-local-proxy-3100-3101__a8217844-f6d1-4ed7-a239-48e1c91ceff4.json`.
- That handoff records browser verification where `http://127.0.0.1:3101/?form_id=FORM-LOCAL-3101...` loaded successfully and browser-side fetches to both `/api/health` and `http://127.0.0.1:3100/api/health` returned HTTP 200 with matching backend payloads.
- Backend embed/open responses now expose a dedicated `data.lineage` object containing stable `form_id`, plus `task_id`, `current_node`, and `status` when an existing task is restored.
