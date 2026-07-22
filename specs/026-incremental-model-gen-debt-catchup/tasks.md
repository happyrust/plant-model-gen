# Tasks

- [ ] T001 `model_gen_debt` 表 schema 与提交流程幂等写入 + run summary 欠账字段
- [ ] T002 模型生成水位读取与欠账区间覆盖性检查 helper（`versioned_db/model_gen_debt.rs`）
- [ ] T003 提炼追赶核心函数（欠账合并 → Incremental 生成 → 锚点 → 消费标记），`run_increment` 与 watch 共用
- [ ] T004 watch 循环接入 per-dbnum 水位比对与追赶，失败隔离
- [ ] T005 `generate_model` 默认翻转 + `--no-generate-model`（watch / incremental-sesno / web 入口对齐）
- [ ] T006 `model-version catch-up` 子命令（`--dbnum` / `--dry-run` / `--json`）；覆盖洞的整库兜底接 specs/027 受控 repair/catch-up 门禁（绑定 data anchor + 项目锁 + run ledger），禁止裸 `--allow-full-regen`
- [ ] T007 属性级过滤接入采集分类（生成链路未知属性默认触发）+ `model_neutral_changes` + `--no-model-impact-filter`
- [ ] T008 delete-only / 空操作的锚点语义统一与验收（含 catch-up 场景）
- [ ] T009 smoke 脚本：失败自愈 / 存量首次追平 / 元数据零生成 / delete-only 水位推进 / dry-run 字段断言
- [ ] T010 文档对齐：CHANGELOG（默认翻转）、ops-notes（洞告警与 catch-up 运维口径）、AGENTS.md 增量段落
