# Research: BRAN Scoped Generation

## Decision: Scope Entry Point

**Decision**: v1 scoped generation is exposed through quick-deploy test/admin quick deploy only, by adding an optional `target_root_refno` request field.

**Rationale**: The user's immediate need is fast BRAN-focused validation, not a broad production deployment mode. Quick deploy already owns test-site creation, parse/generate orchestration, and viewer URL response metadata.

**Alternatives considered**:

- Add a standalone CLI first: rejected for v1 because it would not directly satisfy the frontend auto-test loop.
- Change normal full deployment defaults: rejected because it risks production behavior and violates the "fast-test mode" boundary.

## Decision: Target Validation

**Decision**: v1 accepts only valid BRAN targets. Invalid format, missing refno, dbnum mismatch, or non-BRAN targets fail before generation starts.

**Rationale**: The frontend validation depends on MBD pipe annotation and flow-direction behavior. Allowing arbitrary EQUI/ZONE roots would require separate semantics and would weaken error visibility.

**Alternatives considered**:

- Accept any root refno: rejected because the first workflow is BRAN-specific and MBD pipe data may not exist for other nouns.
- Silently fall back to full generation when invalid: rejected because it hides mistakes and loses the speed benefit.

## Decision: Where To Apply Scope

**Decision**: Reuse parse behavior where practical, but apply the BRAN scope before or during generation/export.

**Rationale**: Parse is already tied to dbnum/library readiness and is not the main frontend iteration cost. The speed win comes from avoiding full model generation and broad output/export.

**Alternatives considered**:

- Full generate then frontend filters with `show_refno`: rejected because backend generation remains slow.
- Rewrite parse to parse only one BRAN immediately: deferred because it is higher risk and can be planned separately after scoped generation proves useful.

## Decision: Scoped Refno Expansion

**Decision**: Reuse existing descendant expansion/query-provider semantics to turn `target_root_refno` into the scoped refno set.

**Rationale**: The repository already has functions such as `query_multi_descendants_with_self()` and frontend `show_refno` depends on visible descendants. Reusing the same domain meaning keeps backend and frontend aligned.

**Alternatives considered**:

- Hand-code BRAN child traversal in quick deploy: rejected because it creates a second traversal semantics.
- Require users to list all child refnos manually: rejected because it is error-prone and defeats fast testing.

## Decision: Frontend Auto-Test Contract

**Decision**: Successful scoped quick deploy returns or logs a viewer URL using existing frontend parameters: `show_refno`, `mbd_refno`, and `data_source=parquet`.

**Rationale**: plant3d-web already supports `show_refno` scoped loading and MBD auto-trigger through `mbd_refno`/`mbd_pipe`. Reusing URL parameters avoids adding a new frontend route for v1.

**Alternatives considered**:

- Add a new frontend route for scoped BRAN tests: rejected for v1 because existing URL parameters are sufficient.
- Return only backend artifact paths: rejected because the user explicitly wants frontend automation.

## Decision: Validation Method

**Decision**: Validate backend through running web_server and HTTP/POST quick-deploy requests; validate frontend with browser automation on the returned URL.

**Rationale**: Repository guidance forbids cargo tests for web_server and prefers real HTTP validation for web_server behavior. Frontend behavior must be tested at the viewer URL level.

**Alternatives considered**:

- Unit-test the web handler with cargo test: rejected by repository rule.
- Validate only by inspecting files: rejected because it does not prove the end-to-end frontend loop.
