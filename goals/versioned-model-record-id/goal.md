# Versioned Model Record ID Goal

## Articulated Goal

将模型生成产物表迁移到 SurrealDB 3.1 range-friendly array record id，并把 `sesno` 作为模型数据版本维度纳入 `pe -> 模型数据` 架构。目标不是单纯删除优化，而是让同一 `pe` 的 current 和历史版本模型数据可并存、可查询、可按 range cleanup。

## Shared Understanding

事实清单见 `facts.md`。其中最重要的约束是：`pe` 主表 ID 不变，`inst_relate:[ref0, ref1, sesno]` 承载模型版本关系，`tubi_relate` 按 BRAN 的 `sesno` 版本化，不做旧数据迁移、不 dual write、不 dual cleanup，且禁止使用 `cargo test` 或编译 test。

## Execution Plan

开发计划见 `plan.md`。实施应在独立 worktree `D:\work\plant-code\plant-model-gen-range-id` 中进行，先锁定版本查询契约和 `neg_relate/ngmr_relate` 的 ID 前缀所有权，再迁移 helper、写入路径、读路径、tubi 版本关系和 cleanup。

## Done Condition

- 新生成的模型产物数据只使用 array record id。
- `inst_relate` 能按 `[ref0, ref1, sesno]` 支持 current 和历史版本查询。
- `pe` current 图关系仍可遍历到实例和几何。
- `tubi_relate` 按 BRAN `sesno` 支持版本查询和 range cleanup。
- 多 geometry refno 不发生 ID 冲突。
- cleanup 主路径使用 record id range，并能按 ref0/refno/sesno 删除目标数据。
- CLI、JSON 和 Surreal 查询验证通过；未运行任何 Rust test。

Done! Launch a goal with `/goal goals/versioned-model-record-id/goal.md`
