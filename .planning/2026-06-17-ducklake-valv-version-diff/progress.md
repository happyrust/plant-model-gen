# Progress

Active plan: `2026-06-17-ducklake-valv-version-diff`

## Current Status

Planning updated after user clarified that a component change, such as a `VALV` sample, may impact a containing `BRAN`. No implementation changes have been made.

## Completed

- Ran SigMap query for DuckLake/version diff planning context.
- Checked tool discovery for `plannator`; no callable plannator-specific tool was exposed in the current environment.
- Read the repository's existing plannotator-style plan format.
- Reviewed relevant repository areas:
  - Parquet model export
  - DuckLake model writer
  - transform DuckLake stub
  - delivery-unit noun definitions
  - current `version_management` module
- Created active planning files.
- Updated the plan to include component-to-delivery-unit impact propagation.

## Next Actions

1. Confirm business identifiers:
   - Is the valve noun in real data `VALV`, `VALE`, or both?
   - Should `EQUIP` always map to `EQUI`?
   - What should identify a release: sesno, task id, timestamp, or manual label?
2. Confirm impact rule:
   - Does any descendant component hash change mark the containing `BRAN` dirty?
   - Are there attribute-only changes that should not affect BRAN version state?
3. Implement Phase 1 release registration.
4. Register two real or controlled Parquet exports.
5. Implement Phase 3 component fingerprinting.
6. Implement Phase 4 component diff CLI with old/new unit membership.
7. Implement Phase 5 component impact CLI and prove `VALV -> BRAN` propagation.
8. Record CLI JSON evidence here.

## Evidence Log

No command/output evidence yet. Add real CLI and HTTP evidence during implementation.
