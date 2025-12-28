# 进展更新 - 2025-12-29

## 已完成事项
- 删除 `rs-core/src/rs_surreal/inst.rs` 中的冗余查询函数：
    - `query_insts_by_zone`
    - `query_insts_ext`
- 同步重构 `query_insts_with_negative` 以使用 `query_insts_with_batch`。
- 修复 `gen-model-fork` 中受影响的引用：
    - `src/bin/debug_query_insts.rs`
    - `src/fast_model/export_model/model_exporter.rs`
- 通过了 `cargo check` 编译验证。

## 下一步计划
- 如有需要，继续清理 `rs-core` 中其他标记为 TODO 或冗余的查询逻辑。
