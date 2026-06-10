# Feature Specification: SurrealDB Review Context Resilience

## User Need

Review evidence save operations must not fail because each request repeats a SurrealDB namespace/database context switch. The backend should keep the review database context stable, avoid unnecessary WebSocket reconnects, and keep the main SurrealDB connection alive during idle periods.

## Scope

- Review API and platform-review API database context checks.
- Review database session acquisition used by records, comments, annotation states, workflow sync, and review forms.
- Main `SUL_DB` idle keepalive during web server runtime.

## Requirements

1. Review route middleware initializes the review database connection if needed, but does not run `USE NS/DB` on every request.
2. Existing review handlers keep using the same public helper names, but the helper returns a cloned session from the initialized review connection instead of opening a new physical WebSocket connection per call.
3. Physical review connection creation remains available internally for first initialization.
4. Existing review query timeout wrappers remain in place.
5. `review.ensure_context` must no longer appear as a per-request timeout operation.
6. Web server startup enables the existing `aios_core` heartbeat for `SUL_DB` once per process when WebSocket SurrealDB mode is active.

## Non-Goals

- Do not migrate every repository-wide `.query(...)` call in this change.
- Do not change SurrealDB schema, table definitions, or review API wire shapes.
- Do not add or run Rust test targets.
- Do not modify admin UI build artifacts.

## Acceptance Criteria

- Saving review evidence no longer returns `校审数据库上下文切换失败: review.ensure_context 超时`.
- Review query failures still include operation-specific timeout names from `await_review_query`.
- Review schema warmup can still run through the shared review DB helper.
- Main web server logs one heartbeat enablement message in WebSocket SurrealDB mode.
