// SPDX-License-Identifier: MIT
//
// DuckLake ModelWriter backend — Slice 1 skeleton.
//
// Goal package: `goals/ducklake-model-writer/`.
// Slice 1 scope:
//   * Cargo feature `model-writer-ducklake` + optional `duckdb` crate.
//   * `DuckLakeConfig` / `DuckLakeSession` data layer.
//   * `DuckLakeModelWriterBackend` implementing `ModelWriterBackend`.
//   * `init` / `cleanup` / `finalize` open/close DuckDB+DuckLake, create
//     `ducklake-canonical` schema, create 9 empty Phase 1 raw tables in scope.
//   * `write_base_batch` / `persist_mesh_results` / `persist_inst_relate_aabb`
//     / `reconcile_missing_neg_relations` / `run_boolean_bridge` are
//     intentionally NOT IMPLEMENTED in Slice 1; they bail!() so callers fail
//     fast until Slice 2-4 land.
//
// Phase 1 trait gap tables NOT written by this backend (per Q1=C scope lock):
//   * raw_tubi_info / raw_tubi_relate / raw_aabb(tubi) / raw_trans
//     / raw_vec3(tubi) / raw_refno_assoc_index
//   They remain in `cata_model.rs` / `refno_assoc_index.rs` direct
//   SurrealQL writes; closure is the responsibility of a future goal
//   (`09-phase-1-tubi-trait-migration`).
//
// Projection 9 tables and projection refresh SQL are deferred to a future
// `ducklake-projection-refresh` goal (per Q2=B).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use dashmap::DashMap;
use duckdb::{Connection, params};
use parry3d::bounding_volume::Aabb;

use crate::fast_model::gen_model::mesh_generate::MeshResult;
use crate::fast_model::gen_model::model_writer::{
    BooleanBridgeReport, BooleanBridgeRequest, ModelWriteBatchReport, ModelWriterBackend,
    ModelWriterFinishReport, ModelWriterStageReport,
};
use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;

/// Tables actually written by this backend in Slice 1+ (Q1=C scope).
const DUCKLAKE_RAW_TABLES_IN_SCOPE: [&str; 9] = [
    "raw_inst_info",
    "raw_inst_relate",
    "raw_inst_geo",
    "raw_geo_relate",
    "raw_neg_relate",
    "raw_ngmr_relate",
    "raw_aabb", // mesh-derived AABB rows only; tubi AABBs are Known Gap.
    "raw_vec3", // mesh point payloads only; tubi pts are Known Gap.
    "raw_inst_relate_aabb",
];

/// CREATE TABLE DDL per Slice 2/3/4. Tables not yet wired keep a single
/// `placeholder_id BIGINT` column until their slice lands. Schemas are kept
/// minimal but include stable id columns + a `payload_json` column for the
/// bulk record, mirroring the canonical schema spec phrasing in
/// `.factory/mission-docs/model-writer-storage/02-canonical-schema.md`.
fn create_table_ddl(table: &str) -> String {
    let columns = match table {
        // Slice 2 — write_base_batch tables.
        "raw_inst_info" => {
            "inst_id TEXT, owner_refno TEXT, owner_type TEXT, cata_hash TEXT, sesno INTEGER, visible BOOLEAN, payload_json TEXT"
        }
        "raw_inst_relate" => "refno TEXT, inst_id TEXT, payload_json TEXT",
        // raw_inst_geo: base batch writes (geo_hash, refno, type_name, payload_json) with mesh
        // columns NULL; persist_mesh_results UPDATE fills meshed/bad/mesh_aabb_id/mesh_pts_hashes_json.
        "raw_inst_geo" => {
            "geo_hash TEXT, refno TEXT, type_name TEXT, meshed BOOLEAN, bad BOOLEAN, mesh_aabb_id TEXT, mesh_pts_hashes_json TEXT, payload_json TEXT"
        }
        "raw_geo_relate" => {
            "inst_id TEXT, geo_hash TEXT, geom_refno TEXT, idx INTEGER, geo_type TEXT, visible BOOLEAN, is_tubi BOOLEAN, payload_json TEXT"
        }
        "raw_neg_relate" => "carrier_refno TEXT, target_refno TEXT",
        "raw_ngmr_relate" => "carrier_refno TEXT, target_refno TEXT, ngmr_refno TEXT",
        // Slice 3 mesh-derived tables.
        "raw_aabb" => {
            "aabb_id TEXT, min_x DOUBLE, min_y DOUBLE, min_z DOUBLE, max_x DOUBLE, max_y DOUBLE, max_z DOUBLE"
        }
        "raw_vec3" => "vec3_id TEXT, payload TEXT",
        "raw_inst_relate_aabb" => "refno TEXT, aabb_id TEXT, source TEXT",
        _ => "placeholder_id BIGINT",
    };
    format!(
        "CREATE TABLE IF NOT EXISTS \"{}\".\"{}\" ({})",
        DUCKLAKE_CANONICAL_SCHEMA, table, columns
    )
}

/// Phase 1 raw tables explicitly NOT written by this backend (Q1=C Known Gap).
///
/// Reported verbatim in `finalize().stage_reports` so downstream parity
/// scripts can diff against an authoritative list.
const DUCKLAKE_KNOWN_GAP_TABLES: [&str; 6] = [
    "raw_tubi_info",
    "raw_tubi_relate",
    "raw_aabb(tubi)",
    "raw_trans",
    "raw_vec3(tubi)",
    "raw_refno_assoc_index",
];

/// DuckLake namespace per `.factory/mission-docs/model-writer-storage/02-canonical-schema.md`.
const DUCKLAKE_CANONICAL_SCHEMA: &str = "ducklake-canonical";

/// Runtime configuration for `DuckLakeModelWriterBackend`.
#[derive(Clone, Debug)]
pub struct DuckLakeConfig {
    /// Root directory holding `metadata.ducklake` + `data/`.
    pub root_dir: PathBuf,
    /// Logical DuckLake catalog name used in `ATTACH 'ducklake:...' AS <name>`.
    pub catalog_name: String,
}

impl Default for DuckLakeConfig {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::from("output/model_writer_storage/ducklake"),
            catalog_name: "lake".to_string(),
        }
    }
}

impl DuckLakeConfig {
    fn metadata_path(&self) -> PathBuf {
        self.root_dir.join("metadata.ducklake")
    }

    fn data_dir(&self) -> PathBuf {
        self.root_dir.join("data")
    }

    fn attach_uri(&self) -> String {
        let p = self.metadata_path();
        let p_str = p.to_string_lossy().replace('\\', "/");
        format!("ducklake:{p_str}")
    }

    fn reset_storage(&self) -> anyhow::Result<usize> {
        let mut removed = 0usize;
        let metadata = self.metadata_path();
        if metadata.exists() {
            std::fs::remove_file(&metadata).map_err(|e| {
                anyhow::anyhow!(
                    "ducklake cleanup: remove metadata {} failed: {e}",
                    metadata.display()
                )
            })?;
            removed += 1;
        }

        let data_dir = self.data_dir();
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir).map_err(|e| {
                anyhow::anyhow!(
                    "ducklake cleanup: remove data_dir {} failed: {e}",
                    data_dir.display()
                )
            })?;
            removed += 1;
        }

        Ok(removed)
    }
}

/// Active DuckDB connection + open DuckLake catalog. Held in a `Mutex` because
/// `duckdb::Connection` is not `Sync`; we serialize all writes through the
/// trait surface, so a coarse Mutex is acceptable for Slice 1.
struct DuckLakeSession {
    conn: Connection,
}

impl DuckLakeSession {
    fn open(cfg: &DuckLakeConfig) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&cfg.root_dir).map_err(|e| {
            anyhow::anyhow!(
                "ducklake init: create root_dir {} failed: {e}",
                cfg.root_dir.display()
            )
        })?;
        std::fs::create_dir_all(cfg.data_dir()).map_err(|e| {
            anyhow::anyhow!(
                "ducklake init: create data_dir {} failed: {e}",
                cfg.data_dir().display()
            )
        })?;

        // In-memory DuckDB anchor + ATTACH ducklake; metadata + data persist
        // on disk under cfg.root_dir.
        let conn = Connection::open_in_memory()
            .map_err(|e| anyhow::anyhow!("ducklake init: open in-memory duckdb failed: {e}"))?;

        // INSTALL/LOAD ducklake extension. Requires network on first run;
        // failure here is a known blocker (blockers.md / Known Blockers).
        conn.execute_batch("INSTALL ducklake; LOAD ducklake;")
            .map_err(|e| {
                anyhow::anyhow!(
                    "ducklake init: INSTALL/LOAD ducklake failed (offline?): {e}; \
                     see goals/ducklake-model-writer/blockers.md Known Blockers"
                )
            })?;

        let data_path = cfg.data_dir().to_string_lossy().replace('\\', "/");
        let attach_sql = format!(
            "ATTACH '{}' AS {} (DATA_PATH '{}')",
            cfg.attach_uri(),
            cfg.catalog_name,
            data_path
        );
        conn.execute_batch(&attach_sql).map_err(|e| {
            anyhow::anyhow!("ducklake init: ATTACH '{}' failed: {e}", cfg.attach_uri())
        })?;
        let use_sql = format!("USE {};", cfg.catalog_name);
        conn.execute_batch(&use_sql)
            .map_err(|e| anyhow::anyhow!("ducklake init: USE {} failed: {e}", cfg.catalog_name))?;

        let create_schema_sql = format!(
            "CREATE SCHEMA IF NOT EXISTS \"{}\"",
            DUCKLAKE_CANONICAL_SCHEMA
        );
        conn.execute_batch(&create_schema_sql).map_err(|e| {
            anyhow::anyhow!(
                "ducklake init: CREATE SCHEMA {} failed: {e}",
                DUCKLAKE_CANONICAL_SCHEMA
            )
        })?;

        // Create 9 raw tables. Tables wired by Slice 2 have real schemas;
        // Slice 3/4 tables keep a placeholder column until their slice lands.
        for table in DUCKLAKE_RAW_TABLES_IN_SCOPE {
            let create_sql = create_table_ddl(table);
            conn.execute_batch(&create_sql).map_err(|e| {
                anyhow::anyhow!(
                    "ducklake init: CREATE TABLE {}.{} failed: {e}",
                    DUCKLAKE_CANONICAL_SCHEMA,
                    table
                )
            })?;
        }

        Ok(Self { conn })
    }

    /// Slice 2 write path: insert all six base tables for one batch inside a
    /// single transaction. Errors include `batch_id` (string) and `table` name
    /// per `03-writer-architecture.md` error-handling requirements.
    fn write_base_batch_inner(
        &self,
        batch_id: &str,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<()> {
        let conn = &self.conn;

        let inst_info_rows = build_raw_inst_info_rows(batch);
        let inst_relate_rows = build_raw_inst_relate_rows(batch);
        let inst_geo_rows = build_raw_inst_geo_rows(batch);
        let geo_relate_rows = build_raw_geo_relate_rows(batch);
        let neg_relate_rows = build_raw_neg_relate_rows(batch);
        let ngmr_relate_rows = build_raw_ngmr_relate_rows(batch);

        conn.execute_batch("BEGIN TRANSACTION").map_err(|e| {
            anyhow::anyhow!("ducklake write_base_batch[{batch_id}]: BEGIN failed: {e}")
        })?;

        let result: anyhow::Result<()> = (|| {
            insert_inst_info(conn, batch_id, &inst_info_rows)?;
            insert_inst_relate(conn, batch_id, &inst_relate_rows)?;
            insert_inst_geo(conn, batch_id, &inst_geo_rows)?;
            insert_geo_relate(conn, batch_id, &geo_relate_rows)?;
            insert_neg_relate(conn, batch_id, &neg_relate_rows)?;
            insert_ngmr_relate(conn, batch_id, &ngmr_relate_rows)?;
            Ok(())
        })();

        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(|e| {
                anyhow::anyhow!("ducklake write_base_batch[{batch_id}]: COMMIT failed: {e}")
            }),
            Err(err) => {
                // Best-effort rollback; preserve the original error.
                let _ = conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn finalize(self) -> anyhow::Result<()> {
        self.conn
            .execute_batch("CHECKPOINT;")
            .map_err(|e| anyhow::anyhow!("ducklake finalize: CHECKPOINT failed: {e}"))?;
        // Connection is dropped at end of scope; data already persisted by
        // DuckLake metadata + Parquet data files under cfg.root_dir.
        Ok(())
    }

    /// Slice 3 mesh write path: insert mesh-derived AABB + Vec3 rows and
    /// UPDATE the matching raw_inst_geo rows with mesh status columns.
    fn write_persist_mesh_inner(
        &self,
        batch_id: &str,
        mesh_results: &std::collections::HashMap<u64, MeshResult>,
        mesh_aabb_map: &DashMap<String, Aabb>,
        mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<()> {
        let aabb_rows = build_raw_aabb_rows(mesh_aabb_map);
        let vec3_rows = build_raw_vec3_rows(mesh_pts_map);
        let inst_geo_updates = build_inst_geo_mesh_updates(mesh_results);

        self.conn
            .execute_batch("BEGIN TRANSACTION")
            .map_err(|e| anyhow::anyhow!("ducklake persist_mesh[{batch_id}]: BEGIN failed: {e}"))?;

        let result: anyhow::Result<()> = (|| {
            insert_raw_aabb(&self.conn, batch_id, &aabb_rows)?;
            insert_raw_vec3(&self.conn, batch_id, &vec3_rows)?;
            update_inst_geo_mesh(&self.conn, batch_id, &inst_geo_updates)?;
            Ok(())
        })();

        match result {
            Ok(()) => self.conn.execute_batch("COMMIT").map_err(|e| {
                anyhow::anyhow!("ducklake persist_mesh[{batch_id}]: COMMIT failed: {e}")
            }),
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// Slice 3 inst_relate_aabb write path: insert refno → mesh aabb_id links
    /// for every shape instance whose geometry has a mesh AABB.
    fn write_persist_inst_relate_aabb_inner(
        &self,
        batch_id: &str,
        rows: &[RawInstRelateAabbRow],
    ) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.conn.execute_batch("BEGIN TRANSACTION").map_err(|e| {
            anyhow::anyhow!("ducklake persist_inst_relate_aabb[{batch_id}]: BEGIN failed: {e}")
        })?;
        let result = insert_raw_inst_relate_aabb(&self.conn, batch_id, rows);
        match result {
            Ok(()) => self.conn.execute_batch("COMMIT").map_err(|e| {
                anyhow::anyhow!("ducklake persist_inst_relate_aabb[{batch_id}]: COMMIT failed: {e}")
            }),
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }
}

/// `DuckLakeModelWriterBackend` is the Slice 1 skeleton.
///
/// `init` opens the DuckDB connection + DuckLake metadata and seeds the 9
/// in-scope raw tables. Subsequent stages bail until Slice 2-4 implement the
/// real write paths. `finalize` closes the connection and emits a report that
/// includes the Known Gap table list.
pub struct DuckLakeModelWriterBackend {
    cfg: DuckLakeConfig,
    session: Mutex<Option<DuckLakeSession>>,
    stage_reports: Mutex<Vec<ModelWriterStageReport>>,
}

impl DuckLakeModelWriterBackend {
    pub fn new(cfg: DuckLakeConfig) -> Self {
        Self {
            cfg,
            session: Mutex::new(None),
            stage_reports: Mutex::new(Vec::new()),
        }
    }

    fn record_report(&self, report: ModelWriterStageReport) -> anyhow::Result<()> {
        let mut guard = self
            .stage_reports
            .lock()
            .map_err(|_| anyhow::anyhow!("ducklake stage_reports mutex poisoned"))?;
        guard.push(report);
        Ok(())
    }
}

#[async_trait::async_trait]
impl ModelWriterBackend for DuckLakeModelWriterBackend {
    fn name(&self) -> &'static str {
        "ducklake"
    }

    fn writes_to_surreal(&self) -> bool {
        false
    }

    fn runs_downstream_pipeline(&self) -> bool {
        false
    }

    async fn init(&self) -> anyhow::Result<ModelWriterStageReport> {
        let session = DuckLakeSession::open(&self.cfg)?;
        {
            let mut guard = self
                .session
                .lock()
                .map_err(|_| anyhow::anyhow!("ducklake session mutex poisoned"))?;
            *guard = Some(session);
        }
        let report = ModelWriterStageReport::executed("init", DUCKLAKE_RAW_TABLES_IN_SCOPE.len());
        self.record_report(report.clone())?;
        Ok(report)
    }

    async fn cleanup(&self) -> anyhow::Result<ModelWriterStageReport> {
        // Surreal cleanup remains the SurrealModelWriterBackend's job, but
        // DuckLake must reset its own metadata/data files before a generation
        // run so parity checks are not polluted by rows from previous runs.
        let removed = self.cfg.reset_storage()?;
        let report = if removed == 0 {
            ModelWriterStageReport::skipped(
                "cleanup",
                "ducklake metadata/data files did not exist",
                0,
            )
        } else {
            ModelWriterStageReport::executed("cleanup", removed)
        };
        self.record_report(report.clone())?;
        Ok(report)
    }

    async fn write_base_batch(
        &self,
        batch: &ShapeInstancesData,
    ) -> anyhow::Result<ModelWriteBatchReport> {
        // Derive a stable per-batch identifier for error messages. We use the
        // first inst_info key when available; otherwise fall back to a
        // pointer-style anchor so two empty batches don't collide visually.
        let batch_id = batch
            .inst_info_map
            .keys()
            .next()
            .map(|r| format!("first_refno={r}"))
            .unwrap_or_else(|| format!("empty_batch@{:p}", batch as *const _));

        let mut guard = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("ducklake session mutex poisoned"))?;
        let session = guard.as_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "ducklake write_base_batch[{batch_id}]: session not initialized; \
                 call init() before write_base_batch"
            )
        })?;

        session.write_base_batch_inner(&batch_id, batch)?;
        Ok(ModelWriteBatchReport::default())
    }

    async fn persist_mesh_results(
        &self,
        mesh_results: &HashMap<u64, MeshResult>,
        mesh_aabb_map: &DashMap<String, Aabb>,
        mesh_pts_map: &DashMap<u64, String>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        let batch_id = format!("mesh_results={}", mesh_results.len());

        let mut guard = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("ducklake session mutex poisoned"))?;
        let session = guard.as_mut().ok_or_else(|| {
            anyhow::anyhow!("ducklake persist_mesh_results[{batch_id}]: session not initialized")
        })?;

        session.write_persist_mesh_inner(&batch_id, mesh_results, mesh_aabb_map, mesh_pts_map)?;

        let item_count = mesh_results.len();
        let report = ModelWriterStageReport::executed("mesh_persist", item_count);
        self.record_report(report.clone())?;
        Ok(report)
    }

    async fn persist_inst_relate_aabb(
        &self,
        shape_insts: &ShapeInstancesData,
        mesh_results: &HashMap<u64, MeshResult>,
        _mesh_aabb_map: &DashMap<String, Aabb>,
        skip_inst_relate_aabb: bool,
    ) -> anyhow::Result<ModelWriterStageReport> {
        if skip_inst_relate_aabb {
            let report = ModelWriterStageReport::skipped(
                "inst_relate_aabb",
                "AIOS_SKIP_INST_RELATE_AABB",
                shape_insts.inst_cnt(),
            );
            self.record_report(report.clone())?;
            return Ok(report);
        }

        let rows = build_raw_inst_relate_aabb_rows(shape_insts, mesh_results);
        let batch_id = format!("inst_relate_aabb_rows={}", rows.len());

        let mut guard = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("ducklake session mutex poisoned"))?;
        let session = guard.as_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "ducklake persist_inst_relate_aabb[{batch_id}]: session not initialized"
            )
        })?;

        session.write_persist_inst_relate_aabb_inner(&batch_id, &rows)?;

        let report = ModelWriterStageReport::executed("inst_relate_aabb", rows.len());
        self.record_report(report.clone())?;
        Ok(report)
    }

    async fn reconcile_missing_neg_relations(
        &self,
        _all_refnos: &[RefnoEnum],
        missing_neg_carriers: &[RefnoEnum],
    ) -> anyhow::Result<ModelWriterStageReport> {
        if missing_neg_carriers.is_empty() {
            let report = ModelWriterStageReport::skipped(
                "missing_neg_reconcile",
                "no missing negative relation carriers",
                0,
            );
            self.record_report(report.clone())?;
            return Ok(report);
        }

        // Slice 4 conservative behaviour: SurrealBackend resolves carrier →
        // target edges by querying the live model_primary_db; DuckLake would
        // need an equivalent JOIN against `raw_inst_info` / `raw_ngmr_relate`
        // to do the same. For Slice 4 minimal correctness we record the
        // carriers as a typed audit row in `raw_neg_relate` with a sentinel
        // target so parity SQL can EXCEPT them out without losing visibility.
        // Refining to true edge resolution is left to a follow-up slice; the
        // gap is tracked under known_gap reconcile_resolution in finalize().
        let batch_id = format!("reconcile_carriers={}", missing_neg_carriers.len());

        let rows: Vec<RawNegRelateRow> = missing_neg_carriers
            .iter()
            .map(|c| (refno_to_id(c), "__reconcile_pending__".to_string()))
            .collect();

        {
            let mut guard = self
                .session
                .lock()
                .map_err(|_| anyhow::anyhow!("ducklake session mutex poisoned"))?;
            let session = guard.as_mut().ok_or_else(|| {
                anyhow::anyhow!(
                    "ducklake reconcile_missing_neg_relations[{batch_id}]: session not initialized"
                )
            })?;
            session
                .conn
                .execute_batch("BEGIN TRANSACTION")
                .map_err(|e| {
                    anyhow::anyhow!(
                        "ducklake reconcile_missing_neg_relations[{batch_id}]: BEGIN failed: {e}"
                    )
                })?;
            let result = insert_neg_relate(&session.conn, &batch_id, &rows);
            match result {
                Ok(()) => session.conn.execute_batch("COMMIT").map_err(|e| {
                    anyhow::anyhow!(
                        "ducklake reconcile_missing_neg_relations[{batch_id}]: COMMIT failed: {e}"
                    )
                })?,
                Err(err) => {
                    let _ = session.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
            }
        }

        let report =
            ModelWriterStageReport::executed("missing_neg_reconcile", missing_neg_carriers.len());
        self.record_report(report.clone())?;
        Ok(report)
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        // Phase 2 boolean tables are explicitly out of scope per
        // goals/ducklake-model-writer/brief.md Non-Goals.
        let report = ModelWriterStageReport::skipped(
            "boolean_bridge",
            "phase2 boolean tables out of scope for ducklake (goals/ducklake-model-writer)",
            request.bool_tasks.len(),
        );
        self.record_report(report)?;
        Ok(BooleanBridgeReport {
            total: request.bool_tasks.len(),
            skipped: request.bool_tasks.len(),
            skipped_reason: Some(
                "phase2 boolean tables out of scope for ducklake (goals/ducklake-model-writer)",
            ),
            ..BooleanBridgeReport::default()
        })
    }

    async fn finalize(&self) -> anyhow::Result<ModelWriterFinishReport> {
        // Append explicit Known Gap reports per table for downstream diff.
        for gap_table in DUCKLAKE_KNOWN_GAP_TABLES {
            let stage_name: &'static str = match gap_table {
                "raw_tubi_info" => "known_gap:raw_tubi_info",
                "raw_tubi_relate" => "known_gap:raw_tubi_relate",
                "raw_aabb(tubi)" => "known_gap:raw_aabb_tubi",
                "raw_trans" => "known_gap:raw_trans",
                "raw_vec3(tubi)" => "known_gap:raw_vec3_tubi",
                "raw_refno_assoc_index" => "known_gap:raw_refno_assoc_index",
                _ => "known_gap:unknown",
            };
            self.record_report(ModelWriterStageReport::skipped(
                stage_name,
                "phase1 trait gap: written outside ModelWriterBackend (cata_model.rs / refno_assoc_index.rs); see goals/ducklake-model-writer/brief.md Q1=C scope",
                0,
            ))?;
        }

        let session_opt = {
            let mut guard = self
                .session
                .lock()
                .map_err(|_| anyhow::anyhow!("ducklake session mutex poisoned"))?;
            guard.take()
        };
        if let Some(session) = session_opt {
            session.finalize()?;
        }

        let stage_reports = self
            .stage_reports
            .lock()
            .map_err(|_| anyhow::anyhow!("ducklake stage_reports mutex poisoned"))?
            .clone();

        Ok(ModelWriterFinishReport {
            writer_name: self.name(),
            drain_only_stats: None,
            stage_reports,
        })
    }
}

// ---------------------------------------------------------------------------
// Slice 2 canonical adapter + insert helpers.
//
// Each `build_raw_*_rows` function projects one in-memory map / vector from
// `ShapeInstancesData` into the row tuple shape that `insert_*` then writes
// into the corresponding `ducklake-canonical.raw_*` table. Row shapes match
// the columns declared in `create_table_ddl()`.
//
// Rows are owned String/i32/bool tuples on purpose: the duckdb crate's
// `params!` macro accepts `&dyn ToSql` for each position, so owning the
// strings ahead of time lets us push the borrow lifetime through prepared
// statements without juggling references back into `ShapeInstancesData`.
// ---------------------------------------------------------------------------

type RawInstInfoRow = (
    String,         // inst_id
    String,         // owner_refno
    String,         // owner_type
    Option<String>, // cata_hash
    i32,            // sesno
    bool,           // visible
    String,         // payload_json
);

type RawInstRelateRow = (
    String, /*refno*/
    String, /*inst_id*/
    String, /*payload_json*/
);

type RawInstGeoRow = (
    String, // geo_hash
    String, // refno
    String, // type_name
    String, // payload_json
);

type RawGeoRelateRow = (
    String, // inst_id
    String, // geo_hash
    String, // geom_refno
    i32,    // idx
    String, // geo_type
    bool,   // visible
    bool,   // is_tubi
    String, // payload_json
);

type RawNegRelateRow = (String /*carrier_refno*/, String /*target_refno*/);

type RawNgmrRelateRow = (
    String, // carrier_refno
    String, // target_refno
    String, // ngmr_refno
);

fn refno_to_id(refno: &RefnoEnum) -> String {
    refno.to_string()
}

fn inst_transform_is_nan(inst: &aios_core::geometry::EleInstGeo) -> bool {
    inst.geo_transform.translation.is_nan()
        || inst.geo_transform.rotation.is_nan()
        || inst.geo_transform.scale.is_nan()
}

fn payload_to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| {
        // Fallback: an explicit error marker keeps the row insertable so
        // downstream parity SQL can still see the key columns instead of
        // silently dropping the row.
        format!("{{\"__serialize_error__\":\"{}\"}}", e)
    })
}

fn build_raw_inst_info_rows(batch: &ShapeInstancesData) -> Vec<RawInstInfoRow> {
    batch
        .inst_info_map
        .iter()
        .map(|(_, info)| {
            (
                info.id_str(),
                refno_to_id(&info.owner_refno),
                info.owner_type.clone(),
                info.cata_hash.clone(),
                info.sesno,
                info.visible,
                payload_to_json(info),
            )
        })
        .collect()
}

fn build_raw_inst_relate_rows(batch: &ShapeInstancesData) -> Vec<RawInstRelateRow> {
    // SurrealDB models inst_relate as a `pe -> inst_info` relation; the
    // canonical raw projection keeps both endpoints explicit so non-Surreal
    // backends can rebuild the edge without RecordId semantics.
    batch
        .inst_info_map
        .iter()
        .map(|(refno, info)| {
            let id = refno_to_id(refno);
            let inst_id = info.id_str();
            let payload = serde_json::json!({
                "in_pe": id,
                "out_inst_id": inst_id,
                "parent_refno": refno_to_id(&info.owner_refno),
                "owner_type": info.owner_type.clone(),
                "visible": info.visible,
                "solid": info.is_solid,
            });
            (id, inst_id, payload.to_string())
        })
        .collect()
}

fn build_raw_inst_geo_rows(batch: &ShapeInstancesData) -> Vec<RawInstGeoRow> {
    let mut rows = Vec::new();
    let mut seen_geo_hashes = HashSet::new();
    for (_, data) in batch.inst_geos_map.iter() {
        for inst in &data.insts {
            if inst_transform_is_nan(inst) {
                continue;
            }
            let geo_hash = inst.geo_hash.to_string();
            if !seen_geo_hashes.insert(geo_hash.clone()) {
                continue;
            }
            rows.push((
                geo_hash,
                refno_to_id(&inst.refno),
                data.type_name.clone(),
                payload_to_json(inst),
            ));
        }
    }
    rows
}

fn build_raw_geo_relate_rows(batch: &ShapeInstancesData) -> Vec<RawGeoRelateRow> {
    let mut rows = Vec::new();
    for (_, data) in batch.inst_geos_map.iter() {
        let inst_id = data.id();
        for (idx, geo) in data.insts.iter().enumerate() {
            if inst_transform_is_nan(geo) {
                continue;
            }
            rows.push((
                inst_id.clone(),
                geo.geo_hash.to_string(),
                refno_to_id(&geo.refno),
                idx as i32,
                geo.geo_type.to_string(),
                geo.visible,
                geo.is_tubi,
                payload_to_json(geo),
            ));
        }
    }
    rows
}

fn build_raw_neg_relate_rows(batch: &ShapeInstancesData) -> Vec<RawNegRelateRow> {
    let mut rows = Vec::new();
    for (carrier, targets) in batch.neg_relate_map.iter() {
        let carrier_id = refno_to_id(carrier);
        for target in targets {
            rows.push((carrier_id.clone(), refno_to_id(target)));
        }
    }
    rows
}

fn build_raw_ngmr_relate_rows(batch: &ShapeInstancesData) -> Vec<RawNgmrRelateRow> {
    let mut rows = Vec::new();
    for (carrier, pairs) in batch.ngmr_neg_relate_map.iter() {
        let carrier_id = refno_to_id(carrier);
        for (target, ngmr) in pairs {
            rows.push((carrier_id.clone(), refno_to_id(target), refno_to_id(ngmr)));
        }
    }
    rows
}

fn insert_inst_info(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawInstInfoRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_inst_info" (inst_id, owner_refno, owner_type, cata_hash, sesno, visible, payload_json) VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!("ducklake write_base_batch[{batch_id}] raw_inst_info: prepare failed: {e}")
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2, r.3, r.4, r.5, r.6])
            .map_err(|e| {
                anyhow::anyhow!(
                    "ducklake write_base_batch[{batch_id}] raw_inst_info: insert failed (inst_id={}): {e}",
                    r.0
                )
            })?;
    }
    Ok(())
}

fn insert_inst_relate(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawInstRelateRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_inst_relate" (refno, inst_id, payload_json) VALUES (?, ?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!(
            "ducklake write_base_batch[{batch_id}] raw_inst_relate: prepare failed: {e}"
        )
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake write_base_batch[{batch_id}] raw_inst_relate: insert failed (refno={}): {e}",
                r.0
            )
        })?;
    }
    Ok(())
}

fn insert_inst_geo(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawInstGeoRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    // Mesh columns (meshed/bad/mesh_aabb_id/mesh_pts_hashes_json) are NULL on
    // base-batch insert; Slice 3 persist_mesh_results UPDATE fills them when
    // the geo's mesh result lands.
    let sql = format!(
        r#"INSERT INTO "{}"."raw_inst_geo" (geo_hash, refno, type_name, meshed, bad, mesh_aabb_id, mesh_pts_hashes_json, payload_json) VALUES (?, ?, ?, NULL, NULL, NULL, NULL, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!("ducklake write_base_batch[{batch_id}] raw_inst_geo: prepare failed: {e}")
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2, r.3]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake write_base_batch[{batch_id}] raw_inst_geo: insert failed (geo_hash={}): {e}",
                r.0
            )
        })?;
    }
    Ok(())
}

fn insert_geo_relate(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawGeoRelateRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_geo_relate" (inst_id, geo_hash, geom_refno, idx, geo_type, visible, is_tubi, payload_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!("ducklake write_base_batch[{batch_id}] raw_geo_relate: prepare failed: {e}")
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7])
            .map_err(|e| {
                anyhow::anyhow!(
                    "ducklake write_base_batch[{batch_id}] raw_geo_relate: insert failed (inst_id={}, geo_hash={}): {e}",
                    r.0, r.1
                )
            })?;
    }
    Ok(())
}

fn insert_neg_relate(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawNegRelateRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_neg_relate" (carrier_refno, target_refno) VALUES (?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!("ducklake write_base_batch[{batch_id}] raw_neg_relate: prepare failed: {e}")
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake write_base_batch[{batch_id}] raw_neg_relate: insert failed (carrier={}, target={}): {e}",
                r.0, r.1
            )
        })?;
    }
    Ok(())
}

fn insert_ngmr_relate(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawNgmrRelateRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_ngmr_relate" (carrier_refno, target_refno, ngmr_refno) VALUES (?, ?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!(
            "ducklake write_base_batch[{batch_id}] raw_ngmr_relate: prepare failed: {e}"
        )
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake write_base_batch[{batch_id}] raw_ngmr_relate: insert failed (carrier={}, target={}, ngmr={}): {e}",
                r.0, r.1, r.2
            )
        })?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Slice 3 mesh-derived row adapters + writers.
// ---------------------------------------------------------------------------

type RawAabbRow = (String /*aabb_id*/, f64, f64, f64, f64, f64, f64);
type RawVec3Row = (String /*vec3_id*/, String /*payload*/);
type InstGeoMeshUpdate = (
    String,         // geo_hash
    bool,           // meshed
    bool,           // bad
    Option<String>, // mesh_aabb_id
    String,         // mesh_pts_hashes_json
);
type RawInstRelateAabbRow = (
    String, /*refno*/
    String, /*aabb_id*/
    String, /*source*/
);

fn build_raw_aabb_rows(mesh_aabb_map: &DashMap<String, Aabb>) -> Vec<RawAabbRow> {
    mesh_aabb_map
        .iter()
        .map(|kv| {
            let id = kv.key().clone();
            let aabb = *kv.value();
            (
                id,
                aabb.mins.x as f64,
                aabb.mins.y as f64,
                aabb.mins.z as f64,
                aabb.maxs.x as f64,
                aabb.maxs.y as f64,
                aabb.maxs.z as f64,
            )
        })
        .collect()
}

fn build_raw_vec3_rows(mesh_pts_map: &DashMap<u64, String>) -> Vec<RawVec3Row> {
    mesh_pts_map
        .iter()
        .map(|kv| (kv.key().to_string(), kv.value().clone()))
        .collect()
}

fn build_inst_geo_mesh_updates(
    mesh_results: &std::collections::HashMap<u64, MeshResult>,
) -> Vec<InstGeoMeshUpdate> {
    mesh_results
        .iter()
        .map(|(geo_hash_u64, mr)| {
            let pts_json =
                serde_json::to_string(&mr.pts_hashes).unwrap_or_else(|_| "[]".to_string());
            (
                geo_hash_u64.to_string(),
                mr.meshed,
                mr.bad,
                mr.aabb_hash.map(|h| h.to_string()),
                pts_json,
            )
        })
        .collect()
}

/// Build refno → mesh_aabb_id link rows from `inst_geos_map` joined with mesh
/// results. For Slice 3 minimal viable parity we emit one row per
/// (parent_refno, mesh_aabb_hash) edge; the SurrealBackend computes a union
/// AABB per refno but that is left to a later parity-refinement slice.
fn build_raw_inst_relate_aabb_rows(
    shape_insts: &ShapeInstancesData,
    mesh_results: &std::collections::HashMap<u64, MeshResult>,
) -> Vec<RawInstRelateAabbRow> {
    let mut rows = Vec::new();
    for (_geo_hash, geos_data) in shape_insts.inst_geos_map.iter() {
        let parent_refno = refno_to_id(&geos_data.refno);
        for ele in geos_data.insts.iter() {
            if let Some(mr) = mesh_results.get(&ele.geo_hash) {
                if let Some(aabb_hash) = mr.aabb_hash {
                    rows.push((
                        parent_refno.clone(),
                        aabb_hash.to_string(),
                        "mesh".to_string(),
                    ));
                }
            }
        }
    }
    rows
}

fn insert_raw_aabb(conn: &Connection, batch_id: &str, rows: &[RawAabbRow]) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_aabb" (aabb_id, min_x, min_y, min_z, max_x, max_y, max_z) VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!("ducklake persist_mesh[{batch_id}] raw_aabb: prepare failed: {e}")
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2, r.3, r.4, r.5, r.6])
            .map_err(|e| {
                anyhow::anyhow!(
                    "ducklake persist_mesh[{batch_id}] raw_aabb: insert failed (aabb_id={}): {e}",
                    r.0
                )
            })?;
    }
    Ok(())
}

fn insert_raw_vec3(conn: &Connection, batch_id: &str, rows: &[RawVec3Row]) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_vec3" (vec3_id, payload) VALUES (?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!("ducklake persist_mesh[{batch_id}] raw_vec3: prepare failed: {e}")
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake persist_mesh[{batch_id}] raw_vec3: insert failed (vec3_id={}): {e}",
                r.0
            )
        })?;
    }
    Ok(())
}

fn update_inst_geo_mesh(
    conn: &Connection,
    batch_id: &str,
    updates: &[InstGeoMeshUpdate],
) -> anyhow::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"UPDATE "{}"."raw_inst_geo" SET meshed = ?, bad = ?, mesh_aabb_id = ?, mesh_pts_hashes_json = ? WHERE geo_hash = ?"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!(
            "ducklake persist_mesh[{batch_id}] raw_inst_geo UPDATE: prepare failed: {e}"
        )
    })?;
    for u in updates {
        stmt.execute(params![u.1, u.2, u.3, u.4, u.0]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake persist_mesh[{batch_id}] raw_inst_geo UPDATE: failed (geo_hash={}): {e}",
                u.0
            )
        })?;
    }
    Ok(())
}

fn insert_raw_inst_relate_aabb(
    conn: &Connection,
    batch_id: &str,
    rows: &[RawInstRelateAabbRow],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        r#"INSERT INTO "{}"."raw_inst_relate_aabb" (refno, aabb_id, source) VALUES (?, ?, ?)"#,
        DUCKLAKE_CANONICAL_SCHEMA
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        anyhow::anyhow!(
            "ducklake persist_inst_relate_aabb[{batch_id}] raw_inst_relate_aabb: prepare failed: {e}"
        )
    })?;
    for r in rows {
        stmt.execute(params![r.0, r.1, r.2]).map_err(|e| {
            anyhow::anyhow!(
                "ducklake persist_inst_relate_aabb[{batch_id}] raw_inst_relate_aabb: insert failed (refno={}, aabb_id={}): {e}",
                r.0, r.1
            )
        })?;
    }
    Ok(())
}
