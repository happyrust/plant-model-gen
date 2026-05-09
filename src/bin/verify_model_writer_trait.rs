//! ModelWriterBackend trait 契约验证 binary。
//!
//! 用 `RecordingBackend`（feature `model-writer-mock`）跑一次完整调用链，
//! 断言 8 个 trait 方法按预期顺序被调用，每次调用入参符合预期。
//!
//! 用法：
//! ```powershell
//! cargo run --bin verify_model_writer_trait --features model-writer-mock
//! ```
//!
//! 退出码：
//!   0 — 通过
//!   1 — trait 方法返回 Err
//!   2 — snapshot 调用计数不符
//!   3 — snapshot 顺序不符

#![cfg(feature = "model-writer-mock")]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;

use aios_core::geometry::ShapeInstancesData;
use aios_database::fast_model::gen_model::mesh_generate::MeshResult;
use aios_database::fast_model::gen_model::model_writer::{
    BaseInstanceBatch, BooleanBridgeRequest, CleanupRequest, FinalizeRequest, InstRelateAabbBatch,
    MeshResultBatch, ModelWriterBackend, ModelWriterContext, ReconcileRequest, RecordingBackend,
};
use aios_database::options::{BooleanPipelineMode, ModelWriterMode};
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let backend = Arc::new(RecordingBackend::default());

    let context = ModelWriterContext {
        project_name: "verify-fixture".to_string(),
        use_surrealdb: true,
        defer_db_write: false,
        mode: ModelWriterMode::Surreal,
    };

    let shape_insts = ShapeInstancesData::default();
    let mesh_aabb_map: DashMap<String, Aabb> = DashMap::new();
    let mesh_pts_map: DashMap<u64, String> = DashMap::new();
    let mesh_results: HashMap<u64, MeshResult> = HashMap::new();

    if let Err(e) = backend.init(&context).await {
        eprintln!("[verify] FAIL: init: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend.cleanup(CleanupRequest { seed_refnos: &[] }).await {
        eprintln!("[verify] FAIL: cleanup: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend
        .write_base_batch(BaseInstanceBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_aabb_map: &mesh_aabb_map,
            replace_exist: false,
            write_inst_relate_aabb: false,
        })
        .await
    {
        eprintln!("[verify] FAIL: write_base_batch: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend
        .persist_mesh_results(MeshResultBatch {
            batch_id: 1,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
            mesh_pts_map: &mesh_pts_map,
            file_mesh_state: false,
        })
        .await
    {
        eprintln!("[verify] FAIL: persist_mesh_results: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend
        .write_inst_relate_aabb(InstRelateAabbBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
        })
        .await
    {
        eprintln!("[verify] FAIL: write_inst_relate_aabb: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend
        .reconcile_missing_neg(ReconcileRequest {
            all_refnos: &[],
            candidate_carriers: &[],
        })
        .await
    {
        eprintln!("[verify] FAIL: reconcile_missing_neg: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend
        .run_boolean_bridge(BooleanBridgeRequest {
            mode: BooleanPipelineMode::DbLegacy,
            db_option: Arc::new(aios_core::options::DbOption::default()),
            bool_tasks: Vec::new(),
        })
        .await
    {
        eprintln!("[verify] FAIL: run_boolean_bridge: {}", e);
        return ExitCode::from(1);
    }

    if let Err(e) = backend.finalize(FinalizeRequest::default()).await {
        eprintln!("[verify] FAIL: finalize: {}", e);
        return ExitCode::from(1);
    }

    let snapshot = backend.snapshot();
    let expected_prefixes = [
        "init:",
        "cleanup:",
        "write_base_batch:",
        "persist_mesh_results:",
        "write_inst_relate_aabb:",
        "reconcile_missing_neg:",
        "run_boolean_bridge:",
        "finalize:",
    ];

    if snapshot.len() != expected_prefixes.len() {
        eprintln!(
            "[verify] FAIL: 调用计数不符 expected={} got={}",
            expected_prefixes.len(),
            snapshot.len()
        );
        for line in &snapshot {
            eprintln!("  - {}", line);
        }
        return ExitCode::from(2);
    }

    for (idx, prefix) in expected_prefixes.iter().enumerate() {
        if !snapshot[idx].starts_with(prefix) {
            eprintln!(
                "[verify] FAIL: 第 {} 步预期前缀 `{}`, 实得 `{}`",
                idx, prefix, snapshot[idx]
            );
            return ExitCode::from(3);
        }
    }

    println!("[verify] PASS — 8 个 trait 方法按预期顺序被调用");
    for (idx, line) in snapshot.iter().enumerate() {
        println!("  [{}] {}", idx + 1, line);
    }
    ExitCode::SUCCESS
}
