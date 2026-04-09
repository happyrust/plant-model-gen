# User Testing

User-testing guidance for the `/admin` local site-management workbench mission.

## Validation Surface

### Browser surface (primary)
- Base URL: `http://127.0.0.1:3333/admin`
- Tool: `agent-browser`
- Validate:
  - standalone `/admin` shell and workbench layout
  - create/edit selection flows
  - status strip, runtime summary, disabled-state matrix, and delete confirmation
  - parse/db/web log tab behavior
  - busy-state and failure-state operator feedback

### API surface (primary)
- Base URL: `http://127.0.0.1:3333`
- Tool: `curl` / POST requests
- Validate:
  - `/api/admin/sites`
  - `/api/admin/sites/{id}`
  - `/api/admin/sites/{id}/runtime`
  - `/api/admin/sites/{id}/logs`
  - create/update/parse/start/stop/delete envelopes and status codes

### Runtime boundary (supporting)
- Tool: `curl` polling
- Validate:
  - action accept vs terminal state
  - `/api/status` readiness on started sites
  - runtime/log convergence
  - no false-positive running status from unrelated port occupants

## Validation Concurrency

### Browser validators
- Max concurrent validators: **1**
- Rationale: memory is tight and admin flows are stateful/polling-heavy.

### API/runtime validators
- Max concurrent validators: **1** for lifecycle flows
- Rationale: parse/start/stop mutate shared local runtime resources and ports.

## Flow Guidance

- Always validate against the isolated `3333` instance.
- A `202` action response is only the trigger, not success proof.
- Collect both browser evidence and matching API/runtime/log evidence for lifecycle flows.
- Preserve operator context during refresh checks: selected site and active log tab should remain stable while fresh data rehydrates.
