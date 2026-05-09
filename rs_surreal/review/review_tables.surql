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
-- ============================================================================
-- RUS-244 schema 升级（2026-05-09）：
--   - 旧字段 operator_id / operator_name / timestamp 改为 option<...>
--     避免新写入路径不传时 SCHEMAFULL 拒绝（生产 regression hotfix）
--   - 新增 form_id / target_node / actor_id / actor_role / actor_name /
--     source / created_at，承载完整的工作流元数据
--     source 取值：plant3d-internal / pms-sync 区分调用来源
-- 重新部署需用 `DEFINE FIELD OVERWRITE` 显式覆盖已存在字段（SurrealDB 2.x 行为）
-- ============================================================================
DEFINE TABLE OVERWRITE review_workflow_history SCHEMAFULL;
DEFINE FIELD OVERWRITE task_id ON review_workflow_history TYPE string;
DEFINE FIELD OVERWRITE node ON review_workflow_history TYPE string;      -- sj/jd/sh/pz
DEFINE FIELD OVERWRITE action ON review_workflow_history TYPE string;    -- submit/return/approve/reject/active/agree/stop/resubmit
DEFINE FIELD OVERWRITE operator_id ON review_workflow_history TYPE option<string>;     -- legacy 兼容字段
DEFINE FIELD OVERWRITE operator_name ON review_workflow_history TYPE option<string>;   -- legacy 兼容字段
DEFINE FIELD OVERWRITE timestamp ON review_workflow_history TYPE option<datetime> DEFAULT time::now();  -- legacy 兼容字段
DEFINE FIELD OVERWRITE comment ON review_workflow_history TYPE option<string>;
-- RUS-244 新字段：完整工作流上下文
DEFINE FIELD OVERWRITE form_id ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE target_node ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE actor_id ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE actor_role ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE actor_name ON review_workflow_history TYPE option<string>;
DEFINE FIELD OVERWRITE source ON review_workflow_history TYPE option<string>;            -- plant3d-internal / pms-sync
DEFINE FIELD OVERWRITE created_at ON review_workflow_history TYPE option<datetime>;
DEFINE INDEX OVERWRITE idx_workflow_task ON review_workflow_history FIELDS task_id;
DEFINE INDEX OVERWRITE idx_workflow_form ON review_workflow_history FIELDS form_id;

-- ============================================================================
-- review_history 表 (update_task_status 写)
-- RUS-244 同步加 form_id / source / created_at 字段
-- ============================================================================
-- 该表历史上为 SCHEMALESS（无 DEFINE FIELD）；这里只补必要的字段定义以便索引/查询，
-- 不强制 SCHEMAFULL 避免破坏老数据。
DEFINE FIELD OVERWRITE form_id ON review_history TYPE option<string>;
DEFINE FIELD OVERWRITE source ON review_history TYPE option<string>;
DEFINE FIELD OVERWRITE created_at ON review_history TYPE option<datetime>;
