# Requirements Checklist: Room Tree Compute And Display

**Purpose**: Validate specification quality before runtime verification.

**Created**: 2026-06-22

**Feature**: `specs/020-room-tree-compute-display/spec.md`

## Content Quality

- [x] No implementation code appears in `spec.md`
- [x] User value and display purpose are clear
- [x] Current static-code conclusion is separated from runtime validation status
- [x] Missing room compute is documented as an expected state
- [x] Validation avoids forbidden `cargo test` workflow for `web_server`

## Requirements Completeness

- [x] Functional requirements cover backend route exposure
- [x] Functional requirements cover backend hierarchy and virtual node IDs
- [x] Functional requirements cover frontend API-base routing
- [x] Functional requirements cover tab-scoped room tree initialization
- [x] Functional requirements cover lazy loading and viewer operations
- [x] Functional requirements cover missing/failing room data behavior
- [x] Success criteria are measurable through HTTP/frontend smoke checks
- [x] Key entities are identified

## Contract Completeness

- [x] Root endpoint contract is documented
- [x] Children endpoint contract is documented for each hierarchy level
- [x] Ancestors endpoint contract is documented
- [x] Search endpoint contract is documented
- [x] Failure response shapes are documented
- [x] ID normalization compatibility note is documented

## Runtime Verification Still Required

- [ ] Validate all `/api/room-tree/*` endpoints against a running backend with computed room relations
- [ ] Capture actual JSON serialization shape for `RefnoEnum` IDs
- [ ] Validate `plant3d-web` room tab against the same backend
- [ ] Validate missing-room-compute behavior against a site without `room_relate`

## Notes

Static review indicates `plant3d-web` has the code path needed to display the room tree, but runtime validation is still required because room tree visibility depends on computed `room_relate` data and correct backend API routing.
