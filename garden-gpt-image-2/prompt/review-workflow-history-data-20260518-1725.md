Create a clean Chinese technical infographic explaining what the database table `review_workflow_history` currently stores in the Plant3D review workflow system.

Style:
- 16:9 landscape, crisp enterprise architecture infographic.
- White background, subtle blue and slate accents.
- Use clear Chinese labels and short field descriptions.
- No tiny unreadable text; use grouped blocks and arrows.

Main title:
`review_workflow_history 当前存储的数据`

Layout:
1. Left column: "它不是当前状态表，而是流程动作日志"
   - Current state source: `review_tasks.current_node / status / return_reason`
   - History table: append-only workflow event records

2. Center: a large table-card named `review_workflow_history`
   Group fields into four sections:
   - Identity:
     `task_id` = 内部任务 ID
     `form_id` = PMS 单据 ID / 外部业务主键
   - Flow:
     `node` = 动作发生节点 sj/jd/sh/pz
     `target_node` = 流转目标节点
     `action` = submit / return / approve / stop / resubmit
   - Actor:
     `actor_id` / `actor_name` / `actor_role` = 新字段
     `operator_id` / `operator_name` = 旧兼容字段
   - Audit:
     `comment` = 审批意见 / 驳回原因
     `source` = plant3d-internal 或 workflow sync 来源
     `timestamp` / `created_at` = 发生时间

3. Right column: "写入来源"
   - Internal API: `submit_to_next_node`, `return_to_node`, `resubmit`
   - External PMS sync: `active`, `agree`, `return`, `stop`
   Show arrows from both sources into the central table.

4. Bottom warning strip:
   "当前问题：表已包含 form_id 索引，但读取和部分业务依赖仍偏 task_id；建议改为 form_id 聚合历史。"

Visual notes:
- Use small icons: database cylinder, arrows, clock, user.
- Keep all text in Simplified Chinese.
- Avoid decorative clutter.
