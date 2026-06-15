# Implementation Plan: BRAN Scoped Generation

**Branch**: `002-bran-flow-direction` | **Date**: 2026-06-15 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/013-bran-scoped-generation/spec.md`

## Summary

Add a quick-deploy fast-test mode that accepts a `target_root_refno` for a BRAN, validates it before generation, scopes generation/export to that BRAN subtree, and returns a frontend viewer URL that loads the same BRAN with MBD pipe annotation enabled. Existing full quick-deploy behavior remains unchanged when no scoped target is provided.

## Technical Context

**Language/Version**: Rust backend with Axum web server and existing async generation pipelines; frontend validation uses the existing plant3d-web viewer URL contract.

**Primary Dependencies**: `QuickDeployTestRequest/Response`, `managed_project_sites::quick_deploy`, site generation pipelines, existing refno parsing, project/db metadata, descendant expansion, Parquet export, and frontend `show_refno`/`mbd_refno` URL parameters.

**Storage**: Existing managed site runtime directories and generated Parquet/model output. Scoped target state must be request-scoped and stored only in site/task metadata when needed for that request.

**Testing**: No `cargo test`. Backend validation uses running web_server + HTTP/POST. CLI-only checks use command + JSON output. Frontend validation uses plant3d-web URL automation.

**Target Platform**: Windows/local managed site quick deploy, with compatibility for existing runtime/admin site layouts.

**Project Type**: Rust web server + model generation backend, integrated with plant3d-web frontend.

**Performance Goals**: Scoped BRAN generation for `2013286704/476` should complete in under 25% of comparable full dbnum generation time in the same environment.

**Constraints**: Preserve normal quick-deploy defaults. Never silently fall back to full generation when scoped validation fails. Do not generalize v1 beyond BRAN roots.

**Scale/Scope**: One target BRAN root per quick-deploy request; first validation target is AvevaPlantSample dbnum `250160`, BRAN `2013286704/476`.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **No cargo tests for web_server**: PASS. Validation plan uses HTTP/POST against a running service, not cargo test.
- **aios-database validation via CLI + JSON**: PASS. Any CLI validation will use command output and generated JSON/Parquet artifacts.
- **Scoped feature isolation**: PASS. The request field is optional and preserves existing full generation behavior by default.
- **Fast-test purpose**: PASS. Scoped filtering is applied before/during generation/export, not merely in the frontend.

No gate violations.

## Project Structure

### Documentation (this feature)

```text
specs/013-bran-scoped-generation/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── quick-deploy-scoped-bran-contract.md
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── web_server/
│   ├── models.rs
│   ├── admin_handlers.rs
│   └── managed_project_sites.rs
├── fast_model/
│   ├── gen_model/
│   ├── export_model/
│   └── query_provider/
└── cli_modes.rs

runtime/
└── scoped-bran-generation-*.json
```

**Structure Decision**: Keep request/response contract changes in `web_server/models.rs`, quick-deploy orchestration in `managed_project_sites.rs`, and reuse existing generation/export/query-provider functions rather than adding a separate standalone generation subsystem.

## Phase 0: Research

Output: [research.md](./research.md)

Resolved decisions:

- Add `target_root_refno` to quick-deploy test/admin request shape.
- Validate target exists, belongs to requested dbnum, and has noun `BRAN`.
- Reuse existing descendant expansion for target BRAN subtree.
- Apply scope before/during generation/export.
- Reuse frontend URL parameters `show_refno`, `mbd_refno`, and `data_source=parquet`.

## Phase 1: Design & Contracts

Output:

- [data-model.md](./data-model.md)
- [contracts/quick-deploy-scoped-bran-contract.md](./contracts/quick-deploy-scoped-bran-contract.md)
- [quickstart.md](./quickstart.md)

## Implementation Approach

1. Extend `QuickDeployTestRequest` with optional `target_root_refno`.
2. Extend `QuickDeployTestResponse` with scoped metadata and viewer URL.
3. Resolve and validate scoped BRAN target after dbnum resolution and before site creation/generation.
4. Persist scoped target metadata into the generated site/task context so generation pipeline can read it.
5. Expand target BRAN subtree via existing query-provider/project metadata functions.
6. Apply scoped refno set to generation/export boundaries.
7. Build viewer URL with `show_refno`, `mbd_refno`, and `data_source=parquet`.
8. Validate through quick-deploy HTTP request and plant3d-web automation.

## Post-Design Constitution Check

- **No cargo tests for web_server**: PASS. Quickstart uses HTTP/POST and artifact inspection.
- **aios-database CLI validation**: PASS. CLI checks, if used, are command-based.
- **Default behavior compatibility**: PASS. No `target_root_refno` means existing full generation.
- **Scoped failure safety**: PASS. Invalid scoped targets fail early and never fall back to full generation.

## Complexity Tracking

No constitution violations or complexity exceptions.
