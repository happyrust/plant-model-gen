//! ZoneStream：按 ZONE 双缓冲的初始化流水（ADR-0016 / spec 030）。
//!
//! 这是 Legacy 之外的第二条初始化编排路径。两条路径的分流**只发生在 managed-site
//! 编排入口**：旧解析（`spawn_parse_process`）与旧生成（`spawn_generation_process`）
//! 入口内部不含任何模式判断（ADR-0016 D1），Legacy 行为逐位不变。

pub mod orchestrator;
pub mod run_store;

pub use orchestrator::{
    is_zone_stream, reject_legacy_entry_for_zone_stream, request_stop, resume_initialization,
    run_initialization, ZoneStreamStage,
};
pub use run_store::{InitializationRun, RunIdentity, SlotState};
