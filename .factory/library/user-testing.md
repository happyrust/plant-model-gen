# User Testing

Manual testing surface, browser entry points, and local validation notes.

**What belongs here:** URLs, tools, setup steps, known quirks, browser/manual validation guidance.

---

## Local Testing Surface

- Backend API: `http://127.0.0.1:3100`
- Frontend app: `http://127.0.0.1:3101`

## Core Manual Flow

1. Start backend on `3100`.
2. Start frontend on `3101`.
3. Open the frontend with embedded parameters including at least:
   - `form_id`
   - `user_token`
   - `user_id`
   - `project_id`
4. Verify role-based landing.
5. Save draft or review data.
6. Submit or return the task.
7. Reopen with the same `form_id`.
8. Trigger model location from task/opinion/attachment context.

## Evidence Expectations

- Capture browser screenshots for landing state, saved state, reopened state, and located model state.
- Capture network evidence showing the same `form_id` across the full flow.
- Record any degraded external sync behavior separately from local workflow success.

## Known Quirks

- Existing code previously defaulted to a non-mission backend port; mission work is expected to align local validation to `3100`.
- Some external sync behavior may degrade to mock behavior locally; this does not excuse failures in local workflow state, persistence, or viewer location.
