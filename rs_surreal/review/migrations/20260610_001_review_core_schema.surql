-- ============================================================================
-- Review core schema convergence
-- ============================================================================
--
-- Keep review table/index/field drift out of request handlers. This migration is
-- intentionally idempotent because existing deployments may already have some of
-- these tables or fields from older runtime warmup code.

-- Query hot paths used by review_api.rs / review_annotation_state.rs.
DEFINE TABLE IF NOT EXISTS review_tasks SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_form_id ON TABLE review_tasks FIELDS form_id;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_requester_id ON TABLE review_tasks FIELDS requester_id;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_checker_id ON TABLE review_tasks FIELDS checker_id;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_reviewer_id ON TABLE review_tasks FIELDS reviewer_id;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_approver_id ON TABLE review_tasks FIELDS approver_id;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_current_node ON TABLE review_tasks FIELDS current_node;
DEFINE INDEX IF NOT EXISTS idx_review_tasks_status ON TABLE review_tasks FIELDS status;

DEFINE TABLE IF NOT EXISTS review_records SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_review_records_form_id ON TABLE review_records FIELDS form_id;
DEFINE INDEX IF NOT EXISTS idx_review_records_task_id ON TABLE review_records FIELDS task_id;
DEFINE INDEX IF NOT EXISTS idx_review_records_slot_key ON TABLE review_records FIELDS slot_key;

DEFINE TABLE IF NOT EXISTS review_comments SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_review_comments_annotation_id ON TABLE review_comments FIELDS annotation_id;

DEFINE TABLE IF NOT EXISTS review_attachment SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_review_attachment_form_id ON TABLE review_attachment FIELDS form_id;

DEFINE TABLE IF NOT EXISTS review_form_model SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_review_form_model_form_id ON TABLE review_form_model FIELDS form_id;

-- Independent annotation state truth table.
DEFINE TABLE IF NOT EXISTS review_annotation_states SCHEMALESS;
DEFINE FIELD OVERWRITE form_id ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE task_id ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE annotation_id ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE annotation_type ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE workflow_node ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE review_round ON review_annotation_states TYPE int;
DEFINE FIELD OVERWRITE resolution_status ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE decision_status ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE note ON review_annotation_states TYPE option<string>;
DEFINE FIELD OVERWRITE updated_by_id ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE updated_by_name ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE updated_by_role ON review_annotation_states TYPE string;
DEFINE FIELD OVERWRITE updated_at ON review_annotation_states TYPE datetime;
DEFINE FIELD OVERWRITE created_at ON review_annotation_states TYPE datetime;
DEFINE FIELD OVERWRITE history ON review_annotation_states TYPE option<array>;
DEFINE INDEX IF NOT EXISTS idx_review_annotation_states_form_id ON TABLE review_annotation_states FIELDS form_id;
DEFINE INDEX IF NOT EXISTS idx_review_annotation_states_task_id ON TABLE review_annotation_states FIELDS task_id;
DEFINE INDEX IF NOT EXISTS idx_review_annotation_states_form_task ON TABLE review_annotation_states FIELDS form_id, task_id;

-- RUS-244 workflow history compatibility upgrade.
DEFINE TABLE OVERWRITE review_workflow_history SCHEMAFULL;
DEFINE FIELD OVERWRITE task_id ON review_workflow_history TYPE string;
DEFINE FIELD OVERWRITE node ON review_workflow_history TYPE string;
DEFINE FIELD OVERWRITE action ON review_workflow_history TYPE string;
DEFINE FIELD OVERWRITE operator_id ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE operator_name ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE timestamp ON review_workflow_history TYPE option<datetime> DEFAULT time::now();
DEFINE FIELD OVERWRITE comment ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE form_id ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE target_node ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE actor_id ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE actor_role ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE actor_name ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE source ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE created_at ON review_workflow_history TYPE option<datetime>;
DEFINE INDEX OVERWRITE idx_workflow_task ON TABLE review_workflow_history FIELDS task_id;
DEFINE INDEX OVERWRITE idx_workflow_form ON TABLE review_workflow_history FIELDS form_id;

-- review_history remains schemaless for old data, only converging searchable fields.
DEFINE FIELD OVERWRITE form_id ON review_history TYPE option<string>;
DEFINE FIELD OVERWRITE source ON review_history TYPE option<string>;
DEFINE FIELD OVERWRITE created_at ON review_history TYPE option<datetime>;
