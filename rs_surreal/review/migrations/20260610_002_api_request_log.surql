-- ============================================================================
-- API request/response log (spec 003-review-log-viewer T101)
-- ============================================================================
--
-- Captured by the review/platform axum middleware layer. SCHEMALESS keeps the
-- write path tolerant; only the queryable fields get indexes.

-- created_at_ms uses epoch milliseconds (number) instead of datetime so the write
-- path and retention cleanup stay schema-version agnostic across SurrealDB upgrades.
DEFINE TABLE IF NOT EXISTS api_request_log SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_api_request_log_form_id ON TABLE api_request_log FIELDS form_id;
DEFINE INDEX IF NOT EXISTS idx_api_request_log_task_id ON TABLE api_request_log FIELDS task_id;
DEFINE INDEX IF NOT EXISTS idx_api_request_log_created_at_ms ON TABLE api_request_log FIELDS created_at_ms;
DEFINE INDEX IF NOT EXISTS idx_api_request_log_status ON TABLE api_request_log FIELDS status;
