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
