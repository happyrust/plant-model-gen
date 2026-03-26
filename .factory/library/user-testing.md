# User Testing

User-testing guidance for the **web console (Vue SPA) migration**.

---

## Validation Surface

### Browser surface (primary)

- URL: `http://127.0.0.1:3100/console`
- Tool: `agent-browser`
- What to check:
  - deep-link refresh (history mode)
  - navigation drawer grouping + reachability
  - no uncaught console errors (`agent-browser errors`)
  - Network has no failed requests (focus on `/console/assets/*` + key `/api/*`)

### API surface (supporting)

- Tool: `curl`
- What to check:
  - `/api/status` basic health
  - feature-specific `/api/*` endpoints referenced by the SPA
  - legacy route redirects via `curl -I`

---

## Validation Concurrency

### Browser validators (agent-browser)

- Max concurrent validators: **2–3**

Rationale (planning dry run): machine is 10 cores / 16 GiB; a single agent-browser session adds multiple Chrome / devtools processes. Keep headroom.

### API-only validators (curl)

- Max concurrent validators: **5**

---

## Known Quirks / Gotchas

- `agent-browser screenshot` syntax is `agent-browser screenshot <path>` (no `-o`).
- Some endpoints (e.g. incremental) may be mock/TODO in backend; parity requirements are “UI consistent and no crash” unless the feature explicitly implements real behavior.
- Port conflicts are common. Always stop by port if needed (see `.factory/services.yaml`).

## Flow Validator Guidance: browser

- Assigned browser validators must stay on `http://127.0.0.1:3100/console*` and only use the local `web_server` on port `3100`.
- Use a dedicated session name per validator; never use the default browser session.
- Stay within read/interaction validation boundaries: navigate, reload, click drawer items, inspect console/network, and capture screenshots/evidence. Do not edit application state unless the assigned assertions explicitly require it.
- Avoid overlapping global-state mutations across concurrent validators. For console-foundation, browser groups are read-mostly and safe to run concurrently when they only navigate and inspect routing/shell behavior.
- Collect `agent-browser errors --clear` before each flow and `agent-browser errors` after the flow. Save screenshots into the assigned evidence directory.

## Flow Validator Guidance: api

- API validators should target only the local `web_server` at `http://127.0.0.1:3100`.
- Prefer `curl` for GET/HEAD/POST checks and capture headers/body snippets needed by the assertion evidence.
- Keep requests scoped to assigned assertions. For console-foundation, API checks are read-only except for harmless non-GET negative tests that verify redirect behavior.
- API validators may run concurrently as long as they do not mutate shared application state outside their assigned negative-test coverage.
- Record exact status codes, content types, and redirect `Location` headers in the flow report.

