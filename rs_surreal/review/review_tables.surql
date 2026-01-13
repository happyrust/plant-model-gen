-- ============================================================================
-- 校审流程数据表定义
-- ============================================================================

-- 校审表单主表
DEFINE TABLE review_form SCHEMAFULL;
DEFINE FIELD form_id ON review_form TYPE string;
DEFINE FIELD project_id ON review_form TYPE string;
DEFINE FIELD user_id ON review_form TYPE string;
DEFINE FIELD created_at ON review_form TYPE datetime DEFAULT time::now();
DEFINE INDEX idx_form_id ON review_form FIELDS form_id UNIQUE;

-- 校审模型关联表
DEFINE TABLE review_form_model SCHEMAFULL;
DEFINE FIELD form_id ON review_form_model TYPE string;
DEFINE FIELD model_refno ON review_form_model TYPE string;
DEFINE INDEX idx_form_model ON review_form_model FIELDS form_id;

-- 审批意见表
DEFINE TABLE review_opinion SCHEMAFULL;
DEFINE FIELD form_id ON review_opinion TYPE string;
DEFINE FIELD model_refnos ON review_opinion TYPE array<string>;
DEFINE FIELD node ON review_opinion TYPE string;  -- sj, jd, sh, pz
DEFINE FIELD seq_order ON review_opinion TYPE int;
DEFINE FIELD author ON review_opinion TYPE string;
DEFINE FIELD opinion ON review_opinion TYPE string;
DEFINE FIELD created_at ON review_opinion TYPE datetime DEFAULT time::now();
DEFINE INDEX idx_opinion_form ON review_opinion FIELDS form_id;

-- 附件表
DEFINE TABLE review_attachment SCHEMAFULL;
DEFINE FIELD form_id ON review_attachment TYPE string;
DEFINE FIELD model_refnos ON review_attachment TYPE array<string>;
DEFINE FIELD file_id ON review_attachment TYPE string;
DEFINE FIELD file_type ON review_attachment TYPE string;  -- markup, file
DEFINE FIELD download_url ON review_attachment TYPE string;
DEFINE FIELD description ON review_attachment TYPE string;
DEFINE FIELD file_ext ON review_attachment TYPE string;
DEFINE INDEX idx_attachment_form ON review_attachment FIELDS form_id;

-- ============================================================================
-- 提资单表 (review_tasks) - 多级审批流程扩展
-- ============================================================================

-- 注意: review_tasks 使用 SCHEMALESS 模式，以下为字段说明
-- 基础字段: id, title, description, model_name, status, priority
-- 人员字段: requester_id, requester_name, reviewer_id, reviewer_name
-- 组件字段: components (array), attachments (array)
-- 时间字段: created_at, updated_at, due_date
-- 审核字段: review_comment

-- 多级审批流程新增字段:
-- current_node: string - 当前节点 (sj/jd/sh/pz)
-- workflow_history: array - 流程历史记录
-- return_reason: string - 驳回原因

-- 工作流历史表 (独立存储，便于查询)
DEFINE TABLE review_workflow_history SCHEMAFULL;
DEFINE FIELD task_id ON review_workflow_history TYPE string;
DEFINE FIELD node ON review_workflow_history TYPE string;      -- sj/jd/sh/pz
DEFINE FIELD action ON review_workflow_history TYPE string;    -- submit/return/approve/reject
DEFINE FIELD operator_id ON review_workflow_history TYPE string;
DEFINE FIELD operator_name ON review_workflow_history TYPE string;
DEFINE FIELD comment ON review_workflow_history TYPE option<string>;
DEFINE FIELD timestamp ON review_workflow_history TYPE datetime DEFAULT time::now();
DEFINE INDEX idx_workflow_task ON review_workflow_history FIELDS task_id;
