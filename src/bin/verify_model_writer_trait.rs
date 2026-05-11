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
//!   6 — Parquet backend 端到端路径失败（v3 Phase B）
//!   7 — Compare wrapper 端到端路径失败（v3 Phase C）

#![cfg(feature = "model-writer-mock")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use aios_database::fast_model::gen_model::canonical_records::CanonicalRawTable;
use aios_database::fast_model::gen_model::mesh_generate::MeshResult;
use aios_database::fast_model::gen_model::model_writer::{
    BaseInstanceBatch, BooleanBridgeRequest, CleanupRequest, CompareModelWriterBackend,
    FinalizeRequest, InstRelateAabbBatch, MeshResultBatch, ModelWriterBackend, ModelWriterContext,
    ParquetModelWriterBackend, ReconcileRequest, RecordingBackend,
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
    if base_report.missing_neg_count != injected_carriers.len() {
        eprintln!(
            "[verify] FAIL: injected missing_neg count not honored: got={} expected={}",
            base_report.missing_neg_count,
            injected_carriers.len()
        );
        return ExitCode::from(4);
    }
    // v3 Phase F.1: drain via the new trait method instead of pulling from WriteBaseReport.
    let drained_carriers = match backend.take_missing_neg_carriers().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[verify] FAIL: take_missing_neg_carriers: {}", e);
            return ExitCode::from(1);
        }
    };
    if drained_carriers.len() != injected_carriers.len() {
        eprintln!(
            "[verify] FAIL: take_missing_neg_carriers drained {} entries, expected {}",
            drained_carriers.len(),
            injected_carriers.len()
        );
        return ExitCode::from(4);
    }
    // 第二次调用必须返回空（drain 幂等）
    let drained_again = backend
        .take_missing_neg_carriers()
        .await
        .expect("second take");
    if !drained_again.is_empty() {
        eprintln!(
            "[verify] FAIL: take_missing_neg_carriers not idempotent: second drain returned {} entries",
            drained_again.len()
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
        "take_missing_neg_carriers", // v3 Phase F.1: 两次 drain（第一次拿到注入值，第二次返回空）
        "take_missing_neg_carriers",
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
        "take_missing_neg_carriers",
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

    // ================================================================
    // v3 Phase B：Parquet backend 端到端路径
    // 走完整 8 个 trait 方法，校验 13 张 canonical raw 表的 JSONL 文件
    // 都在 mission 05 §Layout 描述的位置出现。
    // ================================================================
    let parquet_root = temp_subdir("verify-parquet-backend");
    if let Err(e) = std::fs::create_dir_all(&parquet_root) {
        eprintln!(
            "[verify] FAIL: cannot create temp parquet root {}: {}",
            parquet_root.display(),
            e
        );
        return ExitCode::from(6);
    }
    let parquet_dbnum: u32 = 0;
    let parquet_backend: Arc<dyn ModelWriterBackend> = Arc::new(
        ParquetModelWriterBackend::with_dbnum(parquet_root.clone(), parquet_dbnum),
    );
    let parquet_ctx = ModelWriterContext {
        project_name: "verify-parquet".to_string(),
        use_surrealdb: false,
        defer_db_write: false,
        mode: ModelWriterMode::Parquet,
    };

    if let Err(e) = parquet_backend.init(&parquet_ctx).await {
        eprintln!("[verify] FAIL: parquet init: {}", e);
        return ExitCode::from(6);
    }
    if let Err(e) = parquet_backend
        .cleanup(CleanupRequest { seed_refnos: &[] })
        .await
    {
        eprintln!("[verify] FAIL: parquet cleanup: {}", e);
        return ExitCode::from(6);
    }
    let parquet_base_report = match parquet_backend
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
            eprintln!("[verify] FAIL: parquet write_base_batch: {}", e);
            return ExitCode::from(6);
        }
    };
    if parquet_base_report.batch_id != 1 || parquet_base_report.missing_neg_count != 0 {
        eprintln!(
            "[verify] FAIL: parquet write_base_batch unexpected report batch_id={} missing_neg_count={}",
            parquet_base_report.batch_id, parquet_base_report.missing_neg_count
        );
        return ExitCode::from(6);
    }
    if let Err(e) = parquet_backend
        .persist_mesh_results(MeshResultBatch {
            batch_id: 1,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
            mesh_pts_map: &mesh_pts_map,
            file_mesh_state: false,
        })
        .await
    {
        eprintln!("[verify] FAIL: parquet persist_mesh_results: {}", e);
        return ExitCode::from(6);
    }
    if let Err(e) = parquet_backend
        .write_inst_relate_aabb(InstRelateAabbBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
        })
        .await
    {
        eprintln!("[verify] FAIL: parquet write_inst_relate_aabb: {}", e);
        return ExitCode::from(6);
    }
    let parquet_inserted = match parquet_backend
        .reconcile_missing_neg(ReconcileRequest {
            all_refnos: &[],
            candidate_carriers: &[],
        })
        .await
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[verify] FAIL: parquet reconcile_missing_neg: {}", e);
            return ExitCode::from(6);
        }
    };
    if parquet_inserted != 0 {
        eprintln!(
            "[verify] FAIL: parquet reconcile expected 0 (approximate semantic), got {}",
            parquet_inserted
        );
        return ExitCode::from(6);
    }
    let parquet_bool_report = match parquet_backend
        .run_boolean_bridge(BooleanBridgeRequest {
            mode: BooleanPipelineMode::DbLegacy,
            db_option: Arc::new(aios_core::options::DbOption::default()),
            bool_tasks: Vec::new(),
        })
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[verify] FAIL: parquet run_boolean_bridge: {}", e);
            return ExitCode::from(6);
        }
    };
    if parquet_bool_report.pipeline != "parquet" {
        eprintln!(
            "[verify] FAIL: parquet boolean bridge expected pipeline=`parquet`, got `{}`",
            parquet_bool_report.pipeline
        );
        return ExitCode::from(6);
    }
    if let Err(e) = parquet_backend.finalize(FinalizeRequest::default()).await {
        eprintln!("[verify] FAIL: parquet finalize: {}", e);
        return ExitCode::from(6);
    }

    // 校验 13 张 canonical raw 表的 JSONL 文件路径
    let raw_root = parquet_root.join("model_writer_storage").join("raw");
    for table in CanonicalRawTable::all_phase1() {
        let jsonl = raw_root
            .join(table.as_str())
            .join(format!("project_name={}", parquet_ctx.project_name))
            .join(format!("dbnum={}", parquet_dbnum))
            .join("batch_1.jsonl");
        if !jsonl.exists() {
            eprintln!(
                "[verify] FAIL: parquet expected JSONL not found for table `{}`: {}",
                table.as_str(),
                jsonl.display()
            );
            return ExitCode::from(6);
        }
    }
    // summary 文件也应存在
    let summary_path = parquet_root
        .join("model_writer_storage")
        .join("summary")
        .join(format!("project_name={}", parquet_ctx.project_name))
        .join(format!("dbnum={}", parquet_dbnum))
        .join("batch_1.json");
    if !summary_path.exists() {
        eprintln!(
            "[verify] FAIL: parquet summary JSON not found: {}",
            summary_path.display()
        );
        return ExitCode::from(6);
    }

    // ================================================================
    // v3 Phase C：Compare wrapper 端到端路径
    // Primary = RecordingBackend，Candidate = ParquetModelWriterBackend；
    // 校验：(1) wrapper 所有 8 方法都成功；(2) primary 收到完整调用链；
    //       (3) candidate 13 张 raw 表落盘；(4) wrapper.name() 仍是 "compare"。
    // ================================================================
    let compare_root = temp_subdir("verify-compare-wrapper");
    if let Err(e) = std::fs::create_dir_all(&compare_root) {
        eprintln!(
            "[verify] FAIL: cannot create temp compare root {}: {}",
            compare_root.display(),
            e
        );
        return ExitCode::from(7);
    }
    let compare_primary_concrete = Arc::new(RecordingBackend::default());
    let compare_primary: Arc<dyn ModelWriterBackend> = compare_primary_concrete.clone();
    let compare_candidate: Arc<dyn ModelWriterBackend> = Arc::new(
        ParquetModelWriterBackend::with_dbnum(compare_root.clone(), 0),
    );
    let compare_wrapper: Arc<dyn ModelWriterBackend> = Arc::new(CompareModelWriterBackend::new(
        compare_primary.clone(),
        compare_candidate.clone(),
    ));
    let compare_ctx = ModelWriterContext {
        project_name: "verify-compare".to_string(),
        use_surrealdb: false,
        defer_db_write: false,
        mode: ModelWriterMode::Parquet,
    };
    if compare_wrapper.name() != "compare" {
        eprintln!(
            "[verify] FAIL: compare wrapper name expected `compare`, got `{}`",
            compare_wrapper.name()
        );
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper.init(&compare_ctx).await {
        eprintln!("[verify] FAIL: compare init: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper
        .cleanup(CleanupRequest { seed_refnos: &[] })
        .await
    {
        eprintln!("[verify] FAIL: compare cleanup: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper
        .write_base_batch(BaseInstanceBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_aabb_map: &mesh_aabb_map,
            replace_exist: false,
            write_inst_relate_aabb: false,
        })
        .await
    {
        eprintln!("[verify] FAIL: compare write_base_batch: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper
        .persist_mesh_results(MeshResultBatch {
            batch_id: 1,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
            mesh_pts_map: &mesh_pts_map,
            file_mesh_state: false,
        })
        .await
    {
        eprintln!("[verify] FAIL: compare persist_mesh_results: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper
        .write_inst_relate_aabb(InstRelateAabbBatch {
            batch_id: 1,
            shape_insts: &shape_insts,
            mesh_results: &mesh_results,
            mesh_aabb_map: &mesh_aabb_map,
        })
        .await
    {
        eprintln!("[verify] FAIL: compare write_inst_relate_aabb: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper.take_missing_neg_carriers().await {
        eprintln!("[verify] FAIL: compare take_missing_neg_carriers: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper
        .reconcile_missing_neg(ReconcileRequest {
            all_refnos: &[],
            candidate_carriers: &[],
        })
        .await
    {
        eprintln!("[verify] FAIL: compare reconcile_missing_neg: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper
        .run_boolean_bridge(BooleanBridgeRequest {
            mode: BooleanPipelineMode::DbLegacy,
            db_option: Arc::new(aios_core::options::DbOption::default()),
            bool_tasks: Vec::new(),
        })
        .await
    {
        eprintln!("[verify] FAIL: compare run_boolean_bridge: {}", e);
        return ExitCode::from(7);
    }
    if let Err(e) = compare_wrapper.finalize(FinalizeRequest::default()).await {
        eprintln!("[verify] FAIL: compare finalize: {}", e);
        return ExitCode::from(7);
    }

    // 校验 candidate 也产出了 13 张 canonical raw 表（compare wrapper 必须双写）
    let compare_raw_root = compare_root.join("model_writer_storage").join("raw");
    for table in CanonicalRawTable::all_phase1() {
        let jsonl = compare_raw_root
            .join(table.as_str())
            .join(format!("project_name={}", compare_ctx.project_name))
            .join(format!("dbnum={}", 0u32))
            .join("batch_1.jsonl");
        if !jsonl.exists() {
            eprintln!(
                "[verify] FAIL: compare candidate missing JSONL for `{}`: {}",
                table.as_str(),
                jsonl.display()
            );
            return ExitCode::from(7);
        }
    }

    // 校验 primary（RecordingBackend）也收到全套调用
    let compare_primary_methods: Vec<String> = compare_primary_concrete
        .snapshot()
        .iter()
        .map(|line| line.split(':').next().unwrap_or("").to_string())
        .collect();
    let required_compare_methods = [
        "init",
        "cleanup",
        "write_base_batch",
        "persist_mesh_results",
        "write_inst_relate_aabb",
        "reconcile_missing_neg",
        "run_boolean_bridge",
        "finalize",
        // compare wrapper 也 fan-out take_missing_neg_carriers
        "take_missing_neg_carriers",
    ];
    for method in &required_compare_methods {
        if !compare_primary_methods.iter().any(|m| m == method) {
            eprintln!(
                "[verify] FAIL: compare primary missed routed call `{}` (snapshot={:?})",
                method, compare_primary_methods
            );
            return ExitCode::from(7);
        }
    }

    println!(
        "[verify] PASS — 9 个 trait 调用按预期记录（含二次 init），注入值 reconcile={} missing_neg={} 均被 honored；Parquet backend 13 张 canonical raw 表 + 1 份 summary JSON 全部落盘，输出根 = {}；Compare wrapper 双写到 primary + candidate 共 13 张 raw 表全部落盘，输出根 = {}",
        INJECTED_RECONCILE,
        injected_carriers.len(),
        parquet_root.display(),
        compare_root.display()
    );
    for (idx, line) in snapshot.iter().enumerate() {
        println!("  [{}] {}", idx + 1, line);
    }
    ExitCode::SUCCESS
}

fn temp_subdir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("aios-{label}-{nanos}"))
}
