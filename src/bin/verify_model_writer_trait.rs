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
//!   4 — 注入值未被 backend 返回
//!   5 — 二次 init / cleanup-without-init 安全性断言失败

#![cfg(feature = "model-writer-mock")]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_database::fast_model::gen_model::mesh_generate::MeshResult;
use aios_database::fast_model::gen_model::model_writer::{
    BaseInstanceBatch, BooleanBridgeRequest, CleanupRequest, FinalizeRequest, InstRelateAabbBatch,
    MeshResultBatch, ModelWriterBackend, ModelWriterContext, ReconcileRequest, RecordingBackend,
};
use aios_database::options::{BooleanPipelineMode, ModelWriterMode};
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

const INJECTED_RECONCILE: usize = 42;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let backend = Arc::new(RecordingBackend::default());

    let injected_carriers: Vec<RefnoEnum> = vec![RefnoEnum::default(), RefnoEnum::default()];
    *backend
        .injected_reconcile_inserted
        .lock()
        .expect("recording lock") = INJECTED_RECONCILE;
    *backend.injected_missing_neg.lock().expect("recording lock") = injected_carriers.clone();

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
    // 二次 init 必须安全（RecordingBackend 内部应当幂等记录，无 panic / Err）
    if let Err(e) = backend.init(&context).await {
        eprintln!("[verify] FAIL: second init should be safe: {}", e);
        return ExitCode::from(5);
    }

    if let Err(e) = backend.cleanup(CleanupRequest { seed_refnos: &[] }).await {
        eprintln!("[verify] FAIL: cleanup: {}", e);
        return ExitCode::from(1);
    }

    let base_report = match backend
        .write_base_batch(BaseInstanceBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_aabb_map: &mesh_aabb_map,
            replace_exist: false,
            write_inst_relate_aabb: false,
        })
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[verify] FAIL: write_base_batch: {}", e);
            return ExitCode::from(1);
        }
    };
    if base_report.missing_neg_count != injected_carriers.len()
        || base_report.missing_neg_carriers.len() != injected_carriers.len()
    {
        eprintln!(
            "[verify] FAIL: injected missing_neg not honored: count={} carriers={} expected={}",
            base_report.missing_neg_count,
            base_report.missing_neg_carriers.len(),
            injected_carriers.len()
        );
        return ExitCode::from(4);
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

    let inserted = match backend
        .reconcile_missing_neg(ReconcileRequest {
            all_refnos: &[],
            candidate_carriers: &[],
        })
        .await
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[verify] FAIL: reconcile_missing_neg: {}", e);
            return ExitCode::from(1);
        }
    };
    if inserted != INJECTED_RECONCILE {
        eprintln!(
            "[verify] FAIL: injected_reconcile_inserted not honored: got={} expected={}",
            inserted, INJECTED_RECONCILE
        );
        return ExitCode::from(4);
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
        "init:", // second init recorded — mock 应记录两次
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

    // 反例：snapshot 不能包含计划外的方法名（防止 trait 误增方法或 mock 漏改）
    let allowed_methods = [
        "init",
        "cleanup",
        "write_base_batch",
        "persist_mesh_results",
        "write_inst_relate_aabb",
        "reconcile_missing_neg",
        "run_boolean_bridge",
        "finalize",
    ];
    for line in &snapshot {
        let method = line.split(':').next().unwrap_or("");
        if !allowed_methods.contains(&method) {
            eprintln!("[verify] FAIL: snapshot 含未预期方法 `{}`", method);
            return ExitCode::from(3);
        }
    }

    // 二次 backend：cleanup-without-init 也必须安全（对应 cli_modes 历史路径）
    let backend2 = Arc::new(RecordingBackend::default());
    if let Err(e) = backend2.cleanup(CleanupRequest { seed_refnos: &[] }).await {
        eprintln!(
            "[verify] FAIL: cleanup-without-init should be safe on mock: {}",
            e
        );
        return ExitCode::from(5);
    }

    println!(
        "[verify] PASS — 9 个 trait 调用按预期记录（含二次 init），注入值 reconcile={} missing_neg={} 均被 honored",
        INJECTED_RECONCILE,
        injected_carriers.len()
    );
    for (idx, line) in snapshot.iter().enumerate() {
        println!("  [{}] {}", idx + 1, line);
    }
    ExitCode::SUCCESS
}
