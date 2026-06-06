-- 三维校审：表单(form_id)维度的模型/意见/附件汇总表
-- 用于 /api/review/workflow/sync 以及相关数据同步

-- 1) form_id -> 模型清单
DEFINE TABLE review_form_model SCHEMALESS;

DEFINE FIELD form_id ON TABLE review_form_model TYPE string;
DEFINE FIELD model_refno ON TABLE review_form_model TYPE string;
DEFINE FIELD created_at ON TABLE review_form_model TYPE datetime VALUE time::now();

DEFINE INDEX review_form_model_form_idx ON TABLE review_form_model FIELDS form_id;
DEFINE INDEX review_form_model_unique_idx ON TABLE review_form_model FIELDS form_id, model_refno UNIQUE;

-- 2) form_id -> 审批意见
DEFINE TABLE review_opinion SCHEMALESS;

DEFINE FIELD form_id ON TABLE review_opinion TYPE string;
DEFINE FIELD model_refnos ON TABLE review_opinion TYPE array<string> DEFAULT [];
DEFINE FIELD node ON TABLE review_opinion TYPE string;
DEFINE FIELD seq_order ON TABLE review_opinion TYPE int;
DEFINE FIELD author ON TABLE review_opinion TYPE string;
DEFINE FIELD opinion ON TABLE review_opinion TYPE string;
DEFINE FIELD created_at ON TABLE review_opinion TYPE datetime VALUE time::now();

DEFINE INDEX review_opinion_form_idx ON TABLE review_opinion FIELDS form_id;
DEFINE INDEX review_opinion_node_idx ON TABLE review_opinion FIELDS form_id, node;

-- 3) form_id -> 附件（云线/文件）
DEFINE TABLE review_attachment SCHEMALESS;

DEFINE FIELD form_id ON TABLE review_attachment TYPE string;
DEFINE FIELD model_refnos ON TABLE review_attachment TYPE array<string> DEFAULT [];
DEFINE FIELD file_id ON TABLE review_attachment TYPE string;
DEFINE FIELD file_type ON TABLE review_attachment TYPE string;
DEFINE FIELD download_url ON TABLE review_attachment TYPE string;
DEFINE FIELD description ON TABLE review_attachment TYPE string;
DEFINE FIELD file_ext ON TABLE review_attachment TYPE string;
DEFINE FIELD created_at ON TABLE review_attachment TYPE datetime VALUE time::now();

DEFINE INDEX review_attachment_form_idx ON TABLE review_attachment FIELDS form_id;
DEFINE INDEX review_attachment_file_idx ON TABLE review_attachment FIELDS form_id, file_id;
