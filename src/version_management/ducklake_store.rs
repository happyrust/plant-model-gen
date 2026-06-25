use crate::version_management::types::{
    ModelComponentDiffResponse, ModelComponentDiffRow, ModelComponentDiffSummary,
    ModelComponentSnapshotStats, ModelComponentUnitImpactResponse, ModelComponentUnitImpactRow,
    ModelComponentUnitImpactSummary, ModelReleaseEventsResponse, ModelReleaseFile,
    ModelReleaseLifecycle, ModelReleaseListResponse, ModelReleaseMeshAsset,
    ModelReleaseMeshAssetIndexResponse, ModelReleaseMeshAssetIndexStats,
    ModelReleasePairReadinessResponse, ModelReleaseQuality, ModelReleaseReadinessEvidence,
    ModelReleaseReconcileReport, ModelReleaseRecord, ModelReleaseRegistration,
    ModelReleaseRegistrationStatus, ModelReleaseSceneAabb, ModelReleaseSceneComponent,
    ModelReleaseSceneGeometry, ModelReleaseSceneMeshAssetEvidence, ModelReleaseSceneResponse,
    ModelReleaseStatus, ModelReleaseStatusEvent, ModelUnitDiffResponse, ModelUnitDiffRow,
    ModelUnitDiffSummary, ModelUnitIndexStats, ModelVersionCatalogMigrationReport,
    ModelVersionDuckLakeConfig,
};

#[cfg(feature = "model-version-ducklake")]
mod imp {
    use super::*;
    use anyhow::Context;
    use chrono::{SecondsFormat, Utc};
    use duckdb::{Connection, params};
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime};

    const SCHEMA: &str = "model_version";
    const COMPONENT_HASH_VERSION: &str = "component_snapshot:v1";
    const UNIT_HASH_VERSION: &str = "unit_version:v1";
    const UNIT_RULE_SET_HASH: &str = "unit_impact_rules:v1";
    const MEMBERSHIP_HASH_VERSION: &str = "delivery_unit_membership:v1";
    const COMPONENT_CHANGE_RULE_ID: &str = "component_hash_changes_delivery_unit:v1";
    const METADATA_LOCK_TIMEOUT: Duration = Duration::from_secs(120);
    const METADATA_LOCK_STALE_AFTER: Duration = Duration::from_secs(30 * 60);

    fn required_tables() -> &'static [&'static str] {
        &[
            "model_version_schema_migrations",
            "model_releases",
            "model_release_files",
            "model_release_metadata",
            "model_release_edges",
            "model_release_status_events",
            "component_snapshots",
            "component_index_runs",
            "model_release_mesh_assets",
            "model_release_mesh_asset_index_runs",
            "delivery_unit_memberships",
            "unit_versions",
            "unit_index_runs",
        ]
    }

    fn required_release_columns() -> &'static [&'static str] {
        &[
            "release_status",
            "release_lifecycle",
            "release_quality",
            "release_quality_reason",
            "validation_flags_json",
            "spec_info_fallback_count",
            "source_manifest_path",
            "source_manifest_hash",
            "baseline_state_manifest_path",
            "baseline_state_manifest_hash",
            "generation_job_id",
            "asset_manifest_path",
            "asset_manifest_hash",
        ]
    }

    const MIGRATION_BASE_MODEL_VERSION_SCHEMA: &str = "0001_base_model_version_schema";
    const MIGRATION_RELEASE_LIFECYCLE_QUALITY_COLUMNS: &str =
        "0002_release_lifecycle_quality_columns";
    const MIGRATION_RELEASE_QUALITY_EVIDENCE_COLUMNS: &str =
        "0003_release_quality_evidence_columns";
    const MIGRATION_RELEASE_PROVENANCE_COLUMNS: &str = "0004_release_provenance_columns";
    const MIGRATION_RELEASE_STATUS_LIFECYCLE_QUALITY_BACKFILL: &str =
        "0005_release_status_lifecycle_quality_backfill";
    const MIGRATION_MESH_ASSET_GLB_READABILITY_COLUMNS: &str =
        "0006_mesh_asset_glb_readability_columns";

    fn required_schema_migrations() -> &'static [(&'static str, &'static str)] {
        &[
            (
                MIGRATION_BASE_MODEL_VERSION_SCHEMA,
                "Base model-version release, edge, file, metadata, component, asset, and unit tables are present.",
            ),
            (
                MIGRATION_RELEASE_LIFECYCLE_QUALITY_COLUMNS,
                "Release status, lifecycle, and quality columns are present.",
            ),
            (
                MIGRATION_RELEASE_QUALITY_EVIDENCE_COLUMNS,
                "Release quality reason, validation flags, and spec fallback count columns are present.",
            ),
            (
                MIGRATION_RELEASE_PROVENANCE_COLUMNS,
                "Source manifest, baseline state, generation job, and asset manifest provenance columns are present.",
            ),
            (
                MIGRATION_RELEASE_STATUS_LIFECYCLE_QUALITY_BACKFILL,
                "Existing release rows have status, lifecycle, and quality compatibility backfills.",
            ),
            (
                MIGRATION_MESH_ASSET_GLB_READABILITY_COLUMNS,
                "Mesh asset indexes record GLB readability evidence.",
            ),
        ]
    }

    pub struct ModelVersionDuckLakeStore {
        cfg: ModelVersionDuckLakeConfig,
        conn: Connection,
        _metadata_lock: Option<MetadataFileLock>,
    }

    enum StoreOpenMode {
        Writer,
        ReadOnly,
    }

    impl ModelVersionDuckLakeStore {
        pub fn open(cfg: ModelVersionDuckLakeConfig) -> anyhow::Result<Self> {
            Self::open_writer(cfg)
        }

        pub fn open_writer(cfg: ModelVersionDuckLakeConfig) -> anyhow::Result<Self> {
            Self::open_inner(cfg, StoreOpenMode::Writer)
        }

        pub fn open_readonly(cfg: ModelVersionDuckLakeConfig) -> anyhow::Result<Self> {
            Self::open_inner(cfg, StoreOpenMode::ReadOnly)
        }

        fn open_inner(
            cfg: ModelVersionDuckLakeConfig,
            mode: StoreOpenMode,
        ) -> anyhow::Result<Self> {
            let is_writer = matches!(mode, StoreOpenMode::Writer);
            if let Some(parent) = cfg.metadata_path.parent() {
                if is_writer {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "create DuckLake metadata parent failed: {}",
                            parent.display()
                        )
                    })?;
                } else if !parent.exists() {
                    anyhow::bail!(
                        "DuckLake metadata parent does not exist for read-only access: {}",
                        parent.display()
                    );
                }
            }
            if is_writer {
                std::fs::create_dir_all(&cfg.data_path).with_context(|| {
                    format!(
                        "create DuckLake data path failed: {}",
                        cfg.data_path.display()
                    )
                })?;
            } else {
                if !cfg.metadata_path.is_file() {
                    anyhow::bail!(
                        "DuckLake metadata file does not exist for read-only access: {}",
                        cfg.metadata_path.display()
                    );
                }
                if !cfg.data_path.exists() {
                    anyhow::bail!(
                        "DuckLake data path does not exist for read-only access: {}",
                        cfg.data_path.display()
                    );
                }
            }
            let mode_label = if is_writer { "writer" } else { "read-only" };
            let metadata_lock = Some(MetadataFileLock::acquire(&cfg.metadata_path).with_context(
                || {
                    format!(
                        "acquire DuckLake metadata access lock for {mode_label} open failed: {}",
                        cfg.metadata_path.display()
                    )
                },
            )?);

            let conn = Connection::open_in_memory()
                .context("open in-memory DuckDB connection for model versions")?;
            conn.execute_batch("INSTALL ducklake; LOAD ducklake;")
                .map_err(|e| {
                    anyhow::anyhow!(
                        "DuckLake extension install/load failed: {e}. \
                         Enable network for first run or pre-install the DuckDB ducklake extension."
                    )
                })?;

            let metadata_uri = format!("ducklake:{}", duckdb_path(&cfg.metadata_path));
            let data_path = escape_sql_string(&duckdb_path(&cfg.data_path));
            let attach_options = if is_writer {
                format!("DATA_PATH '{data_path}', OVERRIDE_DATA_PATH true")
            } else {
                format!("DATA_PATH '{data_path}', OVERRIDE_DATA_PATH true, READ_ONLY")
            };
            let attach_sql = format!(
                "ATTACH '{}' AS {} ({}); USE {};",
                escape_sql_string(&metadata_uri),
                cfg.catalog_name,
                attach_options,
                cfg.catalog_name
            );
            conn.execute_batch(&attach_sql).with_context(|| {
                format!(
                    "attach DuckLake metadata failed: {}",
                    cfg.metadata_path.display()
                )
            })?;

            let store = Self {
                cfg,
                conn,
                _metadata_lock: metadata_lock,
            };
            if is_writer {
                store.ensure_schema()?;
            } else {
                store.validate_read_schema()?;
            }
            Ok(store)
        }

        fn ensure_schema(&self) -> anyhow::Result<()> {
            let ddl = format!(
                r#"
CREATE SCHEMA IF NOT EXISTS "{schema}";

CREATE TABLE IF NOT EXISTS "{schema}"."model_version_schema_migrations" (
    migration_id TEXT,
    applied_at TEXT,
    note TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_releases" (
    release_id TEXT,
    project_name TEXT,
    branch_id TEXT,
    release_lifecycle TEXT,
    release_quality TEXT,
    release_quality_reason TEXT,
    validation_flags_json TEXT,
    spec_info_fallback_count BIGINT,
    release_status TEXT,
    release_label TEXT,
    dbnum INTEGER,
    source_package_dir TEXT,
    immutable_package_dir TEXT,
    package_hash TEXT,
    derivation_type TEXT,
    created_at TEXT,
    registered_at TEXT,
    rows_instances BIGINT,
    rows_geo_instances BIGINT,
    rows_transforms BIGINT,
    rows_aabb BIGINT,
    rows_tubings BIGINT,
    rows_ptsets BIGINT,
    rows_primitive_keypoints BIGINT,
    source_manifest_path TEXT,
    source_manifest_hash TEXT,
    baseline_state_manifest_path TEXT,
    baseline_state_manifest_hash TEXT,
    generation_job_id TEXT,
    asset_manifest_path TEXT,
    asset_manifest_hash TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_release_status_events" (
    release_id TEXT,
    release_status TEXT,
    reason TEXT,
    created_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_release_edges" (
    release_id TEXT,
    parent_release_id TEXT,
    edge_type TEXT,
    created_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_release_files" (
    release_id TEXT,
    dbnum INTEGER,
    logical_name TEXT,
    relative_path TEXT,
    absolute_path TEXT,
    bytes BIGINT,
    sha256 TEXT,
    rows BIGINT,
    required BOOLEAN,
    created_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_release_mesh_assets" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    lod_tag TEXT,
    geo_hash TEXT,
    builtin BOOLEAN,
    asset_exists BOOLEAN,
    mesh_relative_path TEXT,
    mesh_absolute_path TEXT,
    mesh_url TEXT,
    bytes BIGINT,
    sha256 TEXT,
    glb_readable BOOLEAN,
    glb_validation_error TEXT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_release_mesh_asset_index_runs" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    lod_tag TEXT,
    geo_hash_count BIGINT,
    present_count BIGINT,
    missing_count BIGINT,
    builtin_count BIGINT,
    total_bytes BIGINT,
    glb_checked_count BIGINT,
    glb_readable_count BIGINT,
    glb_unreadable_count BIGINT,
    asset_index_hash TEXT,
    manifest_path TEXT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."model_release_metadata" (
    release_id TEXT,
    manifest_json TEXT,
    extra_metadata_json TEXT,
    created_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."component_snapshots" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    component_key TEXT,
    refno_str TEXT,
    refno_u64 BIGINT,
    noun TEXT,
    owner_refno_str TEXT,
    owner_refno_u64 BIGINT,
    owner_noun TEXT,
    cata_hash TEXT,
    trans_hash TEXT,
    aabb_hash TEXT,
    spec_value BIGINT,
    has_neg BOOLEAN,
    geo_signature TEXT,
    component_hash TEXT,
    hash_version TEXT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."component_index_runs" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    hash_version TEXT,
    component_count BIGINT,
    distinct_component_hashes BIGINT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."delivery_unit_memberships" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    unit_key TEXT,
    unit_noun TEXT,
    unit_refno_str TEXT,
    unit_refno_u64 BIGINT,
    component_key TEXT,
    component_refno_str TEXT,
    component_refno_u64 BIGINT,
    component_noun TEXT,
    component_hash TEXT,
    owner_refno_str TEXT,
    owner_refno_u64 BIGINT,
    owner_noun TEXT,
    membership_kind TEXT,
    path_confidence DOUBLE,
    unresolved_reason TEXT,
    membership_hash TEXT,
    hash_version TEXT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."unit_versions" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    unit_key TEXT,
    unit_noun TEXT,
    unit_refno_str TEXT,
    unit_refno_u64 BIGINT,
    unit_version_id TEXT,
    aggregate_hash TEXT,
    hash_version TEXT,
    rule_set_hash TEXT,
    member_count BIGINT,
    unresolved_member_count BIGINT,
    member_signature TEXT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "{schema}"."unit_index_runs" (
    release_id TEXT,
    project_name TEXT,
    dbnum INTEGER,
    hash_version TEXT,
    rule_set_hash TEXT,
    unit_count BIGINT,
    member_count BIGINT,
    unresolved_member_count BIGINT,
    indexed_at TEXT
);
"#,
                schema = SCHEMA
            );
            self.conn
                .execute_batch(&ddl)
                .context("create model_version DuckLake schema")?;
            self.ensure_schema_migrations()?;
            Ok(())
        }

        fn validate_read_schema(&self) -> anyhow::Result<()> {
            if !self.table_exists("model_version_schema_migrations")? {
                anyhow::bail!(
                    "model-version DuckLake catalog is missing schema migration audit table; run `aios-database model-version migrate --project <project>` with a migration-capable build before using read-only APIs"
                );
            }
            if !self.model_releases_provenance_columns_exist()? {
                anyhow::bail!(
                    "model-version DuckLake catalog is missing release provenance columns; run `aios-database model-version migrate --project <project>` with a migration-capable build before using read-only APIs"
                );
            }
            let missing_schema_migrations = self.missing_required_schema_migration_ids()?;
            if !missing_schema_migrations.is_empty() {
                anyhow::bail!(
                    "model-version DuckLake catalog is missing required schema migrations ({missing}); run `aios-database model-version migrate --project <project>` with a migration-capable build before using read-only APIs",
                    missing = missing_schema_migrations.join(", ")
                );
            }
            Ok(())
        }

        fn ensure_schema_migrations(&self) -> anyhow::Result<()> {
            self.record_required_schema_migration(MIGRATION_BASE_MODEL_VERSION_SCHEMA)?;
            self.add_release_column_if_missing("release_status", "TEXT DEFAULT 'published'")?;
            self.add_release_column_if_missing("release_lifecycle", "TEXT")?;
            self.add_release_column_if_missing("release_quality", "TEXT")?;
            self.record_required_schema_migration(MIGRATION_RELEASE_LIFECYCLE_QUALITY_COLUMNS)?;
            self.add_release_column_if_missing("release_quality_reason", "TEXT")?;
            self.add_release_column_if_missing("validation_flags_json", "TEXT")?;
            self.add_release_column_if_missing("spec_info_fallback_count", "BIGINT")?;
            self.record_required_schema_migration(MIGRATION_RELEASE_QUALITY_EVIDENCE_COLUMNS)?;
            self.add_release_column_if_missing("source_manifest_path", "TEXT")?;
            self.add_release_column_if_missing("source_manifest_hash", "TEXT")?;
            self.add_release_column_if_missing("baseline_state_manifest_path", "TEXT")?;
            self.add_release_column_if_missing("baseline_state_manifest_hash", "TEXT")?;
            self.add_release_column_if_missing("generation_job_id", "TEXT")?;
            self.add_release_column_if_missing("asset_manifest_path", "TEXT")?;
            self.add_release_column_if_missing("asset_manifest_hash", "TEXT")?;
            self.record_required_schema_migration(MIGRATION_RELEASE_PROVENANCE_COLUMNS)?;
            let sql = format!(
                "UPDATE \"{}\".\"model_releases\" \
                 SET release_status = 'published' \
                 WHERE release_status IS NULL OR release_status = ''",
                SCHEMA
            );
            self.conn
                .execute_batch(&sql)
                .context("backfill model release statuses")?;
            self.backfill_release_lifecycle_quality()?;
            self.record_required_schema_migration(
                MIGRATION_RELEASE_STATUS_LIFECYCLE_QUALITY_BACKFILL,
            )?;
            self.add_table_column_if_missing(
                "model_release_mesh_assets",
                "glb_readable",
                "BOOLEAN",
            )?;
            self.add_table_column_if_missing(
                "model_release_mesh_assets",
                "glb_validation_error",
                "TEXT",
            )?;
            self.add_table_column_if_missing(
                "model_release_mesh_asset_index_runs",
                "glb_checked_count",
                "BIGINT",
            )?;
            self.add_table_column_if_missing(
                "model_release_mesh_asset_index_runs",
                "glb_readable_count",
                "BIGINT",
            )?;
            self.add_table_column_if_missing(
                "model_release_mesh_asset_index_runs",
                "glb_unreadable_count",
                "BIGINT",
            )?;
            self.record_required_schema_migration(MIGRATION_MESH_ASSET_GLB_READABILITY_COLUMNS)?;
            Ok(())
        }

        fn backfill_release_lifecycle_quality(&self) -> anyhow::Result<()> {
            let lifecycle_sql = format!(
                "UPDATE \"{}\".\"model_releases\" SET release_lifecycle = \
                 CASE \
                   WHEN release_status IN ('staged', 'validating', 'assets_materialized', 'indexed', 'published', 'failed') THEN release_status \
                   WHEN release_status IN ('degraded', 'quarantined', 'patch_only') THEN 'published' \
                   ELSE 'failed' \
                 END \
                 WHERE release_lifecycle IS NULL OR release_lifecycle = ''",
                SCHEMA
            );
            self.conn
                .execute_batch(&lifecycle_sql)
                .context("backfill model release lifecycles")?;

            let quality_sql = format!(
                "UPDATE \"{}\".\"model_releases\" SET release_quality = \
                 CASE \
                   WHEN release_status = 'quarantined' THEN 'quarantined_visual' \
                   WHEN release_status = 'degraded' THEN 'degraded_visual' \
                   WHEN release_status = 'patch_only' THEN 'patch_only' \
                   WHEN lower(concat(COALESCE(release_id, ''), ' ', COALESCE(release_label, ''), ' ', COALESCE(derivation_type, ''))) LIKE '%quarantine%' THEN 'quarantined_visual' \
                   WHEN lower(concat(COALESCE(release_id, ''), ' ', COALESCE(release_label, ''), ' ', COALESCE(derivation_type, ''))) LIKE '%quarantined%' THEN 'quarantined_visual' \
                   WHEN lower(concat(COALESCE(release_id, ''), ' ', COALESCE(release_label, ''), ' ', COALESCE(derivation_type, ''))) LIKE '%partial%' THEN 'degraded_visual' \
                   WHEN lower(concat(COALESCE(release_id, ''), ' ', COALESCE(release_label, ''), ' ', COALESCE(derivation_type, ''))) LIKE '%smoke%' THEN 'degraded_visual' \
                   WHEN COALESCE(rows_instances, 0) = 0 OR COALESCE(rows_geo_instances, 0) = 0 THEN 'non_visual' \
                   ELSE 'complete_visual' \
                 END \
                 WHERE release_quality IS NULL OR release_quality = ''",
                SCHEMA
            );
            self.conn
                .execute_batch(&quality_sql)
                .context("backfill model release qualities")?;
            Ok(())
        }

        fn add_release_column_if_missing(
            &self,
            column_name: &str,
            column_type: &str,
        ) -> anyhow::Result<()> {
            self.add_table_column_if_missing("model_releases", column_name, column_type)
        }

        fn add_table_column_if_missing(
            &self,
            table_name: &str,
            column_name: &str,
            column_type: &str,
        ) -> anyhow::Result<()> {
            if self.column_exists(table_name, column_name)? {
                return Ok(());
            }
            let sql = format!(
                "ALTER TABLE \"{}\".\"{}\" ADD COLUMN {} {}",
                SCHEMA, table_name, column_name, column_type
            );
            self.conn
                .execute_batch(&sql)
                .with_context(|| format!("add {table_name}.{column_name} column"))?;
            Ok(())
        }

        fn record_schema_migration(&self, migration_id: &str, note: &str) -> anyhow::Result<()> {
            let exists_sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"model_version_schema_migrations\" \
                 WHERE migration_id = ?",
                SCHEMA
            );
            let existing: i64 = self
                .conn
                .query_row(&exists_sql, params![migration_id], |row| row.get(0))
                .with_context(|| format!("check schema migration {migration_id}"))?;
            if existing > 0 {
                return Ok(());
            }

            let applied_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let insert_sql = format!(
                "INSERT INTO \"{}\".\"model_version_schema_migrations\" VALUES (?, ?, ?)",
                SCHEMA
            );
            self.conn
                .execute(&insert_sql, params![migration_id, applied_at, note])
                .with_context(|| format!("record schema migration {migration_id}"))?;
            Ok(())
        }

        fn record_required_schema_migration(&self, migration_id: &str) -> anyhow::Result<()> {
            let note = required_schema_migrations()
                .iter()
                .find_map(|(id, note)| (*id == migration_id).then_some(*note))
                .with_context(|| format!("unknown required schema migration id {migration_id}"))?;
            self.record_schema_migration(migration_id, note)
        }

        fn schema_migration_ids(&self) -> anyhow::Result<Vec<String>> {
            let sql = format!(
                "SELECT migration_id FROM \"{}\".\"model_version_schema_migrations\" \
                 ORDER BY migration_id",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            collect_rows(rows).map_err(Into::into)
        }

        fn required_schema_migration_ids(&self) -> Vec<String> {
            required_schema_migrations()
                .iter()
                .map(|(id, _)| (*id).to_string())
                .collect()
        }

        fn missing_required_schema_migration_ids(&self) -> anyhow::Result<Vec<String>> {
            let applied = self
                .schema_migration_ids()?
                .into_iter()
                .collect::<BTreeSet<_>>();
            Ok(self
                .required_schema_migration_ids()
                .into_iter()
                .filter(|id| !applied.contains(id))
                .collect())
        }

        fn model_releases_provenance_columns_exist(&self) -> anyhow::Result<bool> {
            for column in required_release_columns() {
                if !self.column_exists("model_releases", column)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        pub fn catalog_migration_report(
            &self,
            project_name: &str,
        ) -> anyhow::Result<ModelVersionCatalogMigrationReport> {
            let mut required_tables_report = BTreeMap::new();
            for table in required_tables() {
                required_tables_report.insert(table.to_string(), self.table_exists(table)?);
            }

            let mut required_columns_report = BTreeMap::new();
            for column in required_release_columns() {
                required_columns_report.insert(
                    column.to_string(),
                    self.column_exists("model_releases", column)?,
                );
            }

            let release_quality_columns_present = [
                "release_quality",
                "release_quality_reason",
                "validation_flags_json",
                "spec_info_fallback_count",
            ]
            .iter()
            .all(|column| {
                required_columns_report
                    .get(*column)
                    .copied()
                    .unwrap_or(false)
            });

            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"model_releases\" WHERE project_name = ?",
                SCHEMA
            );
            let release_count: i64 = self
                .conn
                .query_row(&sql, params![project_name], |row| row.get(0))?;
            let applied_schema_migrations = self.schema_migration_ids()?;
            let required_schema_migrations = self.required_schema_migration_ids();
            let applied_schema_migration_set = applied_schema_migrations
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            let missing_schema_migrations = required_schema_migrations
                .iter()
                .filter(|id| !applied_schema_migration_set.contains(*id))
                .cloned()
                .collect::<Vec<_>>();
            let missing_tables = required_tables_report
                .iter()
                .filter_map(|(name, exists)| (!*exists).then_some(name.clone()))
                .collect::<Vec<_>>();
            let missing_release_columns = required_columns_report
                .iter()
                .filter_map(|(name, exists)| (!*exists).then_some(name.clone()))
                .collect::<Vec<_>>();

            Ok(ModelVersionCatalogMigrationReport {
                project_name: project_name.to_string(),
                ducklake_metadata_path: self.cfg.metadata_path.clone(),
                ducklake_data_path: self.cfg.data_path.clone(),
                catalog_name: self.cfg.catalog_name.clone(),
                schema_name: SCHEMA.to_string(),
                schema_migration_count: applied_schema_migrations.len() as u64,
                required_schema_migrations,
                applied_schema_migrations,
                missing_schema_migrations,
                release_count: i64_to_u64(release_count, "model release count")?,
                required_tables: required_tables_report,
                required_release_columns: required_columns_report,
                missing_tables,
                missing_release_columns,
                release_quality_columns_present,
                migrated: true,
            })
        }

        fn table_exists(&self, table_name: &str) -> anyhow::Result<bool> {
            let mut stmt = self.conn.prepare(
                "SELECT COUNT(*) FROM information_schema.tables \
                 WHERE table_schema = ? AND table_name = ?",
            )?;
            let count: i64 = stmt.query_row(params![SCHEMA, table_name], |row| row.get(0))?;
            Ok(count > 0)
        }

        fn column_exists(&self, table_name: &str, column_name: &str) -> anyhow::Result<bool> {
            let mut stmt = self.conn.prepare(
                "SELECT COUNT(*) FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? AND column_name = ?",
            )?;
            let count: i64 =
                stmt.query_row(params![SCHEMA, table_name, column_name], |row| row.get(0))?;
            Ok(count > 0)
        }

        pub fn register_release(
            &self,
            release: &ModelReleaseRecord,
            files: &[ModelReleaseFile],
            parent_release_id: Option<&str>,
            manifest_json: &serde_json::Value,
            extra_metadata: &serde_json::Value,
        ) -> anyhow::Result<ModelReleaseRegistration> {
            if let Some(existing) = self.find_release(&release.release_id)? {
                if existing.package_hash != release.package_hash
                    || existing.dbnum != release.dbnum
                    || existing.project_name != release.project_name
                    || existing.branch_id != release.branch_id
                {
                    anyhow::bail!(
                        "release_id '{}' already exists with different content",
                        release.release_id
                    );
                }
                let existing_parent = self.find_parent_release_id(&release.release_id)?;
                if let Some(request_parent) = parent_release_id
                    && existing_parent.as_deref() != Some(request_parent)
                {
                    anyhow::bail!(
                        "release_id '{}' already exists with parent {:?}, not '{}'",
                        release.release_id,
                        existing_parent,
                        request_parent
                    );
                }
                self.update_release_provenance_if_missing(release)?;
                let existing = self.find_release(&release.release_id)?.with_context(|| {
                    format!("model release '{}' disappeared", release.release_id)
                })?;
                let existing_files = self.list_release_files(&release.release_id)?;
                return Ok(ModelReleaseRegistration {
                    status: ModelReleaseRegistrationStatus::AlreadyExists,
                    release: existing,
                    files: existing_files,
                    parent_release_id: existing_parent,
                    ducklake_metadata_path: self.cfg.metadata_path.clone(),
                    ducklake_data_path: self.cfg.data_path.clone(),
                    component_index: None,
                });
            }

            self.conn
                .execute_batch("BEGIN TRANSACTION")
                .context("begin model release registration transaction")?;
            let tx_result = self.insert_release(
                release,
                files,
                parent_release_id,
                manifest_json,
                extra_metadata,
            );
            match tx_result {
                Ok(()) => {
                    self.conn
                        .execute_batch("COMMIT")
                        .context("commit model release registration transaction")?;
                }
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
            }

            Ok(ModelReleaseRegistration {
                status: ModelReleaseRegistrationStatus::Created,
                release: release.clone(),
                files: files.to_vec(),
                parent_release_id: parent_release_id.map(|value| value.to_string()),
                ducklake_metadata_path: self.cfg.metadata_path.clone(),
                ducklake_data_path: self.cfg.data_path.clone(),
                component_index: None,
            })
        }

        pub fn list_releases(
            &self,
            project_name: Option<&str>,
        ) -> anyhow::Result<ModelReleaseListResponse> {
            let sql_all = format!(
                "SELECT release_id, project_name, branch_id, \
                 COALESCE(release_status, 'published') AS release_status, \
                 release_label, dbnum, \
                 source_package_dir, immutable_package_dir, package_hash, derivation_type, \
                 created_at, registered_at, rows_instances, rows_geo_instances, rows_transforms, \
                 rows_aabb, rows_tubings, rows_ptsets, rows_primitive_keypoints, \
                 source_manifest_path, source_manifest_hash, baseline_state_manifest_path, \
                 baseline_state_manifest_hash, generation_job_id, asset_manifest_path, \
                 asset_manifest_hash, release_lifecycle, release_quality, release_quality_reason, \
                 validation_flags_json, spec_info_fallback_count \
                 FROM \"{}\".\"model_releases\" \
                 WHERE release_lifecycle = 'published' \
                 ORDER BY registered_at DESC, release_id DESC",
                SCHEMA
            );
            let sql_project = format!(
                "SELECT release_id, project_name, branch_id, \
                 COALESCE(release_status, 'published') AS release_status, \
                 release_label, dbnum, \
                 source_package_dir, immutable_package_dir, package_hash, derivation_type, \
                 created_at, registered_at, rows_instances, rows_geo_instances, rows_transforms, \
                 rows_aabb, rows_tubings, rows_ptsets, rows_primitive_keypoints, \
                 source_manifest_path, source_manifest_hash, baseline_state_manifest_path, \
                 baseline_state_manifest_hash, generation_job_id, asset_manifest_path, \
                 asset_manifest_hash, release_lifecycle, release_quality, release_quality_reason, \
                 validation_flags_json, spec_info_fallback_count \
                 FROM \"{}\".\"model_releases\" WHERE project_name = ? \
                 AND release_lifecycle = 'published' \
                 ORDER BY registered_at DESC, release_id DESC",
                SCHEMA
            );

            let releases = if let Some(project) = project_name {
                let mut stmt = self.conn.prepare(&sql_project)?;
                let rows = stmt.query_map(params![project], row_to_release)?;
                collect_rows(rows)?
            } else {
                let mut stmt = self.conn.prepare(&sql_all)?;
                let rows = stmt.query_map([], row_to_release)?;
                collect_rows(rows)?
            };

            Ok(ModelReleaseListResponse {
                project_name: project_name.map(|value| value.to_string()),
                releases,
            })
        }

        pub fn update_release_status(
            &self,
            release_id: &str,
            status: ModelReleaseStatus,
            reason: Option<&str>,
        ) -> anyhow::Result<()> {
            let updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            self.conn
                .execute_batch("BEGIN TRANSACTION")
                .context("begin model release status transaction")?;
            let result = (|| -> anyhow::Result<()> {
                let sql = format!(
                    "UPDATE \"{}\".\"model_releases\" SET release_status = ?, release_lifecycle = ?, \
                     release_quality = CASE WHEN ? IS NULL THEN release_quality ELSE ? END \
                     WHERE release_id = ?",
                    SCHEMA
                );
                let lifecycle = status.lifecycle();
                let quality = legacy_status_quality(&status).map(|value| value.as_str());
                let updated = self
                    .conn
                    .execute(
                        &sql,
                        params![
                            status.as_str(),
                            lifecycle.as_str(),
                            quality,
                            quality,
                            release_id
                        ],
                    )
                    .context("update model release status")?;
                if updated == 0 {
                    anyhow::bail!("model release '{}' does not exist", release_id);
                }
                let event_sql = format!(
                    "INSERT INTO \"{}\".\"model_release_status_events\" VALUES (?, ?, ?, ?)",
                    SCHEMA
                );
                self.conn.execute(
                    &event_sql,
                    params![release_id, status.as_str(), reason, updated_at],
                )?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    self.conn
                        .execute_batch("COMMIT")
                        .context("commit model release status transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(err)
                }
            }
        }

        pub fn release_events(
            &self,
            release_id: &str,
        ) -> anyhow::Result<ModelReleaseEventsResponse> {
            let release = self.get_release(release_id)?;
            let events = self.list_release_status_events(release_id)?;
            Ok(ModelReleaseEventsResponse { release, events })
        }

        pub fn reconcile_release(
            &self,
            release_id: &str,
            publish_if_complete: bool,
            fail_if_unusable: bool,
        ) -> anyhow::Result<ModelReleaseReconcileReport> {
            let release = self.get_release(release_id)?;
            let previous_status = release.release_status.clone();
            let previous_lifecycle = release.release_lifecycle.clone();
            let package_dir_exists = release.immutable_package_dir.is_dir();
            let package_manifest_path = release.immutable_package_dir.join("manifest.json");
            let package_manifest_exists = package_manifest_path.is_file();
            let release_sidecar_path = match release_root_dir(&release) {
                Ok(root) => root.join("release.json"),
                Err(error) => release.immutable_package_dir.join(format!(
                    "release.json.invalid-parent.{}",
                    error
                        .to_string()
                        .replace('\\', "_")
                        .replace('/', "_")
                        .replace(':', "_")
                )),
            };
            let release_sidecar_exists = release_sidecar_path.is_file();
            let release_sidecar_hash = if release_sidecar_exists {
                Some(crate::version_management::hashing::sha256_file(
                    &release_sidecar_path,
                )?)
            } else {
                None
            };
            let files = self.list_release_files(release_id)?;
            let missing_required_files = missing_required_release_files(&release, &files);

            let mut problems = Vec::new();
            let mut warnings = Vec::new();
            if !package_dir_exists {
                problems.push(format!(
                    "immutable package directory is missing: {}",
                    release.immutable_package_dir.display()
                ));
            }
            if !package_manifest_exists {
                problems.push(format!(
                    "immutable package manifest is missing: {}",
                    package_manifest_path.display()
                ));
            }
            if release_sidecar_exists {
                validate_release_sidecar(
                    &release,
                    &release_sidecar_path,
                    &mut problems,
                    &mut warnings,
                )?;
            } else {
                problems.push(format!(
                    "release sidecar is missing: {}",
                    release_sidecar_path.display()
                ));
            }
            if files.is_empty() {
                problems.push("release file manifest has no files".to_string());
            }
            validate_release_file_catalog(&release, &files, &mut problems);
            if publish_if_complete && release.release_quality != ModelReleaseQuality::CompleteVisual
            {
                problems.push(format!(
                    "release quality is {}, expected complete_visual for publish_if_complete",
                    release.release_quality.as_str()
                ));
            }
            if release.release_quality == ModelReleaseQuality::CompleteVisual || publish_if_complete
            {
                problems.extend(Self::release_validation_flag_problems(&release));
                problems.sort();
                problems.dedup();
            }

            let component_index = self.latest_component_index_stats(&release)?;
            match &component_index {
                Some(stats) => {
                    let current_count = self.component_snapshot_count(release_id)?;
                    if current_count != stats.component_count {
                        problems.push(format!(
                            "component index is stale: indexed_count={} current_count={}",
                            stats.component_count, current_count
                        ));
                    }
                    if let Some(instance_rows) = release.row_count("instances")
                        && stats.component_count != instance_rows
                    {
                        warnings.push(format!(
                            "component index count {} differs from release instances row count {}",
                            stats.component_count, instance_rows
                        ));
                    }
                }
                None => problems.push(format!(
                    "component index is missing for release '{}'",
                    release.release_id
                )),
            }

            let mesh_asset_index = self.latest_mesh_asset_index_stats(&release)?;
            if release.row_count("geo_instances").unwrap_or_default() > 0 {
                match &mesh_asset_index {
                    Some(stats) => {
                        if stats.missing_count > 0 {
                            problems.push(format!(
                                "mesh asset index has {} missing non-builtin assets",
                                stats.missing_count
                            ));
                        }
                        match stats.glb_unreadable_count {
                            Some(count) if count > 0 => problems.push(format!(
                                "mesh asset index has {count} unreadable GLB assets"
                            )),
                            Some(_) => {}
                            None => problems.push(
                                "mesh asset index lacks GLB readability evidence; rerun index-assets --materialize with this build"
                                    .to_string(),
                            ),
                        }
                        if let Some(checked) = stats.glb_checked_count
                            && checked != stats.present_count
                        {
                            problems.push(format!(
                                "mesh asset readability evidence is incomplete: checked_count={} present_count={}",
                                checked, stats.present_count
                            ));
                        }
                        let violation_count =
                            self.release_local_mesh_asset_violation_count(stats)?;
                        if violation_count > 0 {
                            problems.push(format!(
                                "mesh asset index has {} non release-local or missing asset rows",
                                violation_count
                            ));
                        }
                        if release.asset_manifest_path.is_none()
                            || release.asset_manifest_hash.is_none()
                        {
                            warnings.push(
                                "release has visual geometry but no asset_manifest_hash evidence"
                                    .to_string(),
                            );
                        }
                    }
                    None => problems.push(format!(
                        "mesh asset index is missing for visual release '{}'",
                        release.release_id
                    )),
                }
            } else if mesh_asset_index.is_none() {
                warnings
                    .push("release has no geo_instances rows and no mesh asset index".to_string());
            }

            let unit_index = self.latest_unit_index_stats(&release)?;
            if unit_index.is_none() {
                warnings.push(
                    "delivery-unit index is missing; unit diff/impact APIs will require index-units"
                        .to_string(),
                );
            }

            let publishable = problems.is_empty();
            let mut applied = false;
            let mut action_taken = "none".to_string();
            let recommended_action;

            if publishable {
                if release.release_lifecycle == ModelReleaseLifecycle::Published {
                    recommended_action =
                        "release is already published and reconcile evidence is consistent"
                            .to_string();
                } else if publish_if_complete {
                    self.update_release_status(
                        release_id,
                        ModelReleaseStatus::Published,
                        Some("reconciled complete release evidence"),
                    )?;
                    applied = true;
                    action_taken = "published".to_string();
                    recommended_action =
                        "release was marked published after reconcile evidence passed".to_string();
                } else {
                    recommended_action =
                        "release evidence is complete; rerun reconcile with publish_if_complete to mark it published"
                            .to_string();
                }
            } else if fail_if_unusable && release.release_lifecycle != ModelReleaseLifecycle::Failed
            {
                self.update_release_status(
                    release_id,
                    ModelReleaseStatus::Failed,
                    Some(&format!("reconcile failed: {}", problems.join("; "))),
                )?;
                applied = true;
                action_taken = "failed".to_string();
                recommended_action =
                    "release was marked failed because reconcile found blocking evidence problems"
                        .to_string();
            } else {
                recommended_action =
                    "release has blocking evidence problems; repair/index missing evidence or rerun reconcile with fail_if_unusable to mark it failed"
                        .to_string();
            }

            let release = self.get_release(release_id)?;
            let events = self.list_release_status_events(release_id)?;
            Ok(ModelReleaseReconcileReport {
                current_status: release.release_status.clone(),
                current_lifecycle: release.release_lifecycle.clone(),
                release,
                previous_status,
                previous_lifecycle,
                publishable,
                applied,
                action_taken,
                recommended_action,
                package_dir_exists,
                package_manifest_exists,
                release_sidecar_path,
                release_sidecar_exists,
                release_sidecar_hash,
                missing_required_files,
                problems,
                warnings,
                component_index,
                mesh_asset_index,
                unit_index,
                events,
            })
        }

        fn update_release_asset_manifest(
            &self,
            stats: &ModelReleaseMeshAssetIndexStats,
        ) -> anyhow::Result<()> {
            let manifest_hash =
                crate::version_management::hashing::sha256_file(&stats.manifest_path)
                    .with_context(|| {
                        format!(
                            "hash mesh asset manifest failed: {}",
                            stats.manifest_path.display()
                        )
                    })?;
            let sql = format!(
                "UPDATE \"{}\".\"model_releases\" \
                 SET asset_manifest_path = ?, asset_manifest_hash = ? \
                 WHERE release_id = ?",
                SCHEMA
            );
            let updated = self
                .conn
                .execute(
                    &sql,
                    params![
                        stats.manifest_path.to_string_lossy().to_string(),
                        manifest_hash,
                        stats.release_id
                    ],
                )
                .context("update release asset manifest evidence")?;
            if updated == 0 {
                anyhow::bail!(
                    "model release '{}' does not exist while updating asset manifest evidence",
                    stats.release_id
                );
            }
            Ok(())
        }

        pub fn repair_release_source_manifest_to_package(
            &self,
            release_id: &str,
        ) -> anyhow::Result<Option<ModelReleaseRecord>> {
            let release = self.get_release(release_id)?;
            let manifest_path = release.immutable_package_dir.join("manifest.json");
            ensure_file_exists(&manifest_path, "release package manifest")?;
            let manifest_hash = crate::version_management::hashing::sha256_file(&manifest_path)
                .with_context(|| {
                    format!(
                        "hash release package manifest failed: {}",
                        manifest_path.display()
                    )
                })?;
            let path_current = release
                .source_manifest_path
                .as_ref()
                .is_some_and(|path| path_is_equal(path, &manifest_path));
            let hash_current = release.source_manifest_hash.as_deref() == Some(&manifest_hash);
            if path_current && hash_current {
                return Ok(None);
            }

            let sql = format!(
                "UPDATE \"{}\".\"model_releases\" \
                 SET source_manifest_path = ?, source_manifest_hash = ? \
                 WHERE release_id = ?",
                SCHEMA
            );
            let updated = self
                .conn
                .execute(
                    &sql,
                    params![
                        manifest_path.to_string_lossy().to_string(),
                        manifest_hash,
                        release_id
                    ],
                )
                .context("repair release source manifest evidence")?;
            if updated == 0 {
                anyhow::bail!(
                    "release '{}' disappeared while repairing source manifest evidence",
                    release_id
                );
            }
            Ok(Some(self.get_release(release_id)?))
        }

        fn update_release_provenance_if_missing(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<()> {
            let sql = format!(
                "UPDATE \"{}\".\"model_releases\" SET \
                 source_manifest_path = CASE WHEN source_manifest_path IS NULL OR source_manifest_path = '' THEN ? ELSE source_manifest_path END, \
                 source_manifest_hash = CASE WHEN source_manifest_hash IS NULL OR source_manifest_hash = '' THEN ? ELSE source_manifest_hash END, \
                 baseline_state_manifest_path = CASE WHEN baseline_state_manifest_path IS NULL OR baseline_state_manifest_path = '' THEN ? ELSE baseline_state_manifest_path END, \
                 baseline_state_manifest_hash = CASE WHEN baseline_state_manifest_hash IS NULL OR baseline_state_manifest_hash = '' THEN ? ELSE baseline_state_manifest_hash END, \
                 generation_job_id = CASE WHEN generation_job_id IS NULL OR generation_job_id = '' THEN ? ELSE generation_job_id END, \
                 release_quality_reason = CASE WHEN release_quality_reason IS NULL OR release_quality_reason = '' THEN ? ELSE release_quality_reason END, \
                 validation_flags_json = CASE WHEN validation_flags_json IS NULL OR validation_flags_json = '' OR validation_flags_json = '[]' THEN ? ELSE validation_flags_json END, \
                 spec_info_fallback_count = CASE WHEN spec_info_fallback_count IS NULL THEN ? ELSE spec_info_fallback_count END \
                 WHERE release_id = ?",
                SCHEMA
            );
            let validation_flags_json = serde_json::to_string(&release.validation_flags)?;
            let spec_info_fallback_count = release
                .spec_info_fallback_count
                .map(|value| u64_to_i64(value, "spec_info_fallback_count"))
                .transpose()?;
            self.conn
                .execute(
                    &sql,
                    params![
                        release
                            .source_manifest_path
                            .as_ref()
                            .map(|path| path.to_string_lossy().to_string()),
                        release.source_manifest_hash,
                        release
                            .baseline_state_manifest_path
                            .as_ref()
                            .map(|path| path.to_string_lossy().to_string()),
                        release.baseline_state_manifest_hash,
                        release.generation_job_id,
                        release.release_quality_reason,
                        validation_flags_json,
                        spec_info_fallback_count,
                        release.release_id
                    ],
                )
                .context("backfill missing release provenance evidence")?;
            Ok(())
        }

        pub fn annotate_release_quality(
            &self,
            release_id: &str,
            release_quality: Option<ModelReleaseQuality>,
            release_quality_reason: Option<&str>,
            validation_flags: &[String],
            spec_info_fallback_count: Option<u64>,
        ) -> anyhow::Result<ModelReleaseRecord> {
            if release_quality.is_none()
                && release_quality_reason.is_none()
                && validation_flags.is_empty()
                && spec_info_fallback_count.is_none()
            {
                anyhow::bail!(
                    "at least one release quality annotation field is required for '{}'",
                    release_id
                );
            }

            let current = self.get_release(release_id)?;
            let quality = release_quality.unwrap_or_else(|| current.release_quality.clone());
            let reason = release_quality_reason
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| current.release_quality_reason.clone());
            let mut merged_flags = current.validation_flags.clone();
            for flag in validation_flags {
                let flag = flag.trim();
                if flag.is_empty() {
                    continue;
                }
                if !merged_flags.iter().any(|existing| existing == flag) {
                    merged_flags.push(flag.to_string());
                }
            }
            if let Some(count) = spec_info_fallback_count {
                if count == 0 {
                    merged_flags.retain(|flag| {
                        !matches!(
                            flag.trim().to_ascii_lowercase().as_str(),
                            "spec_info_fallback" | "spec_info_fallback_unquantified"
                        )
                    });
                } else {
                    merged_flags.retain(|flag| {
                        flag.trim().to_ascii_lowercase() != "spec_info_fallback_unquantified"
                    });
                    if !merged_flags
                        .iter()
                        .any(|flag| flag.trim().eq_ignore_ascii_case("spec_info_fallback"))
                    {
                        merged_flags.push("spec_info_fallback".to_string());
                    }
                }
            }
            let flags_json = serde_json::to_string(&merged_flags)
                .context("serialize release validation flags")?;
            let spec_count = spec_info_fallback_count
                .or(current.spec_info_fallback_count)
                .map(|value| u64_to_i64(value, "spec_info_fallback_count"))
                .transpose()?;
            let sql = format!(
                "UPDATE \"{}\".\"model_releases\" SET \
                 release_quality = ?, \
                 release_quality_reason = ?, \
                 validation_flags_json = ?, \
                 spec_info_fallback_count = ? \
                 WHERE release_id = ?",
                SCHEMA
            );
            let updated = self
                .conn
                .execute(
                    &sql,
                    params![quality.as_str(), reason, flags_json, spec_count, release_id],
                )
                .with_context(|| format!("annotate model release '{}'", release_id))?;
            if updated == 0 {
                anyhow::bail!("model release '{}' does not exist", release_id);
            }
            self.get_release(release_id)
        }

        pub fn get_release(&self, release_id: &str) -> anyhow::Result<ModelReleaseRecord> {
            self.find_release(release_id)?
                .with_context(|| format!("model release '{}' does not exist", release_id))
        }

        pub fn index_release_components(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelComponentSnapshotStats> {
            let instances_path = release.immutable_package_dir.join("instances.parquet");
            let geo_instances_path = release.immutable_package_dir.join("geo_instances.parquet");
            ensure_file_exists(&instances_path, "instances.parquet")?;
            ensure_file_exists(&geo_instances_path, "geo_instances.parquet")?;

            let indexed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let release_id = escape_sql_string(&release.release_id);
            let project_name = escape_sql_string(&release.project_name);
            let instances = escape_sql_string(&duckdb_path(&instances_path));
            let geo_instances = escape_sql_string(&duckdb_path(&geo_instances_path));
            let hash_version = escape_sql_string(COMPONENT_HASH_VERSION);
            let insert_sql = format!(
                r#"
DELETE FROM "{schema}"."component_snapshots" WHERE release_id = '{release_id}';

WITH geos AS (
    SELECT
        refno_u64,
        string_agg(
            concat(
                CAST(geo_index AS VARCHAR),
                ':',
                geo_hash,
                ':',
                geo_trans_hash
            ),
            '|' ORDER BY geo_index, geo_hash, geo_trans_hash
        ) AS geo_signature
    FROM read_parquet('{geo_instances}')
    GROUP BY refno_u64
)
INSERT INTO "{schema}"."component_snapshots"
SELECT
    '{release_id}' AS release_id,
    '{project_name}' AS project_name,
    TRY_CAST(i.dbnum AS INTEGER) AS dbnum,
    concat(CAST(i.dbnum AS VARCHAR), ':', CAST(i.refno_u64 AS VARCHAR)) AS component_key,
    i.refno_str,
    TRY_CAST(i.refno_u64 AS BIGINT) AS refno_u64,
    i.noun,
    i.owner_refno_str,
    TRY_CAST(i.owner_refno_u64 AS BIGINT) AS owner_refno_u64,
    i.owner_noun,
    i.cata_hash,
    i.trans_hash,
    i.aabb_hash,
    TRY_CAST(i.spec_value AS BIGINT) AS spec_value,
    i.has_neg,
    COALESCE(g.geo_signature, '') AS geo_signature,
    sha256(concat(
        '{hash_version}', '|',
        CAST(i.dbnum AS VARCHAR), '|',
        i.refno_str, '|',
        CAST(i.refno_u64 AS VARCHAR), '|',
        i.noun, '|',
        COALESCE(i.owner_refno_str, ''), '|',
        COALESCE(CAST(i.owner_refno_u64 AS VARCHAR), ''), '|',
        i.owner_noun, '|',
        COALESCE(i.cata_hash, ''), '|',
        i.trans_hash, '|',
        i.aabb_hash, '|',
        CAST(i.spec_value AS VARCHAR), '|',
        CAST(i.has_neg AS VARCHAR), '|',
        COALESCE(g.geo_signature, '')
    )) AS component_hash,
    '{hash_version}' AS hash_version,
    '{indexed_at}' AS indexed_at
FROM read_parquet('{instances}') i
LEFT JOIN geos g ON i.refno_u64 = g.refno_u64;
"#,
                schema = SCHEMA,
                release_id = release_id,
                project_name = project_name,
                geo_instances = geo_instances,
                instances = instances,
                hash_version = hash_version,
                indexed_at = escape_sql_string(&indexed_at),
            );

            self.conn
                .execute_batch("BEGIN TRANSACTION")
                .context("begin component snapshot indexing transaction")?;
            let tx_result = self
                .conn
                .execute_batch(&insert_sql)
                .context("index component snapshots from release Parquet")
                .and_then(|_| self.insert_component_index_run(release, &indexed_at));
            match tx_result {
                Ok(()) => {
                    self.conn
                        .execute_batch("COMMIT")
                        .context("commit component snapshot indexing transaction")?;
                }
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
            }

            self.latest_component_index_stats(release)?
                .with_context(|| {
                    format!("component index stats missing for {}", release.release_id)
                })
        }

        pub fn ensure_release_components_indexed(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelComponentSnapshotStats> {
            if let Some(stats) = self.latest_component_index_stats(release)? {
                let current_count = self.component_snapshot_count(&release.release_id)?;
                if current_count == stats.component_count {
                    return Ok(stats);
                }
            }
            self.index_release_components(release)
        }

        fn require_release_components_indexed(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelComponentSnapshotStats> {
            let Some(stats) = self.latest_component_index_stats(release)? else {
                anyhow::bail!(
                    "missing dependency: component index is missing for release '{}'; run `aios-database model-version index --release-id {}` or POST /api/model-version/releases/{}/index",
                    release.release_id,
                    release.release_id,
                    release.release_id
                );
            };
            let current_count = self.component_snapshot_count(&release.release_id)?;
            if current_count != stats.component_count {
                anyhow::bail!(
                    "missing dependency: component index is stale for release '{}'; indexed_count={} current_count={}. Run `aios-database model-version index --release-id {}` or POST /api/model-version/releases/{}/index",
                    release.release_id,
                    stats.component_count,
                    current_count,
                    release.release_id,
                    release.release_id
                );
            }
            Ok(stats)
        }

        pub fn diff_releases(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            limit: usize,
            change_type_filter: Option<&str>,
        ) -> anyhow::Result<ModelComponentDiffResponse> {
            let from_release = self.get_release(from_release_id)?;
            let to_release = self.get_release(to_release_id)?;
            if from_release.project_name != to_release.project_name {
                anyhow::bail!(
                    "cannot diff releases from different projects: '{}' vs '{}'",
                    from_release.project_name,
                    to_release.project_name
                );
            }
            if from_release.dbnum != to_release.dbnum {
                anyhow::bail!(
                    "cannot diff releases from different dbnums: {} vs {}",
                    from_release.dbnum,
                    to_release.dbnum
                );
            }
            require_release_published(&from_release)?;
            require_release_published(&to_release)?;

            let from_index = self.require_release_components_indexed(&from_release)?;
            let to_index = self.require_release_components_indexed(&to_release)?;
            let summary = self.component_diff_summary(from_release_id, to_release_id)?;
            let rows = self.component_diff_rows(
                from_release_id,
                to_release_id,
                limit,
                change_type_filter,
            )?;

            Ok(ModelComponentDiffResponse {
                from_release_id: from_release_id.to_string(),
                to_release_id: to_release_id.to_string(),
                project_name: from_release.project_name,
                dbnum: from_release.dbnum,
                from_index,
                to_index,
                summary: ModelComponentDiffSummary {
                    emitted: rows.len(),
                    ..summary
                },
                rows,
            })
        }

        pub fn compare_readiness(
            &self,
            from_release_id: &str,
            to_release_id: &str,
        ) -> anyhow::Result<ModelReleasePairReadinessResponse> {
            let from_release = self.find_release(from_release_id)?;
            let to_release = self.find_release(to_release_id)?;
            let from = self.release_readiness(from_release_id, from_release)?;
            let to = self.release_readiness(to_release_id, to_release)?;

            let mut problems = Vec::new();
            let mut warnings = Vec::new();
            let both_releases_exist = from.exists && to.exists;
            let same_project = match (from.release.as_ref(), to.release.as_ref()) {
                (Some(from_release), Some(to_release)) => {
                    from_release.project_name == to_release.project_name
                }
                _ => false,
            };
            let same_dbnum = match (from.release.as_ref(), to.release.as_ref()) {
                (Some(from_release), Some(to_release)) => from_release.dbnum == to_release.dbnum,
                _ => false,
            };

            if !both_releases_exist {
                problems.push("one or both releases are missing".to_string());
            }
            if both_releases_exist && !same_project {
                problems.push("releases belong to different projects".to_string());
            }
            if both_releases_exist && !same_dbnum {
                problems.push("releases belong to different dbnums".to_string());
            }

            let both_published = from.published && to.published;
            let both_complete_visual = from.complete_visual && to.complete_visual;
            let component_indexes_ready = from.component_index_ready && to.component_index_ready;
            let mesh_assets_ready = from.mesh_assets_ready && to.mesh_assets_ready;

            if both_releases_exist && !both_published {
                problems.push("one or both releases are not published".to_string());
            }
            if both_releases_exist && !both_complete_visual {
                problems.push("one or both releases are not complete_visual".to_string());
            }
            if both_releases_exist && !component_indexes_ready {
                problems.push(
                    "one or both releases have missing or stale component indexes".to_string(),
                );
            }
            if both_releases_exist && !mesh_assets_ready {
                problems.push("one or both releases have missing, unreadable, or non release-local mesh assets".to_string());
            }

            let diff_summary =
                if both_releases_exist && same_project && same_dbnum && component_indexes_ready {
                    Some(self.component_diff_summary(from_release_id, to_release_id)?)
                } else {
                    None
                };
            if diff_summary.is_none() && both_releases_exist && same_project && same_dbnum {
                warnings.push(
                    "component diff summary is unavailable until component indexes are ready"
                        .to_string(),
                );
            }

            let production_ready = both_releases_exist
                && same_project
                && same_dbnum
                && both_published
                && both_complete_visual
                && component_indexes_ready
                && mesh_assets_ready
                && from.problems.is_empty()
                && to.problems.is_empty()
                && problems.is_empty();
            let any_quarantined =
                matches!(from.quality, Some(ModelReleaseQuality::QuarantinedVisual))
                    || matches!(to.quality, Some(ModelReleaseQuality::QuarantinedVisual));
            let classification = if production_ready {
                "production_ready"
            } else if !both_releases_exist {
                "missing_release"
            } else if !component_indexes_ready {
                "incomplete_indexes"
            } else if any_quarantined {
                "quarantined_visual"
            } else {
                "not_production_ready"
            }
            .to_string();
            let recommended_action = match classification.as_str() {
                "production_ready" => {
                    "release pair is production-ready for visual comparison".to_string()
                }
                "missing_release" => {
                    "register or publish the missing release before comparing".to_string()
                }
                "incomplete_indexes" => {
                    "run model-version index for both releases, then rerun readiness".to_string()
                }
                "quarantined_visual" => {
                    "comparison may be used for diagnosis/demo, but production sign-off requires complete_visual releases with resolved quarantine evidence"
                        .to_string()
                }
                _ => {
                    "resolve release lifecycle, quality, baseline, and asset evidence before production comparison"
                        .to_string()
                }
            };

            Ok(ModelReleasePairReadinessResponse {
                from_release_id: from_release_id.to_string(),
                to_release_id: to_release_id.to_string(),
                project_name: from
                    .release
                    .as_ref()
                    .or(to.release.as_ref())
                    .map(|release| release.project_name.clone()),
                dbnum: from
                    .release
                    .as_ref()
                    .or(to.release.as_ref())
                    .map(|release| release.dbnum),
                classification,
                production_ready,
                production_comparison_allowed: production_ready,
                both_releases_exist,
                same_project,
                same_dbnum,
                both_published,
                both_complete_visual,
                component_indexes_ready,
                mesh_assets_ready,
                diff_summary,
                from,
                to,
                problems,
                warnings,
                recommended_action,
            })
        }

        fn release_readiness(
            &self,
            release_id: &str,
            release: Option<ModelReleaseRecord>,
        ) -> anyhow::Result<ModelReleaseReadinessEvidence> {
            let Some(release) = release else {
                return Ok(ModelReleaseReadinessEvidence {
                    release_id: release_id.to_string(),
                    exists: false,
                    release: None,
                    lifecycle: None,
                    quality: None,
                    validation_flags: Vec::new(),
                    baseline_state_manifest_path: None,
                    baseline_state_manifest_hash: None,
                    spec_info_manifest_evidence_present: false,
                    spec_info_manifest_fallback_count: None,
                    published: false,
                    complete_visual: false,
                    component_index_ready: false,
                    component_index: None,
                    component_index_current_count: None,
                    mesh_assets_ready: false,
                    mesh_asset_index: None,
                    release_local_asset_violation_count: None,
                    unit_index: None,
                    problems: vec![format!("model release '{release_id}' does not exist")],
                    warnings: Vec::new(),
                    recommended_action: "register or publish this release before comparing"
                        .to_string(),
                });
            };

            let mut problems = Vec::new();
            let mut warnings = Vec::new();
            let published = release.release_lifecycle == ModelReleaseLifecycle::Published;
            if !published {
                problems.push(format!(
                    "release lifecycle is {}, expected published",
                    release.release_lifecycle.as_str()
                ));
            }
            let complete_visual = release.release_quality == ModelReleaseQuality::CompleteVisual;
            if !complete_visual {
                warnings.push(format!(
                    "release quality is {}, not complete_visual",
                    release.release_quality.as_str()
                ));
            }
            if release.baseline_state_manifest_path.is_none()
                || release.baseline_state_manifest_hash.is_none()
            {
                let message = "release has no baseline state manifest evidence".to_string();
                if complete_visual {
                    problems.push(message);
                } else {
                    warnings.push(message);
                }
            }
            let (spec_info_manifest_evidence_present, spec_info_manifest_fallback_count) =
                match Self::release_spec_info_manifest_evidence(&release) {
                    Ok(evidence) => evidence,
                    Err(error) => {
                        let message =
                            format!("release spec_info manifest evidence cannot be read: {error}");
                        if complete_visual {
                            problems.push(message);
                        } else {
                            warnings.push(message);
                        }
                        (false, None)
                    }
                };
            if !spec_info_manifest_evidence_present {
                let message = "release package manifest lacks generated spec_info fallback evidence; rerun parquet export/register or audit legacy spec rows before complete_visual production comparison"
                    .to_string();
                if complete_visual {
                    problems.push(message);
                } else {
                    warnings.push(message);
                }
            }
            match release_root_dir(&release) {
                Ok(root) => {
                    let path = root.join("release.json");
                    if path.is_file() {
                        validate_release_sidecar(&release, &path, &mut problems, &mut warnings)?;
                    } else {
                        problems.push(format!("release sidecar is missing: {}", path.display()));
                    }
                }
                Err(error) => {
                    problems.push(format!("release sidecar path cannot be resolved: {error}"));
                }
            }
            for flag_problem in Self::release_validation_flag_problems(&release) {
                problems.push(flag_problem);
            }
            let release_files = self.list_release_files(&release.release_id)?;
            if release_files.is_empty() {
                problems.push("release file manifest has no files".to_string());
            }
            validate_release_file_catalog(&release, &release_files, &mut problems);

            let component_index = self.latest_component_index_stats(&release)?;
            let component_index_current_count = if component_index.is_some() {
                Some(self.component_snapshot_count(&release.release_id)?)
            } else {
                None
            };
            let component_index_ready = match (&component_index, component_index_current_count) {
                (Some(stats), Some(current_count)) if stats.component_count == current_count => {
                    true
                }
                (Some(stats), Some(current_count)) => {
                    problems.push(format!(
                        "component index is stale: indexed_count={} current_count={}",
                        stats.component_count, current_count
                    ));
                    false
                }
                _ => {
                    problems.push("component index is missing".to_string());
                    false
                }
            };

            let mesh_asset_index = self.latest_mesh_asset_index_stats(&release)?;
            let release_local_asset_violation_count = match &mesh_asset_index {
                Some(stats) => Some(self.release_local_mesh_asset_violation_count(stats)?),
                None => None,
            };
            let visual_geometry_rows = release.row_count("geo_instances").unwrap_or_default();
            let mesh_assets_ready = if visual_geometry_rows == 0 {
                warnings.push("release has no geo_instances rows".to_string());
                false
            } else {
                match (&mesh_asset_index, release_local_asset_violation_count) {
                    (Some(stats), Some(violations)) => {
                        let unreadable = stats.glb_unreadable_count.unwrap_or(u64::MAX);
                        let checked = stats.glb_checked_count.unwrap_or(0);
                        let readability_complete = checked == stats.present_count;
                        if stats.missing_count > 0 {
                            problems.push(format!(
                                "mesh asset index has {} missing non-builtin assets",
                                stats.missing_count
                            ));
                        }
                        if stats.glb_unreadable_count.is_none() {
                            problems.push(
                                "mesh asset index lacks GLB readability evidence".to_string(),
                            );
                        } else if unreadable > 0 {
                            problems.push(format!(
                                "mesh asset index has {unreadable} unreadable GLB assets"
                            ));
                        }
                        if !readability_complete {
                            problems.push(format!(
                                "mesh asset readability evidence is incomplete: checked_count={} present_count={}",
                                checked, stats.present_count
                            ));
                        }
                        if violations > 0 {
                            problems.push(format!(
                                "mesh asset index has {violations} non release-local or missing asset rows"
                            ));
                        }
                        stats.missing_count == 0
                            && stats.glb_unreadable_count == Some(0)
                            && readability_complete
                            && violations == 0
                    }
                    _ => {
                        problems.push("mesh asset index is missing for visual release".to_string());
                        false
                    }
                }
            };

            let unit_index = self.latest_unit_index_stats(&release)?;
            if unit_index.is_none() {
                warnings.push(
                    "delivery-unit index is missing; unit diff/impact APIs may be unavailable"
                        .to_string(),
                );
            }

            let recommended_action = if problems.is_empty() && complete_visual {
                "release evidence is production-ready".to_string()
            } else if !component_index_ready {
                format!(
                    "run `aios-database model-version index --release-id {}`",
                    release.release_id
                )
            } else if !mesh_assets_ready {
                format!(
                    "run `aios-database model-version index-assets --release-id {} --materialize` and repair unreadable/missing GLBs",
                    release.release_id
                )
            } else if !complete_visual {
                "resolve quarantine/degraded evidence and annotate release as complete_visual only after full validation"
                    .to_string()
            } else {
                "resolve listed release readiness problems".to_string()
            };

            Ok(ModelReleaseReadinessEvidence {
                release_id: release.release_id.clone(),
                exists: true,
                lifecycle: Some(release.release_lifecycle.clone()),
                quality: Some(release.release_quality.clone()),
                validation_flags: release.validation_flags.clone(),
                baseline_state_manifest_path: release.baseline_state_manifest_path.clone(),
                baseline_state_manifest_hash: release.baseline_state_manifest_hash.clone(),
                spec_info_manifest_evidence_present,
                spec_info_manifest_fallback_count,
                published,
                complete_visual,
                component_index_ready,
                component_index,
                component_index_current_count,
                mesh_assets_ready,
                mesh_asset_index,
                release_local_asset_violation_count,
                unit_index,
                problems,
                warnings,
                recommended_action,
                release: Some(release),
            })
        }

        pub(crate) fn release_validation_flag_problems(
            release: &ModelReleaseRecord,
        ) -> Vec<String> {
            let mut problems = Vec::new();
            for flag in &release.validation_flags {
                match flag.trim().to_ascii_lowercase().as_str() {
                    "mesh_missing_rows_quarantined" => {
                        problems.push(
                            "release has quarantined missing mesh rows; register a repaired visual package before complete_visual production comparison"
                                .to_string(),
                        );
                    }
                    "spec_info_fallback" => match release.spec_info_fallback_count {
                        Some(count) => problems.push(format!(
                            "release has {count} spec_info fallback rows; regenerate spec_info before complete_visual production comparison"
                        )),
                        None => problems.push(
                            "release has unquantified spec_info fallback risk; quantify or regenerate before complete_visual production comparison"
                                .to_string(),
                        ),
                    },
                    "spec_info_fallback_unquantified" => {
                        problems.push(
                            "release has unquantified spec_info fallback risk; quantify or regenerate before complete_visual production comparison"
                                .to_string(),
                        );
                    }
                    "incremental_handoff_affected_scope" => {
                        problems.push(
                            "release was produced from an affected-scope incremental handoff; hydrate a complete baseline package before complete_visual production comparison"
                                .to_string(),
                        );
                    }
                    "degraded_geometry_fallback" => {
                        problems.push(
                            "release contains degraded fallback geometry; keep it as degraded_visual until reviewed or regenerated from exact source geometry"
                                .to_string(),
                        );
                    }
                    "self_intersecting_input" | "self_intersecting_profile" => {
                        problems.push(
                            "release contains self-intersecting source profiles; repair the source geometry or keep it quarantined with explicit visual-contract evidence"
                                .to_string(),
                        );
                    }
                    "non_renderable_input" => {
                        problems.push(
                            "release contains non-renderable source geometry; repair upstream geometry or keep it quarantined with explicit visual-contract evidence"
                                .to_string(),
                        );
                    }
                    "missing_inst_geo" => {
                        problems.push(
                            "release is missing inst_geo records required for mesh generation; refresh generation inputs or keep it quarantined"
                                .to_string(),
                        );
                    }
                    flag if flag.starts_with("tree_index_missing") => {
                        problems.push(format!(
                            "release validation flag '{}' indicates missing tree index evidence",
                            flag
                        ));
                    }
                    _ => {}
                }
            }
            problems.sort();
            problems.dedup();
            problems
        }

        fn release_spec_info_manifest_evidence(
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<(bool, Option<u64>)> {
            let manifest = read_release_manifest_json(release)?;
            let fallback_count = json_u64_at(&manifest, &["spec_info_fallback_count"])
                .or_else(|| json_u64_at(&manifest, &["spec_info_validation", "fallback_count"]));
            let evidence_present = fallback_count.is_some()
                || manifest
                    .get("spec_info_validation")
                    .map(|value| !value.is_null())
                    .unwrap_or(false);
            Ok((evidence_present, fallback_count))
        }

        pub fn index_release_units(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelUnitIndexStats> {
            self.ensure_release_components_indexed(release)?;

            let indexed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let release_id = escape_sql_string(&release.release_id);
            let membership_hash_version = escape_sql_string(MEMBERSHIP_HASH_VERSION);
            let unit_hash_version = escape_sql_string(UNIT_HASH_VERSION);
            let rule_set_hash = escape_sql_string(UNIT_RULE_SET_HASH);
            let indexed_at_sql = escape_sql_string(&indexed_at);
            let sql = format!(
                r#"
DELETE FROM "{schema}"."delivery_unit_memberships" WHERE release_id = '{release_id}';
DELETE FROM "{schema}"."unit_versions" WHERE release_id = '{release_id}';

WITH base AS (
    SELECT
        release_id,
        project_name,
        dbnum,
        component_key,
        refno_str AS component_refno_str,
        refno_u64 AS component_refno_u64,
        CASE
            WHEN UPPER(COALESCE(noun, '')) = 'EQUIP' THEN 'EQUI'
            ELSE UPPER(COALESCE(noun, ''))
        END AS component_noun,
        component_hash,
        owner_refno_str,
        owner_refno_u64,
        CASE
            WHEN UPPER(COALESCE(owner_noun, '')) = 'EQUIP' THEN 'EQUI'
            ELSE UPPER(COALESCE(owner_noun, ''))
        END AS owner_noun
    FROM "{schema}"."component_snapshots"
    WHERE release_id = '{release_id}'
),
resolved AS (
    SELECT
        *,
        CASE
            WHEN component_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') THEN component_noun
            WHEN owner_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') AND owner_refno_u64 IS NOT NULL THEN owner_noun
            ELSE 'UNASSIGNED'
        END AS unit_noun,
        CASE
            WHEN component_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') THEN component_refno_str
            WHEN owner_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') AND owner_refno_u64 IS NOT NULL THEN owner_refno_str
            ELSE NULL
        END AS unit_refno_str,
        CASE
            WHEN component_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') THEN component_refno_u64
            WHEN owner_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') AND owner_refno_u64 IS NOT NULL THEN owner_refno_u64
            ELSE NULL
        END AS unit_refno_u64,
        CASE
            WHEN component_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') THEN 'self_unit'
            WHEN owner_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') AND owner_refno_u64 IS NOT NULL THEN 'direct_owner'
            ELSE 'unassigned'
        END AS membership_kind,
        CASE
            WHEN component_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') THEN 1.0
            WHEN owner_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') AND owner_refno_u64 IS NOT NULL THEN 0.8
            ELSE 0.0
        END AS path_confidence,
        CASE
            WHEN component_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') THEN NULL
            WHEN owner_noun IN ('BRAN', 'HANG', 'EQUI', 'WALL', 'FLOOR') AND owner_refno_u64 IS NOT NULL THEN NULL
            WHEN owner_refno_u64 IS NULL THEN 'missing_owner_refno'
            WHEN owner_noun IS NULL OR owner_noun = '' THEN 'missing_owner_noun'
            ELSE concat('owner_not_delivery_unit:', owner_noun)
        END AS unresolved_reason
    FROM base
),
unitized AS (
    SELECT
        *,
        CASE
            WHEN unit_noun = 'UNASSIGNED' THEN concat(project_name, '|', CAST(dbnum AS VARCHAR), '|UNASSIGNED')
            ELSE concat(project_name, '|', CAST(dbnum AS VARCHAR), '|', unit_noun, '|', CAST(unit_refno_u64 AS VARCHAR))
        END AS unit_key
    FROM resolved
)
INSERT INTO "{schema}"."delivery_unit_memberships"
SELECT
    release_id,
    project_name,
    dbnum,
    unit_key,
    unit_noun,
    unit_refno_str,
    unit_refno_u64,
    component_key,
    component_refno_str,
    component_refno_u64,
    component_noun,
    component_hash,
    owner_refno_str,
    owner_refno_u64,
    owner_noun,
    membership_kind,
    path_confidence,
    unresolved_reason,
    sha256(concat(
        '{membership_hash_version}', '|',
        component_key, '|',
        unit_key, '|',
        membership_kind, '|',
        COALESCE(component_hash, '')
    )) AS membership_hash,
    '{membership_hash_version}' AS hash_version,
    '{indexed_at}' AS indexed_at
FROM unitized;

WITH grouped AS (
    SELECT
        release_id,
        project_name,
        dbnum,
        unit_key,
        unit_noun,
        any_value(unit_refno_str) AS unit_refno_str,
        any_value(unit_refno_u64) AS unit_refno_u64,
        COUNT(*) AS member_count,
        COALESCE(SUM(CASE WHEN unresolved_reason IS NULL THEN 0 ELSE 1 END), 0) AS unresolved_member_count,
        string_agg(
            concat(
                component_key,
                '=',
                COALESCE(component_hash, ''),
                '@',
                membership_hash
            ),
            '|' ORDER BY component_key
        ) AS member_signature
    FROM "{schema}"."delivery_unit_memberships"
    WHERE release_id = '{release_id}'
    GROUP BY release_id, project_name, dbnum, unit_key, unit_noun
),
hashed AS (
    SELECT
        *,
        sha256(concat(
            '{unit_hash_version}', '|',
            unit_key, '|',
            unit_noun, '|',
            '{rule_set_hash}', '|',
            COALESCE(member_signature, '')
        )) AS aggregate_hash
    FROM grouped
)
INSERT INTO "{schema}"."unit_versions"
SELECT
    release_id,
    project_name,
    dbnum,
    unit_key,
    unit_noun,
    unit_refno_str,
    unit_refno_u64,
    sha256(concat('unit_version_id:v1|', release_id, '|', unit_key, '|', aggregate_hash)) AS unit_version_id,
    aggregate_hash,
    '{unit_hash_version}' AS hash_version,
    '{rule_set_hash}' AS rule_set_hash,
    member_count,
    unresolved_member_count,
    member_signature,
    '{indexed_at}' AS indexed_at
FROM hashed;
"#,
                schema = SCHEMA,
                release_id = release_id,
                membership_hash_version = membership_hash_version,
                unit_hash_version = unit_hash_version,
                rule_set_hash = rule_set_hash,
                indexed_at = indexed_at_sql,
            );

            self.conn
                .execute_batch("BEGIN TRANSACTION")
                .context("begin unit version indexing transaction")?;
            let tx_result = self
                .conn
                .execute_batch(&sql)
                .context("index delivery-unit memberships and unit versions")
                .and_then(|_| self.insert_unit_index_run(release, &indexed_at));
            match tx_result {
                Ok(()) => {
                    self.conn
                        .execute_batch("COMMIT")
                        .context("commit unit version indexing transaction")?;
                }
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
            }

            self.latest_unit_index_stats(release)?
                .with_context(|| format!("unit index stats missing for {}", release.release_id))
        }

        pub fn ensure_release_units_indexed(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelUnitIndexStats> {
            if let Some(stats) = self.latest_unit_index_stats(release)? {
                let current_members = self.delivery_unit_membership_count(&release.release_id)?;
                let current_units = self.unit_version_count(&release.release_id)?;
                if current_members == stats.member_count && current_units == stats.unit_count {
                    return Ok(stats);
                }
            }
            self.index_release_units(release)
        }

        fn require_release_units_indexed(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelUnitIndexStats> {
            let Some(stats) = self.latest_unit_index_stats(release)? else {
                anyhow::bail!(
                    "missing dependency: unit index is missing for release '{}'; run `aios-database model-version index-units --release-id {}` or POST /api/model-version/releases/{}/index-units",
                    release.release_id,
                    release.release_id,
                    release.release_id
                );
            };
            let current_members = self.delivery_unit_membership_count(&release.release_id)?;
            let current_units = self.unit_version_count(&release.release_id)?;
            if current_members != stats.member_count || current_units != stats.unit_count {
                anyhow::bail!(
                    "missing dependency: unit index is stale for release '{}'; indexed_units={} current_units={} indexed_members={} current_members={}. Run `aios-database model-version index-units --release-id {}` or POST /api/model-version/releases/{}/index-units",
                    release.release_id,
                    stats.unit_count,
                    current_units,
                    stats.member_count,
                    current_members,
                    release.release_id,
                    release.release_id
                );
            }
            Ok(stats)
        }

        pub fn diff_units(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            limit: usize,
            unit_noun_filter: Option<&str>,
        ) -> anyhow::Result<ModelUnitDiffResponse> {
            let from_release = self.get_release(from_release_id)?;
            let to_release = self.get_release(to_release_id)?;
            if from_release.project_name != to_release.project_name {
                anyhow::bail!(
                    "cannot diff unit versions from different projects: '{}' vs '{}'",
                    from_release.project_name,
                    to_release.project_name
                );
            }
            if from_release.dbnum != to_release.dbnum {
                anyhow::bail!(
                    "cannot diff unit versions from different dbnums: {} vs {}",
                    from_release.dbnum,
                    to_release.dbnum
                );
            }
            require_release_published(&from_release)?;
            require_release_published(&to_release)?;

            let from_index = self.require_release_units_indexed(&from_release)?;
            let to_index = self.require_release_units_indexed(&to_release)?;
            let normalized_filter = normalize_unit_noun_filter(unit_noun_filter)?;
            let summary = self.unit_diff_summary(
                from_release_id,
                to_release_id,
                normalized_filter.as_deref(),
            )?;
            let rows = self.unit_diff_rows(
                from_release_id,
                to_release_id,
                limit,
                normalized_filter.as_deref(),
            )?;

            Ok(ModelUnitDiffResponse {
                from_release_id: from_release_id.to_string(),
                to_release_id: to_release_id.to_string(),
                project_name: from_release.project_name,
                dbnum: from_release.dbnum,
                from_index,
                to_index,
                summary: ModelUnitDiffSummary {
                    emitted: rows.len(),
                    ..summary
                },
                rows,
            })
        }

        pub fn component_unit_impacts(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            limit: usize,
            component_key_filter: Option<&str>,
        ) -> anyhow::Result<ModelComponentUnitImpactResponse> {
            let from_release = self.get_release(from_release_id)?;
            let to_release = self.get_release(to_release_id)?;
            if from_release.project_name != to_release.project_name {
                anyhow::bail!(
                    "cannot compute unit impact for different projects: '{}' vs '{}'",
                    from_release.project_name,
                    to_release.project_name
                );
            }
            if from_release.dbnum != to_release.dbnum {
                anyhow::bail!(
                    "cannot compute unit impact for different dbnums: {} vs {}",
                    from_release.dbnum,
                    to_release.dbnum
                );
            }
            require_release_published(&from_release)?;
            require_release_published(&to_release)?;

            let from_unit_index = self.require_release_units_indexed(&from_release)?;
            let to_unit_index = self.require_release_units_indexed(&to_release)?;
            let component_diff_summary =
                self.component_diff_summary(from_release_id, to_release_id)?;
            let rows = self.component_unit_impact_rows(
                from_release_id,
                to_release_id,
                limit,
                component_key_filter,
            )?;
            let impacted_units = rows
                .iter()
                .map(|row| row.unit_key.as_str())
                .collect::<HashSet<_>>()
                .len() as u64;
            let component_changes = component_diff_summary.added
                + component_diff_summary.deleted
                + component_diff_summary.changed;

            Ok(ModelComponentUnitImpactResponse {
                from_release_id: from_release_id.to_string(),
                to_release_id: to_release_id.to_string(),
                project_name: from_release.project_name,
                dbnum: from_release.dbnum,
                from_unit_index,
                to_unit_index,
                component_diff_summary,
                summary: ModelComponentUnitImpactSummary {
                    component_changes,
                    impacted_units,
                    emitted: rows.len(),
                },
                rows,
            })
        }

        pub fn release_scene(
            &self,
            release_id: &str,
            limit: usize,
            offset: usize,
            component_key_filter: Option<&str>,
        ) -> anyhow::Result<ModelReleaseSceneResponse> {
            if limit == 0 {
                anyhow::bail!("runtime-scene limit must be at least 1");
            }
            let release = self.get_release(release_id)?;
            require_release_published(&release)?;
            self.require_release_components_indexed(&release)?;
            self.require_release_mesh_assets_ready(&release)?;

            let instances_path = release.immutable_package_dir.join("instances.parquet");
            let geo_instances_path = release.immutable_package_dir.join("geo_instances.parquet");
            let transforms_path = release.immutable_package_dir.join("transforms.parquet");
            let aabb_path = release.immutable_package_dir.join("aabb.parquet");
            ensure_file_exists(&instances_path, "instances.parquet")?;
            ensure_file_exists(&geo_instances_path, "geo_instances.parquet")?;
            ensure_file_exists(&transforms_path, "transforms.parquet")?;
            ensure_file_exists(&aabb_path, "aabb.parquet")?;
            let total_components = release.row_count("instances").unwrap_or(0);

            let release_id_sql = escape_sql_string(&release.release_id);
            let instances = escape_sql_string(&duckdb_path(&instances_path));
            let geo_instances = escape_sql_string(&duckdb_path(&geo_instances_path));
            let transforms = escape_sql_string(&duckdb_path(&transforms_path));
            let aabb = escape_sql_string(&duckdb_path(&aabb_path));
            let component_key_where = component_key_filter
                .map(|value| {
                    format!(
                        "WHERE concat(CAST(i.dbnum AS VARCHAR), ':', CAST(i.refno_u64 AS VARCHAR)) = '{}'",
                        escape_sql_string(value)
                    )
                })
                .unwrap_or_default();
            let sql = format!(
                r#"
WITH selected_instances AS (
    SELECT
        concat(CAST(i.dbnum AS VARCHAR), ':', CAST(i.refno_u64 AS VARCHAR)) AS component_key,
        i.refno_str,
        TRY_CAST(i.refno_u64 AS BIGINT) AS refno_u64,
        i.noun,
        i.owner_refno_str,
        TRY_CAST(i.owner_refno_u64 AS BIGINT) AS owner_refno_u64,
        NULLIF(i.owner_noun, '') AS owner_noun,
        i.cata_hash,
        NULLIF(i.trans_hash, '') AS trans_hash,
        NULLIF(i.aabb_hash, '') AS aabb_hash,
        COALESCE(TRY_CAST(i.spec_value AS BIGINT), 0) AS spec_value,
        COALESCE(i.has_neg, false) AS has_neg,
        s.component_hash
    FROM read_parquet('{instances}') i
    LEFT JOIN "{schema}"."component_snapshots" s
        ON s.release_id = '{release_id}'
       AND s.refno_u64 = TRY_CAST(i.refno_u64 AS BIGINT)
    {component_key_where}
    ORDER BY i.refno_u64
    LIMIT {limit}
    OFFSET {offset}
)
SELECT
    i.component_key,
    i.refno_str,
    i.refno_u64,
    i.noun,
    i.owner_refno_str,
    i.owner_refno_u64,
    i.owner_noun,
    i.cata_hash,
    i.trans_hash,
    i.aabb_hash,
    i.spec_value,
    i.has_neg,
    i.component_hash,
    it.m00, it.m10, it.m20, it.m30,
    it.m01, it.m11, it.m21, it.m31,
    it.m02, it.m12, it.m22, it.m32,
    it.m03, it.m13, it.m23, it.m33,
    ab.min_x, ab.min_y, ab.min_z, ab.max_x, ab.max_y, ab.max_z,
    TRY_CAST(g.geo_index AS BIGINT) AS geo_index,
    g.geo_hash,
    NULLIF(g.geo_trans_hash, '') AS geo_trans_hash,
    gt.m00, gt.m10, gt.m20, gt.m30,
    gt.m01, gt.m11, gt.m21, gt.m31,
    gt.m02, gt.m12, gt.m22, gt.m32,
    gt.m03, gt.m13, gt.m23, gt.m33,
    ma.builtin,
    ma.asset_exists,
    NULLIF(ma.mesh_relative_path, '') AS mesh_relative_path,
    NULLIF(ma.mesh_absolute_path, '') AS mesh_absolute_path,
    NULLIF(ma.mesh_url, '') AS mesh_url,
    ma.bytes,
    NULLIF(ma.sha256, '') AS mesh_sha256,
    ma.glb_readable,
    NULLIF(ma.glb_validation_error, '') AS glb_validation_error
FROM selected_instances i
LEFT JOIN read_parquet('{transforms}') it
    ON i.trans_hash = it.trans_hash
LEFT JOIN read_parquet('{aabb}') ab
    ON i.aabb_hash = ab.aabb_hash
LEFT JOIN read_parquet('{geo_instances}') g
    ON i.refno_u64 = TRY_CAST(g.refno_u64 AS BIGINT)
LEFT JOIN read_parquet('{transforms}') gt
    ON NULLIF(g.geo_trans_hash, '') = gt.trans_hash
LEFT JOIN "{schema}"."model_release_mesh_assets" ma
    ON ma.release_id = '{release_id}'
   AND ma.geo_hash = g.geo_hash
ORDER BY i.refno_u64, g.geo_index, g.geo_hash;
"#,
                schema = SCHEMA,
                release_id = release_id_sql,
                instances = instances,
                geo_instances = geo_instances,
                transforms = transforms,
                aabb = aabb,
                component_key_where = component_key_where,
                limit = limit,
                offset = offset,
            );

            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            let mut components = Vec::<ModelReleaseSceneComponent>::new();
            let mut component_index_by_key = HashMap::<String, usize>::new();

            while let Some(row) = rows.next()? {
                let component_key: String = row.get(0)?;
                let component_index =
                    if let Some(index) = component_index_by_key.get(&component_key) {
                        *index
                    } else {
                        let refno_u64 = i64_to_u64(row.get(2)?, "refno_u64")?;
                        let owner_refno_u64: Option<i64> = row.get(5)?;
                        let component = ModelReleaseSceneComponent {
                            component_key: component_key.clone(),
                            refno_str: row.get(1)?,
                            refno_u64,
                            noun: row.get(3)?,
                            owner_refno_str: clean_string(row.get(4)?),
                            owner_refno_u64: owner_refno_u64
                                .map(|value| i64_to_u64(value, "owner_refno_u64"))
                                .transpose()?,
                            owner_noun: clean_string(row.get(6)?),
                            cata_hash: clean_string(row.get(7)?),
                            trans_hash: clean_string(row.get(8)?),
                            aabb_hash: clean_string(row.get(9)?),
                            spec_value: row.get(10)?,
                            has_neg: row.get(11)?,
                            component_hash: clean_string(row.get(12)?),
                            instance_matrix: read_matrix(row, 13)?,
                            aabb: read_aabb(row, 29)?,
                            geometries: Vec::new(),
                        };
                        components.push(component);
                        let index = components.len() - 1;
                        component_index_by_key.insert(component_key.clone(), index);
                        index
                    };

                let geo_hash: Option<String> = row.get(36)?;
                let Some(geo_hash) = clean_string(geo_hash) else {
                    continue;
                };
                let geo_index_i64: Option<i64> = row.get(35)?;
                let Some(geo_index_i64) = geo_index_i64 else {
                    continue;
                };
                components[component_index]
                    .geometries
                    .push(ModelReleaseSceneGeometry {
                        geo_index: i64_to_u32(geo_index_i64, "geo_index")?,
                        mesh_asset: read_mesh_asset_evidence(row, 54, &geo_hash)?,
                        geo_hash,
                        geo_trans_hash: clean_string(row.get(37)?),
                        geo_matrix: read_matrix(row, 38)?,
                    });
            }

            let geometry_count = components
                .iter()
                .map(|component| component.geometries.len())
                .sum::<usize>();
            let page_end = offset.saturating_add(components.len());
            let has_more = component_key_filter.is_none() && (page_end as u64) < total_components;
            let next_offset = if has_more {
                Some(if components.is_empty() {
                    offset.saturating_add(limit)
                } else {
                    page_end
                })
            } else {
                None
            };

            Ok(ModelReleaseSceneResponse {
                release: release.clone(),
                row_counts: release.rows_by_table.clone(),
                component_count: components.len(),
                geometry_count,
                total_components,
                offset,
                limit,
                next_offset,
                has_more,
                truncated: has_more,
                components,
            })
        }

        fn require_release_mesh_assets_ready(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<()> {
            let geo_instances = release.row_count("geo_instances").unwrap_or(0);
            if geo_instances == 0 {
                return Ok(());
            }

            let stats = self
                .latest_mesh_asset_index_stats(release)?
                .with_context(|| {
                    format!(
                        "missing dependency: mesh asset index is missing for release '{}'; run `aios-database model-version index-assets --release-id {} --materialize` or POST /api/model-version/releases/{}/index-assets?materialize=true before loading runtime-scene",
                        release.release_id, release.release_id, release.release_id
                    )
                })?;

            if stats.missing_count > 0 {
                anyhow::bail!(
                    "missing dependency: release '{}' has {} missing non-builtin mesh assets; repair/generate the missing GLB files and rerun index-assets --materialize before loading runtime-scene",
                    release.release_id,
                    stats.missing_count
                );
            }
            match stats.glb_unreadable_count {
                Some(count) if count > 0 => {
                    anyhow::bail!(
                        "missing dependency: release '{}' has {} unreadable GLB mesh assets; repair/regenerate the bad GLB files and rerun index-assets --materialize before loading runtime-scene",
                        release.release_id,
                        count
                    );
                }
                Some(_) => {}
                None => {
                    anyhow::bail!(
                        "missing dependency: release '{}' mesh asset index lacks GLB readability evidence; rerun index-assets --materialize before loading runtime-scene",
                        release.release_id
                    );
                }
            }
            if let Some(checked) = stats.glb_checked_count
                && checked != stats.present_count
            {
                anyhow::bail!(
                    "missing dependency: release '{}' mesh asset readability evidence is incomplete: checked_count={} present_count={}. Rerun index-assets --materialize.",
                    release.release_id,
                    checked,
                    stats.present_count
                );
            }

            let indexed_rows = self.mesh_asset_row_count(&release.release_id)?;
            if indexed_rows != stats.geo_hash_count {
                anyhow::bail!(
                    "missing dependency: mesh asset index for release '{}' is inconsistent; stats geo_hash_count={} but indexed rows={}. Rerun index-assets --materialize.",
                    release.release_id,
                    stats.geo_hash_count,
                    indexed_rows
                );
            }

            let non_builtin_count = stats.geo_hash_count.saturating_sub(stats.builtin_count);
            if non_builtin_count == 0 {
                return Ok(());
            }

            let release_root = release_root_dir(release)?;
            let mesh_dir = release_root
                .join("meshes")
                .join(format!("lod_{}", stats.lod_tag));
            if !mesh_dir.is_dir() {
                anyhow::bail!(
                    "missing dependency: release-local mesh directory is missing for published release '{}' at {}; rerun index-assets --materialize",
                    release.release_id,
                    mesh_dir.display()
                );
            }

            let non_local = self.release_local_mesh_asset_violation_count(&stats)?;
            if non_local > 0 {
                anyhow::bail!(
                    "missing dependency: release '{}' has {} non-builtin mesh assets that are not materialized under release-local meshes/lod_{}; rerun index-assets --materialize",
                    release.release_id,
                    non_local,
                    stats.lod_tag
                );
            }

            Ok(())
        }

        pub fn index_release_mesh_assets(
            &self,
            release: &ModelReleaseRecord,
            mesh_root: &Path,
            mesh_base_url: Option<&str>,
            materialize: bool,
        ) -> anyhow::Result<ModelReleaseMeshAssetIndexStats> {
            let geo_instances_path = release.immutable_package_dir.join("geo_instances.parquet");
            ensure_file_exists(&geo_instances_path, "geo_instances.parquet")?;
            let manifest_json = read_release_manifest_json(release)?;
            let lod_tag = manifest_mesh_lod_tag(&manifest_json);
            let indexed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let geo_hashes = read_unique_geo_hashes(&geo_instances_path).with_context(|| {
                format!("read geo_hashes from {}", geo_instances_path.display())
            })?;

            let mut assets = Vec::with_capacity(geo_hashes.len());
            for geo_hash in geo_hashes {
                let builtin = is_builtin_geo_hash(&geo_hash);
                let mesh_path = if materialize {
                    let release_local = release_mesh_asset_path(release, &lod_tag, &geo_hash)?;
                    release_local
                        .is_file()
                        .then_some(release_local)
                        .or_else(|| find_mesh_asset(mesh_root, &lod_tag, &geo_hash))
                } else {
                    find_mesh_asset(mesh_root, &lod_tag, &geo_hash)
                };
                let (
                    exists,
                    relative_path,
                    absolute_path,
                    mesh_url,
                    bytes,
                    sha256,
                    glb_readable,
                    glb_validation_error,
                ) = if let Some(path) = mesh_path {
                    let metadata = fs::metadata(&path).with_context(|| {
                        format!("read mesh file metadata failed: {}", path.display())
                    })?;
                    let sha256 = crate::version_management::hashing::sha256_file(&path)?;
                    let (asset_path, relative_path, mesh_url) = if materialize {
                        let release_root = release_root_dir(release)?;
                        let asset_path = materialize_release_mesh_asset(
                            release, &lod_tag, &geo_hash, &path, &sha256,
                        )?;
                        let relative_path = mesh_relative_path(&release_root, &asset_path);
                        let mesh_url = match mesh_base_url {
                            Some(base) => mesh_url_for_relative(base, &relative_path),
                            None => release_mesh_url(release, &relative_path),
                        };
                        (asset_path, relative_path, mesh_url)
                    } else {
                        let relative_path = mesh_relative_path(mesh_root, &path);
                        let base = mesh_base_url.unwrap_or("/files/meshes");
                        let mesh_url = mesh_url_for_relative(base, &relative_path);
                        (path.clone(), relative_path, mesh_url)
                    };
                    let (glb_readable, glb_validation_error) =
                        validate_glb_asset_readable(&asset_path);
                    (
                        true,
                        Some(relative_path),
                        Some(asset_path.canonicalize().unwrap_or(asset_path)),
                        mesh_url,
                        Some(metadata.len()),
                        Some(sha256),
                        Some(glb_readable),
                        glb_validation_error,
                    )
                } else {
                    (false, None, None, None, None, None, None, None)
                };

                assets.push(ModelReleaseMeshAsset {
                    release_id: release.release_id.clone(),
                    project_name: release.project_name.clone(),
                    dbnum: release.dbnum,
                    lod_tag: lod_tag.clone(),
                    geo_hash,
                    builtin,
                    exists,
                    mesh_relative_path: relative_path,
                    mesh_absolute_path: absolute_path,
                    mesh_url,
                    bytes,
                    sha256,
                    glb_readable,
                    glb_validation_error,
                    indexed_at: indexed_at.clone(),
                });
            }

            let present_count = assets.iter().filter(|asset| asset.exists).count() as u64;
            let builtin_count = assets.iter().filter(|asset| asset.builtin).count() as u64;
            let missing_count = assets
                .iter()
                .filter(|asset| !asset.exists && !asset.builtin)
                .count() as u64;
            let total_bytes = assets.iter().filter_map(|asset| asset.bytes).sum::<u64>();
            let glb_checked_count = assets
                .iter()
                .filter(|asset| asset.exists && asset.glb_readable.is_some())
                .count() as u64;
            let glb_readable_count = assets
                .iter()
                .filter(|asset| asset.glb_readable == Some(true))
                .count() as u64;
            let glb_unreadable_count = assets
                .iter()
                .filter(|asset| asset.glb_readable == Some(false))
                .count() as u64;
            let asset_index_hash = mesh_asset_index_hash(&assets)?;
            let manifest_path = self.mesh_asset_manifest_path(release);
            let stats = ModelReleaseMeshAssetIndexStats {
                release_id: release.release_id.clone(),
                project_name: release.project_name.clone(),
                dbnum: release.dbnum,
                lod_tag,
                geo_hash_count: assets.len() as u64,
                present_count,
                missing_count,
                builtin_count,
                total_bytes,
                glb_checked_count: Some(glb_checked_count),
                glb_readable_count: Some(glb_readable_count),
                glb_unreadable_count: Some(glb_unreadable_count),
                asset_index_hash,
                manifest_path,
                indexed_at,
            };

            write_mesh_asset_manifest(&stats, &assets)?;
            self.replace_mesh_asset_index(&stats, &assets)?;
            self.update_release_asset_manifest(&stats)?;
            Ok(stats)
        }

        pub fn get_release_mesh_assets(
            &self,
            release_id: &str,
            limit: usize,
            missing_only: bool,
        ) -> anyhow::Result<ModelReleaseMeshAssetIndexResponse> {
            let release = self.get_release(release_id)?;
            require_release_published(&release)?;
            let stats = self
                .latest_mesh_asset_index_stats(&release)?
                .with_context(|| {
                    format!(
                        "mesh asset index is missing for release '{}'; run index-assets first",
                        release_id
                    )
                })?;
            let assets = self.list_mesh_assets(release_id, limit, missing_only)?;
            Ok(ModelReleaseMeshAssetIndexResponse { stats, assets })
        }

        fn mesh_asset_manifest_path(&self, release: &ModelReleaseRecord) -> PathBuf {
            let root = self
                .cfg
                .metadata_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            root.join("asset_indexes")
                .join(&release.release_id)
                .join(release.dbnum.to_string())
                .join("mesh_assets_manifest.json")
        }

        fn replace_mesh_asset_index(
            &self,
            stats: &ModelReleaseMeshAssetIndexStats,
            assets: &[ModelReleaseMeshAsset],
        ) -> anyhow::Result<()> {
            self.conn
                .execute_batch("BEGIN TRANSACTION")
                .context("begin mesh asset index transaction")?;
            let tx_result = (|| -> anyhow::Result<()> {
                let delete_assets_sql = format!(
                    "DELETE FROM \"{}\".\"model_release_mesh_assets\" WHERE release_id = ?",
                    SCHEMA
                );
                self.conn
                    .execute(&delete_assets_sql, params![stats.release_id])?;
                let delete_run_sql = format!(
                    "DELETE FROM \"{}\".\"model_release_mesh_asset_index_runs\" WHERE release_id = ?",
                    SCHEMA
                );
                self.conn
                    .execute(&delete_run_sql, params![stats.release_id])?;

                let asset_sql = format!(
                    "INSERT INTO \"{}\".\"model_release_mesh_assets\" \
                     (release_id, project_name, dbnum, lod_tag, geo_hash, builtin, asset_exists, \
                      mesh_relative_path, mesh_absolute_path, mesh_url, bytes, sha256, \
                      glb_readable, glb_validation_error, indexed_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    SCHEMA
                );
                let mut stmt = self.conn.prepare(&asset_sql)?;
                for asset in assets {
                    stmt.execute(params![
                        asset.release_id,
                        asset.project_name,
                        i64::from(asset.dbnum),
                        asset.lod_tag,
                        asset.geo_hash,
                        asset.builtin,
                        asset.exists,
                        asset.mesh_relative_path,
                        asset
                            .mesh_absolute_path
                            .as_ref()
                            .map(|path| path.to_string_lossy().to_string()),
                        asset.mesh_url,
                        opt_u64_to_i64(asset.bytes, "mesh asset bytes")?,
                        asset.sha256,
                        asset.glb_readable,
                        asset.glb_validation_error,
                        asset.indexed_at,
                    ])?;
                }

                let run_sql = format!(
                    "INSERT INTO \"{}\".\"model_release_mesh_asset_index_runs\" \
                     (release_id, project_name, dbnum, lod_tag, geo_hash_count, present_count, \
                      missing_count, builtin_count, total_bytes, glb_checked_count, \
                      glb_readable_count, glb_unreadable_count, asset_index_hash, manifest_path, indexed_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    SCHEMA
                );
                self.conn.execute(
                    &run_sql,
                    params![
                        stats.release_id,
                        stats.project_name,
                        i64::from(stats.dbnum),
                        stats.lod_tag,
                        u64_to_i64(stats.geo_hash_count, "geo_hash_count")?,
                        u64_to_i64(stats.present_count, "present_count")?,
                        u64_to_i64(stats.missing_count, "missing_count")?,
                        u64_to_i64(stats.builtin_count, "builtin_count")?,
                        u64_to_i64(stats.total_bytes, "total_bytes")?,
                        opt_u64_to_i64(stats.glb_checked_count, "glb_checked_count")?,
                        opt_u64_to_i64(stats.glb_readable_count, "glb_readable_count")?,
                        opt_u64_to_i64(stats.glb_unreadable_count, "glb_unreadable_count")?,
                        stats.asset_index_hash,
                        stats.manifest_path.to_string_lossy().to_string(),
                        stats.indexed_at,
                    ],
                )?;
                Ok(())
            })();

            match tx_result {
                Ok(()) => self
                    .conn
                    .execute_batch("COMMIT")
                    .context("commit mesh asset index transaction")?,
                Err(err) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(err);
                }
            }
            Ok(())
        }

        fn latest_mesh_asset_index_stats(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<Option<ModelReleaseMeshAssetIndexStats>> {
            let sql = format!(
                "SELECT release_id, project_name, dbnum, lod_tag, geo_hash_count, \
                 present_count, missing_count, builtin_count, total_bytes, glb_checked_count, \
                 glb_readable_count, glb_unreadable_count, asset_index_hash, manifest_path, indexed_at \
                 FROM \"{}\".\"model_release_mesh_asset_index_runs\" WHERE release_id = ? \
                 ORDER BY indexed_at DESC LIMIT 1",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![release.release_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ModelReleaseMeshAssetIndexStats {
                    release_id: row.get(0)?,
                    project_name: row.get(1)?,
                    dbnum: i64_to_u32(row.get(2)?, "dbnum")?,
                    lod_tag: row.get(3)?,
                    geo_hash_count: i64_to_u64(row.get(4)?, "geo_hash_count")?,
                    present_count: i64_to_u64(row.get(5)?, "present_count")?,
                    missing_count: i64_to_u64(row.get(6)?, "missing_count")?,
                    builtin_count: i64_to_u64(row.get(7)?, "builtin_count")?,
                    total_bytes: i64_to_u64(row.get(8)?, "total_bytes")?,
                    glb_checked_count: opt_i64_to_u64(row.get(9)?, "glb_checked_count")?,
                    glb_readable_count: opt_i64_to_u64(row.get(10)?, "glb_readable_count")?,
                    glb_unreadable_count: opt_i64_to_u64(row.get(11)?, "glb_unreadable_count")?,
                    asset_index_hash: row.get(12)?,
                    manifest_path: PathBuf::from(row.get::<_, String>(13)?),
                    indexed_at: row.get(14)?,
                }))
            } else {
                Ok(None)
            }
        }

        fn mesh_asset_row_count(&self, release_id: &str) -> anyhow::Result<u64> {
            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"model_release_mesh_assets\" WHERE release_id = ?",
                SCHEMA
            );
            let count: i64 = self
                .conn
                .query_row(&sql, params![release_id], |row| row.get(0))?;
            Ok(i64_to_u64(count, "mesh asset row count")?)
        }

        fn release_local_mesh_asset_violation_count(
            &self,
            stats: &ModelReleaseMeshAssetIndexStats,
        ) -> anyhow::Result<u64> {
            let expected_prefix = format!("meshes/lod_{}/%", stats.lod_tag);
            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"model_release_mesh_assets\" \
                 WHERE release_id = ? AND builtin = false AND \
                 (asset_exists = false OR mesh_relative_path IS NULL OR mesh_relative_path NOT LIKE ?)",
                SCHEMA
            );
            let count: i64 =
                self.conn
                    .query_row(&sql, params![stats.release_id, expected_prefix], |row| {
                        row.get(0)
                    })?;
            Ok(i64_to_u64(
                count,
                "release-local mesh asset violation count",
            )?)
        }

        fn list_mesh_assets(
            &self,
            release_id: &str,
            limit: usize,
            missing_only: bool,
        ) -> anyhow::Result<Vec<ModelReleaseMeshAsset>> {
            let limit = limit.clamp(1, 100_000);
            let sql = if missing_only {
                format!(
                    "SELECT release_id, project_name, dbnum, lod_tag, geo_hash, builtin, \
                     asset_exists, mesh_relative_path, mesh_absolute_path, mesh_url, bytes, \
                     sha256, glb_readable, glb_validation_error, indexed_at \
                     FROM \"{}\".\"model_release_mesh_assets\" \
                     WHERE release_id = ? AND asset_exists = false AND builtin = false \
                     ORDER BY geo_hash LIMIT ?",
                    SCHEMA
                )
            } else {
                format!(
                    "SELECT release_id, project_name, dbnum, lod_tag, geo_hash, builtin, \
                     asset_exists, mesh_relative_path, mesh_absolute_path, mesh_url, bytes, \
                     sha256, glb_readable, glb_validation_error, indexed_at \
                     FROM \"{}\".\"model_release_mesh_assets\" \
                     WHERE release_id = ? \
                     ORDER BY asset_exists ASC, builtin ASC, geo_hash LIMIT ?",
                    SCHEMA
                )
            };
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![release_id, limit as i64], row_to_mesh_asset)?;
            collect_rows(rows).map_err(Into::into)
        }

        fn insert_release(
            &self,
            release: &ModelReleaseRecord,
            files: &[ModelReleaseFile],
            parent_release_id: Option<&str>,
            manifest_json: &serde_json::Value,
            extra_metadata: &serde_json::Value,
        ) -> anyhow::Result<()> {
            let sql = format!(
                "INSERT INTO \"{}\".\"model_releases\" \
                 (release_id, project_name, branch_id, release_lifecycle, release_quality, \
                  release_quality_reason, validation_flags_json, spec_info_fallback_count, \
                  release_status, release_label, dbnum, \
                  source_package_dir, immutable_package_dir, package_hash, derivation_type, \
                  created_at, registered_at, rows_instances, rows_geo_instances, rows_transforms, \
                  rows_aabb, rows_tubings, rows_ptsets, rows_primitive_keypoints, \
                  source_manifest_path, source_manifest_hash, baseline_state_manifest_path, \
                  baseline_state_manifest_hash, generation_job_id, asset_manifest_path, \
                  asset_manifest_hash) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                SCHEMA
            );
            let validation_flags_json = serde_json::to_string(&release.validation_flags)?;
            let spec_info_fallback_count = release
                .spec_info_fallback_count
                .map(|value| u64_to_i64(value, "spec_info_fallback_count"))
                .transpose()?;
            self.conn.execute(
                &sql,
                params![
                    release.release_id,
                    release.project_name,
                    release.branch_id,
                    release.release_lifecycle.as_str(),
                    release.release_quality.as_str(),
                    release.release_quality_reason,
                    validation_flags_json,
                    spec_info_fallback_count,
                    release.release_status.as_str(),
                    release.release_label,
                    i64::from(release.dbnum),
                    release.source_package_dir.to_string_lossy().to_string(),
                    release.immutable_package_dir.to_string_lossy().to_string(),
                    release.package_hash,
                    release.derivation_type,
                    release.created_at,
                    release.registered_at,
                    opt_u64_to_i64(release.row_count("instances"), "instances")?,
                    opt_u64_to_i64(release.row_count("geo_instances"), "geo_instances")?,
                    opt_u64_to_i64(release.row_count("transforms"), "transforms")?,
                    opt_u64_to_i64(release.row_count("aabb"), "aabb")?,
                    opt_u64_to_i64(release.row_count("tubings"), "tubings")?,
                    opt_u64_to_i64(release.row_count("ptsets"), "ptsets")?,
                    opt_u64_to_i64(
                        release.row_count("primitive_keypoints"),
                        "primitive_keypoints"
                    )?,
                    release
                        .source_manifest_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                    release.source_manifest_hash,
                    release
                        .baseline_state_manifest_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                    release.baseline_state_manifest_hash,
                    release.generation_job_id,
                    release
                        .asset_manifest_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                    release.asset_manifest_hash,
                ],
            )?;

            let status_event_sql = format!(
                "INSERT INTO \"{}\".\"model_release_status_events\" VALUES (?, ?, ?, ?)",
                SCHEMA
            );
            self.conn.execute(
                &status_event_sql,
                params![
                    release.release_id,
                    release.release_status.as_str(),
                    "release registered",
                    release.registered_at
                ],
            )?;

            if let Some(parent) = parent_release_id {
                let edge_sql = format!(
                    "INSERT INTO \"{}\".\"model_release_edges\" VALUES (?, ?, ?, ?)",
                    SCHEMA
                );
                self.conn.execute(
                    &edge_sql,
                    params![release.release_id, parent, "parent", release.registered_at],
                )?;
            }

            let file_sql = format!(
                "INSERT INTO \"{}\".\"model_release_files\" VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&file_sql)?;
            for file in files {
                stmt.execute(params![
                    release.release_id,
                    i64::from(release.dbnum),
                    file.logical_name,
                    file.relative_path,
                    file.absolute_path.to_string_lossy().to_string(),
                    u64_to_i64(file.bytes, &format!("bytes for {}", file.relative_path))?,
                    file.sha256,
                    opt_u64_to_i64(file.rows, &format!("rows for {}", file.relative_path))?,
                    file.required,
                    release.registered_at,
                ])?;
            }

            let metadata_sql = format!(
                "INSERT INTO \"{}\".\"model_release_metadata\" VALUES (?, ?, ?, ?)",
                SCHEMA
            );
            self.conn.execute(
                &metadata_sql,
                params![
                    release.release_id,
                    serde_json::to_string(manifest_json)?,
                    serde_json::to_string(extra_metadata)?,
                    release.registered_at,
                ],
            )?;

            Ok(())
        }

        fn find_release(&self, release_id: &str) -> anyhow::Result<Option<ModelReleaseRecord>> {
            let sql = format!(
                "SELECT release_id, project_name, branch_id, \
                 COALESCE(release_status, 'published') AS release_status, \
                 release_label, dbnum, \
                 source_package_dir, immutable_package_dir, package_hash, derivation_type, \
                 created_at, registered_at, rows_instances, rows_geo_instances, rows_transforms, \
                 rows_aabb, rows_tubings, rows_ptsets, rows_primitive_keypoints, \
                 source_manifest_path, source_manifest_hash, baseline_state_manifest_path, \
                 baseline_state_manifest_hash, generation_job_id, asset_manifest_path, \
                 asset_manifest_hash, release_lifecycle, release_quality, release_quality_reason, \
                 validation_flags_json, spec_info_fallback_count \
                 FROM \"{}\".\"model_releases\" WHERE release_id = ? LIMIT 1",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![release_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_release(row)?))
            } else {
                Ok(None)
            }
        }

        fn list_release_files(&self, release_id: &str) -> anyhow::Result<Vec<ModelReleaseFile>> {
            let sql = format!(
                "SELECT logical_name, relative_path, absolute_path, bytes, sha256, rows, required \
                 FROM \"{}\".\"model_release_files\" WHERE release_id = ? ORDER BY logical_name",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![release_id], |row| {
                let rows_value: Option<i64> = row.get(5)?;
                Ok(ModelReleaseFile {
                    logical_name: row.get(0)?,
                    relative_path: row.get(1)?,
                    absolute_path: std::path::PathBuf::from(row.get::<_, String>(2)?),
                    bytes: i64_to_u64(row.get(3)?, "bytes")?,
                    sha256: row.get(4)?,
                    rows: rows_value
                        .map(|value| i64_to_u64(value, "rows"))
                        .transpose()?,
                    required: row.get(6)?,
                })
            })?;
            collect_rows(rows).map_err(Into::into)
        }

        fn list_release_status_events(
            &self,
            release_id: &str,
        ) -> anyhow::Result<Vec<ModelReleaseStatusEvent>> {
            let sql = format!(
                "SELECT release_id, release_status, reason, created_at \
                 FROM \"{}\".\"model_release_status_events\" \
                 WHERE release_id = ? ORDER BY created_at ASC",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![release_id], |row| {
                let release_status: Option<String> = row.get(1)?;
                Ok(ModelReleaseStatusEvent {
                    release_id: row.get(0)?,
                    release_status: ModelReleaseStatus::from_storage(release_status),
                    reason: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?;
            collect_rows(rows).map_err(Into::into)
        }

        fn find_parent_release_id(&self, release_id: &str) -> anyhow::Result<Option<String>> {
            let sql = format!(
                "SELECT parent_release_id FROM \"{}\".\"model_release_edges\" \
                 WHERE release_id = ? AND edge_type = 'parent' LIMIT 1",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![release_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        }

        fn insert_component_index_run(
            &self,
            release: &ModelReleaseRecord,
            indexed_at: &str,
        ) -> anyhow::Result<()> {
            let (component_count, distinct_component_hashes) = self
                .component_snapshot_counts(&release.release_id)
                .context("count indexed component snapshots")?;
            let delete_sql = format!(
                "DELETE FROM \"{}\".\"component_index_runs\" WHERE release_id = ?",
                SCHEMA
            );
            self.conn
                .execute(&delete_sql, params![release.release_id])?;
            let insert_sql = format!(
                "INSERT INTO \"{}\".\"component_index_runs\" VALUES (?, ?, ?, ?, ?, ?, ?)",
                SCHEMA
            );
            self.conn.execute(
                &insert_sql,
                params![
                    release.release_id,
                    release.project_name,
                    i64::from(release.dbnum),
                    COMPONENT_HASH_VERSION,
                    u64_to_i64(component_count, "component_count")?,
                    u64_to_i64(distinct_component_hashes, "distinct_component_hashes")?,
                    indexed_at,
                ],
            )?;
            Ok(())
        }

        fn latest_component_index_stats(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<Option<ModelComponentSnapshotStats>> {
            let sql = format!(
                "SELECT release_id, project_name, dbnum, hash_version, component_count, \
                 distinct_component_hashes, indexed_at \
                 FROM \"{}\".\"component_index_runs\" WHERE release_id = ? \
                 ORDER BY indexed_at DESC LIMIT 1",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![release.release_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ModelComponentSnapshotStats {
                    release_id: row.get(0)?,
                    project_name: row.get(1)?,
                    dbnum: i64_to_u32(row.get(2)?, "dbnum")?,
                    hash_version: row.get(3)?,
                    component_count: i64_to_u64(row.get(4)?, "component_count")?,
                    distinct_component_hashes: i64_to_u64(
                        row.get(5)?,
                        "distinct_component_hashes",
                    )?,
                    indexed_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        }

        fn component_snapshot_count(&self, release_id: &str) -> anyhow::Result<u64> {
            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"component_snapshots\" WHERE release_id = ?",
                SCHEMA
            );
            let count: i64 = self
                .conn
                .query_row(&sql, params![release_id], |row| row.get(0))?;
            i64_to_u64(count, "component_snapshot_count").map_err(Into::into)
        }

        fn component_snapshot_counts(&self, release_id: &str) -> anyhow::Result<(u64, u64)> {
            let sql = format!(
                "SELECT COUNT(*), COUNT(DISTINCT component_hash) \
                 FROM \"{}\".\"component_snapshots\" WHERE release_id = ?",
                SCHEMA
            );
            let (count, distinct): (i64, i64) =
                self.conn.query_row(&sql, params![release_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
            Ok((
                i64_to_u64(count, "component_count")?,
                i64_to_u64(distinct, "distinct_component_hashes")?,
            ))
        }

        fn component_diff_summary(
            &self,
            from_release_id: &str,
            to_release_id: &str,
        ) -> anyhow::Result<ModelComponentDiffSummary> {
            let sql = format!(
                r#"
WITH old_components AS (
    SELECT * FROM "{schema}"."component_snapshots" WHERE release_id = ?
),
new_components AS (
    SELECT * FROM "{schema}"."component_snapshots" WHERE release_id = ?
),
joined AS (
    SELECT
        o.component_key AS old_key,
        n.component_key AS new_key,
        o.component_hash AS old_hash,
        n.component_hash AS new_hash
    FROM old_components o
    FULL OUTER JOIN new_components n ON o.component_key = n.component_key
)
SELECT
    COALESCE(SUM(CASE WHEN old_key IS NULL THEN 1 ELSE 0 END), 0) AS added,
    COALESCE(SUM(CASE WHEN new_key IS NULL THEN 1 ELSE 0 END), 0) AS deleted,
    COALESCE(SUM(CASE WHEN old_key IS NOT NULL AND new_key IS NOT NULL AND old_hash <> new_hash THEN 1 ELSE 0 END), 0) AS changed,
    COALESCE(SUM(CASE WHEN old_key IS NOT NULL AND new_key IS NOT NULL AND old_hash = new_hash THEN 1 ELSE 0 END), 0) AS unchanged,
    COALESCE(COUNT(old_key), 0) AS total_old,
    COALESCE(COUNT(new_key), 0) AS total_new
FROM joined
"#,
                schema = SCHEMA
            );
            let (added, deleted, changed, unchanged, total_old, total_new): (
                i64,
                i64,
                i64,
                i64,
                i64,
                i64,
            ) = self
                .conn
                .query_row(&sql, params![from_release_id, to_release_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?;
            Ok(ModelComponentDiffSummary {
                added: i64_to_u64(added, "added")?,
                deleted: i64_to_u64(deleted, "deleted")?,
                changed: i64_to_u64(changed, "changed")?,
                unchanged: i64_to_u64(unchanged, "unchanged")?,
                total_old: i64_to_u64(total_old, "total_old")?,
                total_new: i64_to_u64(total_new, "total_new")?,
                emitted: 0,
            })
        }

        fn component_diff_rows(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            limit: usize,
            change_type_filter: Option<&str>,
        ) -> anyhow::Result<Vec<ModelComponentDiffRow>> {
            let filter_sql = match change_type_filter {
                Some("added") => "AND change_type = 'added'",
                Some("deleted") => "AND change_type = 'deleted'",
                Some("changed") => "AND change_type = 'changed'",
                Some(other) => anyhow::bail!(
                    "unsupported component diff change type '{}'; expected added, deleted, or changed",
                    other
                ),
                None => "",
            };
            let sql = format!(
                r#"
WITH old_components AS (
    SELECT * FROM "{schema}"."component_snapshots" WHERE release_id = ?
),
new_components AS (
    SELECT * FROM "{schema}"."component_snapshots" WHERE release_id = ?
),
joined AS (
    SELECT
        CASE
            WHEN o.component_key IS NULL THEN 'added'
            WHEN n.component_key IS NULL THEN 'deleted'
            WHEN o.component_hash <> n.component_hash THEN 'changed'
            ELSE 'unchanged'
        END AS change_type,
        COALESCE(n.component_key, o.component_key) AS component_key,
        COALESCE(n.dbnum, o.dbnum) AS dbnum,
        COALESCE(n.refno_str, o.refno_str) AS refno_str,
        COALESCE(n.refno_u64, o.refno_u64) AS refno_u64,
        COALESCE(n.noun, o.noun) AS noun,
        o.component_hash AS old_component_hash,
        n.component_hash AS new_component_hash,
        o.owner_refno_str AS old_owner_refno_str,
        n.owner_refno_str AS new_owner_refno_str,
        o.cata_hash AS old_cata_hash,
        n.cata_hash AS new_cata_hash,
        o.trans_hash AS old_trans_hash,
        n.trans_hash AS new_trans_hash,
        o.aabb_hash AS old_aabb_hash,
        n.aabb_hash AS new_aabb_hash
    FROM old_components o
    FULL OUTER JOIN new_components n ON o.component_key = n.component_key
)
SELECT
    change_type,
    component_key,
    dbnum,
    refno_str,
    refno_u64,
    noun,
    old_component_hash,
    new_component_hash,
    old_owner_refno_str,
    new_owner_refno_str,
    old_cata_hash,
    new_cata_hash,
    old_trans_hash,
    new_trans_hash,
    old_aabb_hash,
    new_aabb_hash
FROM joined
WHERE change_type <> 'unchanged'
{filter_sql}
ORDER BY
    CASE change_type WHEN 'added' THEN 0 WHEN 'deleted' THEN 1 ELSE 2 END,
    component_key
LIMIT {limit}
"#,
                schema = SCHEMA,
                filter_sql = filter_sql,
                limit = limit
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![from_release_id, to_release_id], |row| {
                let refno_u64: Option<i64> = row.get(4)?;
                Ok(ModelComponentDiffRow {
                    change_type: row.get(0)?,
                    component_key: row.get(1)?,
                    dbnum: i64_to_u32(row.get(2)?, "dbnum")?,
                    refno_str: row.get(3)?,
                    refno_u64: refno_u64
                        .map(|value| i64_to_u64(value, "refno_u64"))
                        .transpose()?,
                    noun: row.get(5)?,
                    old_component_hash: row.get(6)?,
                    new_component_hash: row.get(7)?,
                    old_owner_refno_str: row.get(8)?,
                    new_owner_refno_str: row.get(9)?,
                    old_cata_hash: row.get(10)?,
                    new_cata_hash: row.get(11)?,
                    old_trans_hash: row.get(12)?,
                    new_trans_hash: row.get(13)?,
                    old_aabb_hash: row.get(14)?,
                    new_aabb_hash: row.get(15)?,
                })
            })?;
            collect_rows(rows).map_err(Into::into)
        }

        fn insert_unit_index_run(
            &self,
            release: &ModelReleaseRecord,
            indexed_at: &str,
        ) -> anyhow::Result<()> {
            let (unit_count, member_count, unresolved_member_count) =
                self.unit_index_counts(&release.release_id)?;
            let delete_sql = format!(
                "DELETE FROM \"{}\".\"unit_index_runs\" WHERE release_id = ?",
                SCHEMA
            );
            self.conn
                .execute(&delete_sql, params![release.release_id])?;
            let insert_sql = format!(
                "INSERT INTO \"{}\".\"unit_index_runs\" VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                SCHEMA
            );
            self.conn.execute(
                &insert_sql,
                params![
                    release.release_id,
                    release.project_name,
                    i64::from(release.dbnum),
                    UNIT_HASH_VERSION,
                    UNIT_RULE_SET_HASH,
                    u64_to_i64(unit_count, "unit_count")?,
                    u64_to_i64(member_count, "member_count")?,
                    u64_to_i64(unresolved_member_count, "unresolved_member_count")?,
                    indexed_at,
                ],
            )?;
            Ok(())
        }

        fn latest_unit_index_stats(
            &self,
            release: &ModelReleaseRecord,
        ) -> anyhow::Result<Option<ModelUnitIndexStats>> {
            let sql = format!(
                "SELECT release_id, project_name, dbnum, hash_version, rule_set_hash, \
                 unit_count, member_count, unresolved_member_count, indexed_at \
                 FROM \"{}\".\"unit_index_runs\" WHERE release_id = ? \
                 ORDER BY indexed_at DESC LIMIT 1",
                SCHEMA
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params![release.release_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ModelUnitIndexStats {
                    release_id: row.get(0)?,
                    project_name: row.get(1)?,
                    dbnum: i64_to_u32(row.get(2)?, "dbnum")?,
                    hash_version: row.get(3)?,
                    rule_set_hash: row.get(4)?,
                    unit_count: i64_to_u64(row.get(5)?, "unit_count")?,
                    member_count: i64_to_u64(row.get(6)?, "member_count")?,
                    unresolved_member_count: i64_to_u64(row.get(7)?, "unresolved_member_count")?,
                    indexed_at: row.get(8)?,
                }))
            } else {
                Ok(None)
            }
        }

        fn delivery_unit_membership_count(&self, release_id: &str) -> anyhow::Result<u64> {
            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"delivery_unit_memberships\" WHERE release_id = ?",
                SCHEMA
            );
            let count: i64 = self
                .conn
                .query_row(&sql, params![release_id], |row| row.get(0))?;
            i64_to_u64(count, "delivery_unit_membership_count").map_err(Into::into)
        }

        fn unit_version_count(&self, release_id: &str) -> anyhow::Result<u64> {
            let sql = format!(
                "SELECT COUNT(*) FROM \"{}\".\"unit_versions\" WHERE release_id = ?",
                SCHEMA
            );
            let count: i64 = self
                .conn
                .query_row(&sql, params![release_id], |row| row.get(0))?;
            i64_to_u64(count, "unit_version_count").map_err(Into::into)
        }

        fn unit_index_counts(&self, release_id: &str) -> anyhow::Result<(u64, u64, u64)> {
            let sql = format!(
                "SELECT \
                    (SELECT COUNT(*) FROM \"{schema}\".\"unit_versions\" WHERE release_id = ?), \
                    (SELECT COUNT(*) FROM \"{schema}\".\"delivery_unit_memberships\" WHERE release_id = ?), \
                    (SELECT COALESCE(SUM(CASE WHEN unresolved_reason IS NULL THEN 0 ELSE 1 END), 0) \
                       FROM \"{schema}\".\"delivery_unit_memberships\" WHERE release_id = ?)",
                schema = SCHEMA
            );
            let (units, members, unresolved): (i64, i64, i64) =
                self.conn
                    .query_row(&sql, params![release_id, release_id, release_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?;
            Ok((
                i64_to_u64(units, "unit_count")?,
                i64_to_u64(members, "member_count")?,
                i64_to_u64(unresolved, "unresolved_member_count")?,
            ))
        }

        fn unit_diff_summary(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            unit_noun_filter: Option<&str>,
        ) -> anyhow::Result<ModelUnitDiffSummary> {
            let filter_sql = unit_noun_filter
                .map(|value| {
                    format!(
                        "WHERE COALESCE(n.unit_noun, o.unit_noun) = '{}'",
                        escape_sql_string(value)
                    )
                })
                .unwrap_or_default();
            let sql = format!(
                r#"
WITH old_units AS (
    SELECT * FROM "{schema}"."unit_versions" WHERE release_id = ?
),
new_units AS (
    SELECT * FROM "{schema}"."unit_versions" WHERE release_id = ?
),
joined AS (
    SELECT
        o.unit_key AS old_key,
        n.unit_key AS new_key,
        o.aggregate_hash AS old_hash,
        n.aggregate_hash AS new_hash,
        o.unit_noun AS old_unit_noun,
        n.unit_noun AS new_unit_noun
    FROM old_units o
    FULL OUTER JOIN new_units n ON o.unit_key = n.unit_key
    {filter_sql}
)
SELECT
    COALESCE(SUM(CASE WHEN old_key IS NULL THEN 1 ELSE 0 END), 0) AS added,
    COALESCE(SUM(CASE WHEN new_key IS NULL THEN 1 ELSE 0 END), 0) AS deleted,
    COALESCE(SUM(CASE WHEN old_key IS NOT NULL AND new_key IS NOT NULL AND old_hash <> new_hash THEN 1 ELSE 0 END), 0) AS changed,
    COALESCE(SUM(CASE WHEN old_key IS NOT NULL AND new_key IS NOT NULL AND old_hash = new_hash THEN 1 ELSE 0 END), 0) AS unchanged,
    COALESCE(COUNT(old_key), 0) AS total_old,
    COALESCE(COUNT(new_key), 0) AS total_new
FROM joined
"#,
                schema = SCHEMA,
                filter_sql = filter_sql
            );
            let (added, deleted, changed, unchanged, total_old, total_new): (
                i64,
                i64,
                i64,
                i64,
                i64,
                i64,
            ) = self
                .conn
                .query_row(&sql, params![from_release_id, to_release_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?;
            Ok(ModelUnitDiffSummary {
                added: i64_to_u64(added, "added")?,
                deleted: i64_to_u64(deleted, "deleted")?,
                changed: i64_to_u64(changed, "changed")?,
                unchanged: i64_to_u64(unchanged, "unchanged")?,
                total_old: i64_to_u64(total_old, "total_old")?,
                total_new: i64_to_u64(total_new, "total_new")?,
                emitted: 0,
            })
        }

        fn unit_diff_rows(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            limit: usize,
            unit_noun_filter: Option<&str>,
        ) -> anyhow::Result<Vec<ModelUnitDiffRow>> {
            let filter_sql = unit_noun_filter
                .map(|value| format!("AND unit_noun = '{}'", escape_sql_string(value)))
                .unwrap_or_default();
            let sql = format!(
                r#"
WITH old_units AS (
    SELECT * FROM "{schema}"."unit_versions" WHERE release_id = ?
),
new_units AS (
    SELECT * FROM "{schema}"."unit_versions" WHERE release_id = ?
),
joined AS (
    SELECT
        CASE
            WHEN o.unit_key IS NULL THEN 'added'
            WHEN n.unit_key IS NULL THEN 'deleted'
            WHEN o.aggregate_hash <> n.aggregate_hash THEN 'changed'
            ELSE 'unchanged'
        END AS change_type,
        COALESCE(n.unit_key, o.unit_key) AS unit_key,
        COALESCE(n.unit_noun, o.unit_noun) AS unit_noun,
        COALESCE(n.unit_refno_str, o.unit_refno_str) AS unit_refno_str,
        COALESCE(n.unit_refno_u64, o.unit_refno_u64) AS unit_refno_u64,
        o.unit_version_id AS old_unit_version_id,
        n.unit_version_id AS new_unit_version_id,
        o.aggregate_hash AS old_aggregate_hash,
        n.aggregate_hash AS new_aggregate_hash,
        o.member_count AS old_member_count,
        n.member_count AS new_member_count,
        o.unresolved_member_count AS old_unresolved_member_count,
        n.unresolved_member_count AS new_unresolved_member_count
    FROM old_units o
    FULL OUTER JOIN new_units n ON o.unit_key = n.unit_key
)
SELECT
    change_type,
    unit_key,
    unit_noun,
    unit_refno_str,
    unit_refno_u64,
    old_unit_version_id,
    new_unit_version_id,
    old_aggregate_hash,
    new_aggregate_hash,
    old_member_count,
    new_member_count,
    old_unresolved_member_count,
    new_unresolved_member_count
FROM joined
WHERE change_type <> 'unchanged'
{filter_sql}
ORDER BY
    CASE change_type WHEN 'added' THEN 0 WHEN 'deleted' THEN 1 ELSE 2 END,
    unit_noun,
    unit_key
LIMIT {limit}
"#,
                schema = SCHEMA,
                filter_sql = filter_sql,
                limit = limit
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![from_release_id, to_release_id], |row| {
                let unit_refno_u64: Option<i64> = row.get(4)?;
                let old_member_count: Option<i64> = row.get(9)?;
                let new_member_count: Option<i64> = row.get(10)?;
                let old_unresolved_member_count: Option<i64> = row.get(11)?;
                let new_unresolved_member_count: Option<i64> = row.get(12)?;
                Ok(ModelUnitDiffRow {
                    change_type: row.get(0)?,
                    unit_key: row.get(1)?,
                    unit_noun: row.get(2)?,
                    unit_refno_str: row.get(3)?,
                    unit_refno_u64: opt_i64_to_u64(unit_refno_u64, "unit_refno_u64")?,
                    old_unit_version_id: row.get(5)?,
                    new_unit_version_id: row.get(6)?,
                    old_aggregate_hash: row.get(7)?,
                    new_aggregate_hash: row.get(8)?,
                    old_member_count: opt_i64_to_u64(old_member_count, "old_member_count")?,
                    new_member_count: opt_i64_to_u64(new_member_count, "new_member_count")?,
                    old_unresolved_member_count: opt_i64_to_u64(
                        old_unresolved_member_count,
                        "old_unresolved_member_count",
                    )?,
                    new_unresolved_member_count: opt_i64_to_u64(
                        new_unresolved_member_count,
                        "new_unresolved_member_count",
                    )?,
                })
            })?;
            collect_rows(rows).map_err(Into::into)
        }

        fn component_unit_impact_rows(
            &self,
            from_release_id: &str,
            to_release_id: &str,
            limit: usize,
            component_key_filter: Option<&str>,
        ) -> anyhow::Result<Vec<ModelComponentUnitImpactRow>> {
            let filter_sql = component_key_filter
                .map(|value| format!("AND component_key = '{}'", escape_sql_string(value)))
                .unwrap_or_default();
            let sql = format!(
                r#"
WITH old_components AS (
    SELECT * FROM "{schema}"."component_snapshots" WHERE release_id = ?
),
new_components AS (
    SELECT * FROM "{schema}"."component_snapshots" WHERE release_id = ?
),
diff AS (
    SELECT
        CASE
            WHEN o.component_key IS NULL THEN 'added'
            WHEN n.component_key IS NULL THEN 'deleted'
            WHEN o.component_hash <> n.component_hash THEN 'changed'
            ELSE 'unchanged'
        END AS change_type,
        COALESCE(n.component_key, o.component_key) AS component_key,
        COALESCE(n.dbnum, o.dbnum) AS dbnum,
        COALESCE(n.refno_str, o.refno_str) AS refno_str,
        COALESCE(n.refno_u64, o.refno_u64) AS refno_u64,
        COALESCE(n.noun, o.noun) AS noun,
        o.component_hash AS old_component_hash,
        n.component_hash AS new_component_hash
    FROM old_components o
    FULL OUTER JOIN new_components n ON o.component_key = n.component_key
),
changed AS (
    SELECT * FROM diff
    WHERE change_type <> 'unchanged'
    {filter_sql}
),
with_membership AS (
    SELECT
        d.*,
        om.unit_key AS old_unit_key,
        om.unit_noun AS old_unit_noun,
        om.unit_refno_str AS old_unit_refno_str,
        om.unit_refno_u64 AS old_unit_refno_u64,
        om.membership_kind AS old_membership_kind,
        nm.unit_key AS new_unit_key,
        nm.unit_noun AS new_unit_noun,
        nm.unit_refno_str AS new_unit_refno_str,
        nm.unit_refno_u64 AS new_unit_refno_u64,
        nm.membership_kind AS new_membership_kind
    FROM changed d
    LEFT JOIN "{schema}"."delivery_unit_memberships" om
        ON om.release_id = ? AND om.component_key = d.component_key
    LEFT JOIN "{schema}"."delivery_unit_memberships" nm
        ON nm.release_id = ? AND nm.component_key = d.component_key
),
impact_rows AS (
    SELECT
        CASE
            WHEN change_type = 'added' THEN 'member_added'
            WHEN change_type = 'deleted' THEN 'member_deleted'
            WHEN old_unit_key IS NOT NULL AND new_unit_key IS NOT NULL AND old_unit_key <> new_unit_key THEN 'member_moved_out'
            ELSE 'member_changed'
        END AS impact_kind,
        CASE
            WHEN change_type = 'added' THEN new_unit_key
            WHEN change_type = 'deleted' THEN old_unit_key
            WHEN old_unit_key IS NOT NULL AND new_unit_key IS NOT NULL AND old_unit_key <> new_unit_key THEN old_unit_key
            ELSE COALESCE(new_unit_key, old_unit_key)
        END AS selected_unit_key,
        *
    FROM with_membership
    WHERE COALESCE(new_unit_key, old_unit_key) IS NOT NULL

    UNION ALL

    SELECT
        'member_moved_in' AS impact_kind,
        new_unit_key AS selected_unit_key,
        *
    FROM with_membership
    WHERE old_unit_key IS NOT NULL
      AND new_unit_key IS NOT NULL
      AND old_unit_key <> new_unit_key
)
SELECT
    impact_kind,
    '{rule_id}' AS rule_id,
    i.component_key,
    i.dbnum,
    i.refno_str,
    i.refno_u64,
    i.noun,
    i.change_type,
    selected_unit_key AS unit_key,
    COALESCE(
        CASE WHEN selected_unit_key = new_unit_key THEN new_unit_noun END,
        CASE WHEN selected_unit_key = old_unit_key THEN old_unit_noun END,
        new_unit_noun,
        old_unit_noun
    ) AS unit_noun,
    COALESCE(
        CASE WHEN selected_unit_key = new_unit_key THEN new_unit_refno_str END,
        CASE WHEN selected_unit_key = old_unit_key THEN old_unit_refno_str END,
        new_unit_refno_str,
        old_unit_refno_str
    ) AS unit_refno_str,
    COALESCE(
        CASE WHEN selected_unit_key = new_unit_key THEN new_unit_refno_u64 END,
        CASE WHEN selected_unit_key = old_unit_key THEN old_unit_refno_u64 END,
        new_unit_refno_u64,
        old_unit_refno_u64
    ) AS unit_refno_u64,
    ou.unit_version_id AS old_unit_version_id,
    nu.unit_version_id AS new_unit_version_id,
    ou.aggregate_hash AS old_aggregate_hash,
    nu.aggregate_hash AS new_aggregate_hash,
    old_component_hash,
    new_component_hash,
    old_membership_kind,
    new_membership_kind,
    concat(
        '["',
        replace(COALESCE(noun, 'UNKNOWN'), '"', ''),
        '","',
        replace(COALESCE(old_unit_key, 'NONE'), '"', ''),
        '","',
        replace(COALESCE(new_unit_key, 'NONE'), '"', ''),
        '"]'
    ) AS dependency_path_json,
    concat(
        '{{"rule_set_hash":"{rule_set_hash}","old_unit_key":"',
        replace(COALESCE(old_unit_key, ''), '"', ''),
        '","new_unit_key":"',
        replace(COALESCE(new_unit_key, ''), '"', ''),
        '","old_membership_kind":"',
        replace(COALESCE(old_membership_kind, ''), '"', ''),
        '","new_membership_kind":"',
        replace(COALESCE(new_membership_kind, ''), '"', ''),
        '"}}'
    ) AS evidence_json
FROM impact_rows i
LEFT JOIN "{schema}"."unit_versions" ou
    ON ou.release_id = ? AND ou.unit_key = i.selected_unit_key
LEFT JOIN "{schema}"."unit_versions" nu
    ON nu.release_id = ? AND nu.unit_key = i.selected_unit_key
ORDER BY
    CASE impact_kind
        WHEN 'member_added' THEN 0
        WHEN 'member_deleted' THEN 1
        WHEN 'member_moved_out' THEN 2
        WHEN 'member_moved_in' THEN 3
        ELSE 4
    END,
    component_key,
    unit_key
LIMIT {limit}
"#,
                schema = SCHEMA,
                rule_id = COMPONENT_CHANGE_RULE_ID,
                rule_set_hash = UNIT_RULE_SET_HASH,
                filter_sql = filter_sql,
                limit = limit
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![
                    from_release_id,
                    to_release_id,
                    from_release_id,
                    to_release_id,
                    from_release_id,
                    to_release_id,
                ],
                |row| {
                    let refno_u64: Option<i64> = row.get(5)?;
                    let unit_refno_u64: Option<i64> = row.get(11)?;
                    Ok(ModelComponentUnitImpactRow {
                        impact_kind: row.get(0)?,
                        rule_id: row.get(1)?,
                        component_key: row.get(2)?,
                        dbnum: i64_to_u32(row.get(3)?, "dbnum")?,
                        refno_str: row.get(4)?,
                        refno_u64: opt_i64_to_u64(refno_u64, "refno_u64")?,
                        noun: row.get(6)?,
                        change_type: row.get(7)?,
                        unit_key: row.get(8)?,
                        unit_noun: row.get(9)?,
                        unit_refno_str: row.get(10)?,
                        unit_refno_u64: opt_i64_to_u64(unit_refno_u64, "unit_refno_u64")?,
                        old_unit_version_id: row.get(12)?,
                        new_unit_version_id: row.get(13)?,
                        old_aggregate_hash: row.get(14)?,
                        new_aggregate_hash: row.get(15)?,
                        old_component_hash: row.get(16)?,
                        new_component_hash: row.get(17)?,
                        old_membership_kind: row.get(18)?,
                        new_membership_kind: row.get(19)?,
                        dependency_path_json: row.get(20)?,
                        evidence_json: row.get(21)?,
                    })
                },
            )?;
            collect_rows(rows).map_err(Into::into)
        }
    }

    impl ModelReleaseRecord {
        fn row_count(&self, table: &str) -> Option<u64> {
            self.rows_by_table.get(table).copied()
        }
    }

    fn row_to_release(row: &duckdb::Row<'_>) -> duckdb::Result<ModelReleaseRecord> {
        let mut rows_by_table = std::collections::BTreeMap::new();
        let rows_instances = row.get(12)?;
        let rows_geo_instances = row.get(13)?;
        push_row_count(&mut rows_by_table, "instances", rows_instances)?;
        push_row_count(&mut rows_by_table, "geo_instances", rows_geo_instances)?;
        push_row_count(&mut rows_by_table, "transforms", row.get(14)?)?;
        push_row_count(&mut rows_by_table, "aabb", row.get(15)?)?;
        push_row_count(&mut rows_by_table, "tubings", row.get(16)?)?;
        push_row_count(&mut rows_by_table, "ptsets", row.get(17)?)?;
        push_row_count(&mut rows_by_table, "primitive_keypoints", row.get(18)?)?;

        let dbnum_i64: i64 = row.get(5)?;
        let release_id: String = row.get(0)?;
        let release_label: Option<String> = row.get(4)?;
        let release_status = ModelReleaseStatus::from_storage(row.get(3)?);
        let derivation_type: String = row.get(9)?;
        let release_lifecycle = ModelReleaseLifecycle::from_storage(row.get(26)?, &release_status);
        let release_quality = ModelReleaseQuality::from_storage_or_infer(
            row.get(27)?,
            &release_status,
            &release_id,
            release_label.as_deref(),
            &derivation_type,
            rows_instances
                .map(|value| i64_to_u64(value, "instances"))
                .transpose()?,
            rows_geo_instances
                .map(|value| i64_to_u64(value, "geo_instances"))
                .transpose()?,
        );
        let validation_flags = validation_flags_from_json(row.get(29)?);
        let spec_info_fallback_count = row
            .get::<_, Option<i64>>(30)?
            .map(|value| i64_to_u64(value, "spec_info_fallback_count"))
            .transpose()?;
        Ok(ModelReleaseRecord {
            release_id,
            project_name: row.get(1)?,
            branch_id: row.get(2)?,
            release_lifecycle,
            release_quality,
            release_quality_reason: row.get(28)?,
            validation_flags,
            spec_info_fallback_count,
            release_status,
            release_label,
            dbnum: i64_to_u32(dbnum_i64, "dbnum")?,
            source_package_dir: std::path::PathBuf::from(row.get::<_, String>(6)?),
            immutable_package_dir: std::path::PathBuf::from(row.get::<_, String>(7)?),
            package_hash: row.get(8)?,
            derivation_type,
            created_at: row.get(10)?,
            registered_at: row.get(11)?,
            rows_by_table,
            source_manifest_path: optional_path(row.get(19)?),
            source_manifest_hash: row.get(20)?,
            baseline_state_manifest_path: optional_path(row.get(21)?),
            baseline_state_manifest_hash: row.get(22)?,
            generation_job_id: row.get(23)?,
            asset_manifest_path: optional_path(row.get(24)?),
            asset_manifest_hash: row.get(25)?,
        })
    }

    fn validation_flags_from_json(value: Option<String>) -> Vec<String> {
        let Some(raw) = value else {
            return Vec::new();
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        if let Ok(values) = serde_json::from_str::<Vec<String>>(trimmed) {
            return values
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
        }
        trimmed
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    fn optional_path(value: Option<String>) -> Option<std::path::PathBuf> {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    }

    fn require_release_published(release: &ModelReleaseRecord) -> anyhow::Result<()> {
        if release.release_lifecycle != ModelReleaseLifecycle::Published {
            anyhow::bail!(
                "model release '{}' is not published (lifecycle={}, legacy_status={}, quality={}); complete or repair the publish workflow before using read APIs",
                release.release_id,
                release.release_lifecycle.as_str(),
                release.release_status.as_str(),
                release.release_quality.as_str()
            );
        }
        Ok(())
    }

    fn legacy_status_quality(status: &ModelReleaseStatus) -> Option<ModelReleaseQuality> {
        match status {
            ModelReleaseStatus::Degraded => Some(ModelReleaseQuality::DegradedVisual),
            ModelReleaseStatus::Quarantined => Some(ModelReleaseQuality::QuarantinedVisual),
            ModelReleaseStatus::PatchOnly => Some(ModelReleaseQuality::PatchOnly),
            _ => None,
        }
    }

    fn push_row_count(
        rows_by_table: &mut std::collections::BTreeMap<String, u64>,
        table: &str,
        value: Option<i64>,
    ) -> duckdb::Result<()> {
        if let Some(value) = value {
            rows_by_table.insert(table.to_string(), i64_to_u64(value, table)?);
        }
        Ok(())
    }

    fn collect_rows<T, I>(rows: I) -> duckdb::Result<Vec<T>>
    where
        I: IntoIterator<Item = duckdb::Result<T>>,
    {
        rows.into_iter().collect()
    }

    fn duckdb_path(path: &Path) -> String {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| PathBuf::from(path))
        };
        absolute.to_string_lossy().replace('\\', "/")
    }

    fn escape_sql_string(value: &str) -> String {
        value.replace('\'', "''")
    }

    fn clean_string(value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn normalize_unit_noun_filter(raw: Option<&str>) -> anyhow::Result<Option<String>> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        let value = raw.trim().to_ascii_uppercase();
        if value.is_empty() {
            return Ok(None);
        }
        let normalized = if value == "EQUIP" {
            "EQUI"
        } else {
            value.as_str()
        };
        match normalized {
            "BRAN" | "HANG" | "EQUI" | "WALL" | "FLOOR" | "UNASSIGNED" => {
                Ok(Some(normalized.to_string()))
            }
            other => anyhow::bail!(
                "unsupported delivery unit noun '{}'; expected BRAN, HANG, EQUI, EQUIP, WALL, FLOOR, or UNASSIGNED",
                other
            ),
        }
    }

    fn row_to_mesh_asset(row: &duckdb::Row<'_>) -> duckdb::Result<ModelReleaseMeshAsset> {
        let bytes_value: Option<i64> = row.get(10)?;
        let absolute_path: Option<String> = row.get(8)?;
        Ok(ModelReleaseMeshAsset {
            release_id: row.get(0)?,
            project_name: row.get(1)?,
            dbnum: i64_to_u32(row.get(2)?, "dbnum")?,
            lod_tag: row.get(3)?,
            geo_hash: row.get(4)?,
            builtin: row.get(5)?,
            exists: row.get(6)?,
            mesh_relative_path: row.get(7)?,
            mesh_absolute_path: absolute_path.map(PathBuf::from),
            mesh_url: row.get(9)?,
            bytes: bytes_value
                .map(|value| i64_to_u64(value, "mesh asset bytes"))
                .transpose()?,
            sha256: row.get(11)?,
            glb_readable: row.get(12)?,
            glb_validation_error: row.get(13)?,
            indexed_at: row.get(14)?,
        })
    }

    fn validate_release_sidecar(
        release: &ModelReleaseRecord,
        path: &Path,
        problems: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read release sidecar failed: {}", path.display()))?;
        let sidecar: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("parse release sidecar failed: {}", path.display()))?;

        compare_sidecar_string(
            &sidecar,
            "schema_version",
            "model_release_sidecar:v1",
            problems,
        );
        compare_sidecar_string(&sidecar, "release_id", &release.release_id, problems);
        compare_sidecar_string(&sidecar, "project_name", &release.project_name, problems);
        compare_sidecar_string(&sidecar, "branch_id", &release.branch_id, problems);
        compare_sidecar_u64(&sidecar, "dbnum", u64::from(release.dbnum), problems);
        compare_sidecar_string(
            &sidecar,
            "release_lifecycle",
            release.release_lifecycle.as_str(),
            problems,
        );
        compare_sidecar_string(
            &sidecar,
            "release_quality",
            release.release_quality.as_str(),
            problems,
        );
        compare_sidecar_optional_string(
            &sidecar,
            "release_quality_reason",
            release.release_quality_reason.as_deref(),
            problems,
        );
        compare_sidecar_string(
            &sidecar,
            "release_status",
            release.release_status.as_str(),
            problems,
        );
        compare_sidecar_optional_string(
            &sidecar,
            "release_label",
            release.release_label.as_deref(),
            warnings,
        );
        compare_sidecar_string(
            &sidecar,
            "derivation_type",
            &release.derivation_type,
            problems,
        );
        compare_sidecar_optional_string(
            &sidecar,
            "generation_job_id",
            release.generation_job_id.as_deref(),
            problems,
        );
        let immutable_package_dir = release.immutable_package_dir.to_string_lossy();
        compare_sidecar_string(
            &sidecar,
            "immutable_package_dir",
            immutable_package_dir.as_ref(),
            problems,
        );
        let source_package_dir = release.source_package_dir.to_string_lossy();
        compare_sidecar_string(
            &sidecar,
            "source_package_dir",
            source_package_dir.as_ref(),
            problems,
        );
        compare_sidecar_string(&sidecar, "package_hash", &release.package_hash, problems);
        let source_manifest_path = release
            .source_manifest_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        compare_sidecar_optional_string(
            &sidecar,
            "source_manifest_path",
            source_manifest_path.as_deref(),
            problems,
        );
        compare_sidecar_optional_string(
            &sidecar,
            "source_manifest_hash",
            release.source_manifest_hash.as_deref(),
            problems,
        );
        verify_optional_evidence_file(
            "source_manifest",
            release.source_manifest_path.as_deref(),
            release.source_manifest_hash.as_deref(),
            problems,
        );
        verify_evidence_path_under(
            "source_manifest",
            release.source_manifest_path.as_deref(),
            &release.immutable_package_dir,
            problems,
        );
        let baseline_state_manifest_path = release
            .baseline_state_manifest_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        compare_sidecar_optional_string(
            &sidecar,
            "baseline_state_manifest_path",
            baseline_state_manifest_path.as_deref(),
            problems,
        );
        compare_sidecar_optional_string(
            &sidecar,
            "baseline_state_manifest_hash",
            release.baseline_state_manifest_hash.as_deref(),
            problems,
        );
        verify_optional_evidence_file(
            "baseline_state_manifest",
            release.baseline_state_manifest_path.as_deref(),
            release.baseline_state_manifest_hash.as_deref(),
            problems,
        );
        let asset_manifest_path = release
            .asset_manifest_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        compare_sidecar_optional_string(
            &sidecar,
            "asset_manifest_path",
            asset_manifest_path.as_deref(),
            problems,
        );
        compare_sidecar_optional_string(
            &sidecar,
            "asset_manifest_hash",
            release.asset_manifest_hash.as_deref(),
            problems,
        );
        verify_optional_evidence_file(
            "asset_manifest",
            release.asset_manifest_path.as_deref(),
            release.asset_manifest_hash.as_deref(),
            problems,
        );

        let sidecar_flags = sidecar_string_vec(&sidecar, "validation_flags");
        let expected_flags = release
            .validation_flags
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        if sidecar_flags != expected_flags {
            problems.push(format!(
                "release sidecar validation_flags mismatch: sidecar={:?} catalog={:?}",
                sidecar_flags, expected_flags
            ));
        }

        let rows = sidecar
            .get("rows_by_table")
            .and_then(|value| value.as_object());
        for (table, expected) in &release.rows_by_table {
            let actual = rows
                .and_then(|rows| rows.get(table))
                .and_then(|value| value.as_u64());
            if actual != Some(*expected) {
                problems.push(format!(
                    "release sidecar rows_by_table.{} mismatch: sidecar={:?} catalog={}",
                    table, actual, expected
                ));
            }
        }

        Ok(())
    }

    fn missing_required_release_files(
        release: &ModelReleaseRecord,
        files: &[ModelReleaseFile],
    ) -> Vec<String> {
        files
            .iter()
            .filter(|file| file.required)
            .filter_map(|file| match release_catalog_file_path(release, file) {
                Ok(path) if path.is_file() => None,
                _ => Some(file.relative_path.clone()),
            })
            .collect()
    }

    fn validate_release_file_catalog(
        release: &ModelReleaseRecord,
        files: &[ModelReleaseFile],
        problems: &mut Vec<String>,
    ) {
        match crate::version_management::hashing::package_hash(files) {
            Ok(package_hash) if package_hash.eq_ignore_ascii_case(&release.package_hash) => {}
            Ok(package_hash) => problems.push(format!(
                "release file catalog package_hash mismatch: catalog_files={} release={}",
                package_hash, release.package_hash
            )),
            Err(error) => problems.push(format!("release file catalog hash failed: {error}")),
        }

        for file in files {
            let path = match release_catalog_file_path(release, file) {
                Ok(path) => path,
                Err(error) => {
                    problems.push(error);
                    continue;
                }
            };
            if !path.is_file() {
                if file.required {
                    problems.push(format!(
                        "required release file is missing: {}",
                        path.display()
                    ));
                }
                continue;
            }
            match fs::metadata(&path) {
                Ok(metadata) if metadata.len() == file.bytes => {}
                Ok(metadata) => problems.push(format!(
                    "release file bytes mismatch: logical_name={} path={} catalog={} actual={}",
                    file.logical_name,
                    path.display(),
                    file.bytes,
                    metadata.len()
                )),
                Err(error) => {
                    problems.push(format!(
                        "release file metadata failed: {}: {error}",
                        path.display()
                    ));
                    continue;
                }
            }
            match crate::version_management::hashing::sha256_file(&path) {
                Ok(actual_hash) if actual_hash.eq_ignore_ascii_case(&file.sha256) => {}
                Ok(actual_hash) => problems.push(format!(
                    "release file sha256 mismatch: logical_name={} path={} catalog={} actual={}",
                    file.logical_name,
                    path.display(),
                    file.sha256,
                    actual_hash
                )),
                Err(error) => problems.push(format!(
                    "release file sha256 check failed: {}: {error}",
                    path.display()
                )),
            }
        }
    }

    fn release_catalog_file_path(
        release: &ModelReleaseRecord,
        file: &ModelReleaseFile,
    ) -> Result<PathBuf, String> {
        let relative_path = Path::new(&file.relative_path);
        if relative_path.as_os_str().is_empty() || relative_path.is_absolute() {
            return Err(format!(
                "release file catalog has unsafe relative path: {}",
                file.relative_path
            ));
        }
        if relative_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
                    | std::path::Component::ParentDir
            )
        }) {
            return Err(format!(
                "release file catalog has unsafe relative path: {}",
                file.relative_path
            ));
        }
        Ok(release.immutable_package_dir.join(relative_path))
    }

    fn verify_optional_evidence_file(
        label: &str,
        path: Option<&Path>,
        expected_hash: Option<&str>,
        problems: &mut Vec<String>,
    ) {
        match (path, expected_hash) {
            (Some(path), Some(expected_hash)) => {
                if !path.is_file() {
                    problems.push(format!(
                        "release evidence {label} is missing: {}",
                        path.display()
                    ));
                    return;
                }
                match crate::version_management::hashing::sha256_file(path) {
                    Ok(actual_hash) if actual_hash.eq_ignore_ascii_case(expected_hash) => {}
                    Ok(actual_hash) => problems.push(format!(
                        "release evidence {label} hash mismatch: file={} catalog={}",
                        actual_hash, expected_hash
                    )),
                    Err(error) => problems.push(format!(
                        "release evidence {label} hash check failed: {}: {error}",
                        path.display()
                    )),
                }
            }
            (Some(path), None) => problems.push(format!(
                "release evidence {label} has path but no hash: {}",
                path.display()
            )),
            (None, Some(hash)) => problems.push(format!(
                "release evidence {label} has hash but no path: {hash}"
            )),
            (None, None) => {}
        }
    }

    fn verify_evidence_path_under(
        label: &str,
        path: Option<&Path>,
        root: &Path,
        problems: &mut Vec<String>,
    ) {
        let Some(path) = path else {
            return;
        };
        if !path_is_equal_or_nested(path, root) {
            problems.push(format!(
                "release evidence {label} is not release-local: file={} root={}",
                path.display(),
                root.display()
            ));
        }
    }

    fn path_is_equal_or_nested(path: &Path, root: &Path) -> bool {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path_abs = canonical_or_absolute(path, &cwd);
        let root_abs = canonical_or_absolute(root, &cwd);
        path_abs.starts_with(root_abs)
    }

    fn path_is_equal(left: &Path, right: &Path) -> bool {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        canonical_or_absolute(left, &cwd) == canonical_or_absolute(right, &cwd)
    }

    fn canonical_or_absolute(path: &Path, cwd: &Path) -> PathBuf {
        path.canonicalize()
            .unwrap_or_else(|_| absolute_path(path, cwd))
    }

    fn compare_sidecar_string(
        sidecar: &serde_json::Value,
        field: &str,
        expected: &str,
        problems: &mut Vec<String>,
    ) {
        let actual = sidecar.get(field).and_then(|value| value.as_str());
        if actual != Some(expected) {
            problems.push(format!(
                "release sidecar {field} mismatch: sidecar={:?} catalog={expected}",
                actual
            ));
        }
    }

    fn compare_sidecar_optional_string(
        sidecar: &serde_json::Value,
        field: &str,
        expected: Option<&str>,
        problems: &mut Vec<String>,
    ) {
        let actual = sidecar
            .get(field)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if actual != expected {
            problems.push(format!(
                "release sidecar {field} mismatch: sidecar={:?} catalog={:?}",
                actual, expected
            ));
        }
    }

    fn compare_sidecar_u64(
        sidecar: &serde_json::Value,
        field: &str,
        expected: u64,
        problems: &mut Vec<String>,
    ) {
        let actual = sidecar.get(field).and_then(|value| value.as_u64());
        if actual != Some(expected) {
            problems.push(format!(
                "release sidecar {field} mismatch: sidecar={:?} catalog={expected}",
                actual
            ));
        }
    }

    fn sidecar_string_vec(sidecar: &serde_json::Value, field: &str) -> BTreeSet<String> {
        sidecar
            .get(field)
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    fn read_release_manifest_json(
        release: &ModelReleaseRecord,
    ) -> anyhow::Result<serde_json::Value> {
        let path = release.immutable_package_dir.join("manifest.json");
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read release manifest failed: {}", path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("parse release manifest failed: {}", path.display()))
    }

    fn json_u64_at(value: &serde_json::Value, path: &[&str]) -> Option<u64> {
        let mut current = value;
        for key in path {
            current = current.get(*key)?;
        }
        current.as_u64()
    }

    fn manifest_mesh_lod_tag(manifest: &serde_json::Value) -> String {
        manifest
            .get("mesh_validation")
            .and_then(|value| value.get("lod_tag"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("L1")
            .to_string()
    }

    fn read_unique_geo_hashes(path: &Path) -> anyhow::Result<Vec<String>> {
        use arrow_array::{Array, RecordBatch, StringArray};
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let file = fs::File::open(path)
            .with_context(|| format!("open geo_instances parquet failed: {}", path.display()))?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .with_context(|| {
                format!(
                    "read geo_instances parquet metadata failed: {}",
                    path.display()
                )
            })?
            .with_batch_size(8192)
            .build()
            .with_context(|| {
                format!(
                    "create geo_instances parquet reader failed: {}",
                    path.display()
                )
            })?;
        let mut values = BTreeSet::new();
        for batch in reader {
            let batch = batch
                .with_context(|| format!("read geo_instances batch failed: {}", path.display()))?;
            let column = string_column(&batch, "geo_hash", path)?;
            for row in 0..batch.num_rows() {
                if column.is_null(row) {
                    continue;
                }
                let value = column.value(row).trim();
                if !value.is_empty() {
                    values.insert(value.to_string());
                }
            }
        }
        Ok(values.into_iter().collect())
    }

    fn string_column<'a>(
        batch: &'a arrow_array::RecordBatch,
        column: &str,
        path: &Path,
    ) -> anyhow::Result<&'a arrow_array::StringArray> {
        let index = batch
            .schema()
            .fields()
            .iter()
            .position(|field| field.name() == column)
            .with_context(|| format!("{} is missing column '{}'", path.display(), column))?;
        batch
            .column(index)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .with_context(|| format!("{} column '{}' is not String", path.display(), column))
    }

    fn is_builtin_geo_hash(geo_hash: &str) -> bool {
        matches!(geo_hash.trim(), "0" | "1" | "2" | "3")
    }

    fn find_mesh_asset(mesh_root: &Path, lod_tag: &str, geo_hash: &str) -> Option<PathBuf> {
        mesh_candidates(mesh_root, lod_tag, geo_hash)
            .into_iter()
            .find(|path| path.is_file())
    }

    fn mesh_candidates(mesh_root: &Path, lod_tag: &str, geo_hash: &str) -> [PathBuf; 3] {
        let lod_dir = mesh_root.join(format!("lod_{lod_tag}"));
        [
            lod_dir.join(format!("{geo_hash}_{lod_tag}.glb")),
            lod_dir.join(format!("{geo_hash}.glb")),
            mesh_root.join(format!("{geo_hash}.glb")),
        ]
    }

    fn release_root_dir(release: &ModelReleaseRecord) -> anyhow::Result<PathBuf> {
        let parquet_dir = release.immutable_package_dir.parent().with_context(|| {
            format!(
                "release immutable package dir has no parquet parent: {}",
                release.immutable_package_dir.display()
            )
        })?;
        let release_root = parquet_dir.parent().with_context(|| {
            format!(
                "release immutable package dir has no release root: {}",
                release.immutable_package_dir.display()
            )
        })?;
        Ok(release_root.to_path_buf())
    }

    fn release_mesh_asset_path(
        release: &ModelReleaseRecord,
        lod_tag: &str,
        geo_hash: &str,
    ) -> anyhow::Result<PathBuf> {
        ensure_safe_mesh_segment(lod_tag, "lod_tag")?;
        ensure_safe_mesh_segment(geo_hash, "geo_hash")?;
        Ok(release_root_dir(release)?
            .join("meshes")
            .join(format!("lod_{lod_tag}"))
            .join(format!("{geo_hash}_{lod_tag}.glb")))
    }

    fn ensure_safe_mesh_segment(value: &str, label: &str) -> anyhow::Result<()> {
        let value = value.trim();
        if value.is_empty() || value == "." || value == ".." {
            anyhow::bail!("{label} is not a safe mesh path segment: '{value}'");
        }
        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            anyhow::bail!("{label} contains unsafe mesh path characters: '{value}'");
        }
        Ok(())
    }

    fn materialize_release_mesh_asset(
        release: &ModelReleaseRecord,
        lod_tag: &str,
        geo_hash: &str,
        source_path: &Path,
        expected_sha256: &str,
    ) -> anyhow::Result<PathBuf> {
        let dest = release_mesh_asset_path(release, lod_tag, geo_hash)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create release mesh asset dir failed: {}", parent.display())
            })?;
        }

        if dest.is_file() {
            verify_existing_mesh_asset(&dest, expected_sha256)?;
            return Ok(dest);
        }

        let tmp = temp_mesh_asset_path(&dest);
        if tmp.exists() {
            let _ = fs::remove_file(&tmp);
        }
        fs::copy(source_path, &tmp).with_context(|| {
            format!(
                "copy mesh asset into staging file failed: {} -> {}",
                source_path.display(),
                tmp.display()
            )
        })?;
        let staged_sha256 = crate::version_management::hashing::sha256_file(&tmp)?;
        if staged_sha256 != expected_sha256 {
            let _ = fs::remove_file(&tmp);
            anyhow::bail!(
                "staged mesh asset hash mismatch for {}: expected {}, got {}",
                dest.display(),
                expected_sha256,
                staged_sha256
            );
        }

        match fs::rename(&tmp, &dest) {
            Ok(()) => {}
            Err(error) if dest.is_file() => {
                let _ = fs::remove_file(&tmp);
                verify_existing_mesh_asset(&dest, expected_sha256).with_context(|| {
                    format!("release mesh asset appeared during rename after error: {error}")
                })?;
            }
            Err(error) => {
                let _ = fs::remove_file(&tmp);
                return Err(error).with_context(|| {
                    format!(
                        "publish release mesh asset failed: {} -> {}",
                        tmp.display(),
                        dest.display()
                    )
                });
            }
        }

        verify_existing_mesh_asset(&dest, expected_sha256)?;
        Ok(dest)
    }

    fn verify_existing_mesh_asset(path: &Path, expected_sha256: &str) -> anyhow::Result<()> {
        let existing_sha256 = crate::version_management::hashing::sha256_file(path)?;
        if existing_sha256 != expected_sha256 {
            anyhow::bail!(
                "release mesh asset already exists with different content: {} expected {} got {}",
                path.display(),
                expected_sha256,
                existing_sha256
            );
        }
        Ok(())
    }

    fn validate_glb_asset_readable(path: &Path) -> (bool, Option<String>) {
        let path = path.to_path_buf();
        match std::panic::catch_unwind(|| validate_glb_asset_readable_inner(&path)) {
            Ok(Ok(())) => (true, None),
            Ok(Err(error)) => (false, Some(compact_validation_error(format!("{error:#}")))),
            Err(_) => (
                false,
                Some("GLB parser panicked while validating asset".to_string()),
            ),
        }
    }

    fn validate_glb_asset_readable_inner(path: &Path) -> anyhow::Result<()> {
        let (document, _buffers, _) = gltf::import(path).with_context(|| {
            format!(
                "import GLB for readability validation failed: {}",
                path.display()
            )
        })?;
        let mut primitive_count = 0usize;
        let mut position_count = 0usize;
        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                primitive_count += 1;
                let Some(position_accessor) = primitive.get(&gltf::Semantic::Positions) else {
                    anyhow::bail!(
                        "GLB primitive is missing POSITION accessor: {}",
                        path.display()
                    );
                };
                let count = position_accessor.count();
                if count == 0 {
                    anyhow::bail!("GLB POSITION accessor is empty: {}", path.display());
                }
                position_count += count;
            }
        }
        if primitive_count == 0 {
            anyhow::bail!("GLB has no mesh primitives: {}", path.display());
        }
        if position_count == 0 {
            anyhow::bail!("GLB has no positions: {}", path.display());
        }
        Ok(())
    }

    fn compact_validation_error(message: String) -> String {
        let single_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
        const MAX_LEN: usize = 500;
        if single_line.chars().count() <= MAX_LEN {
            return single_line;
        }
        let mut truncated = single_line.chars().take(MAX_LEN).collect::<String>();
        truncated.push_str("...");
        truncated
    }

    fn temp_mesh_asset_path(dest: &Path) -> PathBuf {
        let file_name = dest
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "mesh_asset.glb".into());
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        dest.with_file_name(format!(".{file_name}.tmp-{}-{suffix}", std::process::id()))
    }

    fn release_mesh_url(release: &ModelReleaseRecord, relative_path: &str) -> Option<String> {
        let base = release_static_base_url(release);
        mesh_url_for_relative(&base, relative_path)
    }

    fn release_static_base_url(release: &ModelReleaseRecord) -> String {
        format!(
            "/files/output/{}/model_versions/releases/{}",
            urlencoding::encode(&release.project_name),
            urlencoding::encode(&release.release_id)
        )
    }

    fn mesh_relative_path(mesh_root: &Path, mesh_path: &Path) -> String {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let abs_root = absolute_path(mesh_root, &cwd);
        let abs_mesh = absolute_path(mesh_path, &cwd);
        abs_mesh
            .strip_prefix(&abs_root)
            .unwrap_or(&abs_mesh)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn absolute_path(path: &Path, cwd: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        }
    }

    fn mesh_url_for_relative(base_url: &str, relative_path: &str) -> Option<String> {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() || relative_path.trim().is_empty() {
            return None;
        }
        let mut segments = Vec::new();
        for segment in relative_path.replace('\\', "/").split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                return None;
            }
            segments.push(urlencoding::encode(segment).to_string());
        }
        Some(format!("{}/{}", base, segments.join("/")))
    }

    fn mesh_asset_index_hash(assets: &[ModelReleaseMeshAsset]) -> anyhow::Result<String> {
        let mut rows = assets
            .iter()
            .map(|asset| {
                serde_json::json!({
                    "lod_tag": asset.lod_tag,
                    "geo_hash": asset.geo_hash,
                    "builtin": asset.builtin,
                    "exists": asset.exists,
                    "mesh_relative_path": asset.mesh_relative_path,
                    "bytes": asset.bytes,
                    "sha256": asset.sha256,
                    "glb_readable": asset.glb_readable,
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            let a_key = a
                .get("geo_hash")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let b_key = b
                .get("geo_hash")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            a_key.cmp(b_key)
        });
        let bytes = serde_json::to_vec(&rows).context("serialize mesh asset index hash payload")?;
        Ok(crate::version_management::hashing::sha256_bytes(&bytes))
    }

    fn write_mesh_asset_manifest(
        stats: &ModelReleaseMeshAssetIndexStats,
        assets: &[ModelReleaseMeshAsset],
    ) -> anyhow::Result<()> {
        if let Some(parent) = stats.manifest_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "create mesh asset manifest parent failed: {}",
                    parent.display()
                )
            })?;
        }
        let manifest = serde_json::json!({
            "version": 1,
            "stats": stats,
            "assets": assets,
        });
        let bytes =
            serde_json::to_vec_pretty(&manifest).context("serialize mesh asset manifest")?;
        fs::write(&stats.manifest_path, bytes).with_context(|| {
            format!(
                "write mesh asset manifest failed: {}",
                stats.manifest_path.display()
            )
        })?;
        Ok(())
    }

    fn read_matrix(row: &duckdb::Row<'_>, start: usize) -> duckdb::Result<Option<Vec<f64>>> {
        let first: Option<f64> = row.get(start)?;
        let Some(first) = first else {
            return Ok(None);
        };
        let mut values = Vec::with_capacity(16);
        values.push(first);
        for offset in 1..16 {
            let value: Option<f64> = row.get(start + offset)?;
            values.push(value.unwrap_or(0.0));
        }
        Ok(Some(values))
    }

    fn read_aabb(
        row: &duckdb::Row<'_>,
        start: usize,
    ) -> duckdb::Result<Option<ModelReleaseSceneAabb>> {
        let min_x: Option<f64> = row.get(start)?;
        let Some(min_x) = min_x else {
            return Ok(None);
        };
        Ok(Some(ModelReleaseSceneAabb {
            min: [
                min_x,
                row.get::<_, Option<f64>>(start + 1)?.unwrap_or(0.0),
                row.get::<_, Option<f64>>(start + 2)?.unwrap_or(0.0),
            ],
            max: [
                row.get::<_, Option<f64>>(start + 3)?.unwrap_or(0.0),
                row.get::<_, Option<f64>>(start + 4)?.unwrap_or(0.0),
                row.get::<_, Option<f64>>(start + 5)?.unwrap_or(0.0),
            ],
        }))
    }

    fn read_mesh_asset_evidence(
        row: &duckdb::Row<'_>,
        start: usize,
        geo_hash: &str,
    ) -> duckdb::Result<Option<ModelReleaseSceneMeshAssetEvidence>> {
        let builtin: Option<bool> = row.get(start)?;
        let Some(builtin) = builtin else {
            return Ok(None);
        };
        let exists = row.get::<_, Option<bool>>(start + 1)?.unwrap_or(false);
        let mesh_relative_path = clean_string(row.get(start + 2)?);
        let mesh_absolute_path = clean_string(row.get(start + 3)?).map(PathBuf::from);
        let mesh_url = clean_string(row.get(start + 4)?);
        let bytes = opt_i64_to_u64(row.get(start + 5)?, "mesh asset bytes")?;
        let sha256 = clean_string(row.get(start + 6)?);
        let glb_readable: Option<bool> = row.get(start + 7)?;
        let glb_validation_error = clean_string(row.get(start + 8)?);

        Ok(Some(ModelReleaseSceneMeshAssetEvidence {
            geo_hash: geo_hash.to_string(),
            builtin,
            exists,
            mesh_relative_path,
            mesh_absolute_path,
            mesh_url,
            bytes,
            sha256,
            glb_readable,
            glb_validation_error,
        }))
    }

    fn ensure_file_exists(path: &Path, label: &str) -> anyhow::Result<()> {
        if !path.exists() {
            anyhow::bail!(
                "{} is missing for component indexing: {}",
                label,
                path.display()
            );
        }
        if !path.is_file() {
            anyhow::bail!(
                "{} path is not a file for component indexing: {}",
                label,
                path.display()
            );
        }
        Ok(())
    }

    struct MetadataFileLock {
        path: PathBuf,
    }

    impl MetadataFileLock {
        fn acquire(metadata_path: &Path) -> anyhow::Result<Self> {
            let path = metadata_lock_path(metadata_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("create metadata lock parent failed: {}", parent.display())
                })?;
            }

            let deadline = Instant::now() + METADATA_LOCK_TIMEOUT;
            loop {
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(mut file) => {
                        writeln!(
                            file,
                            "pid={}\ncreated_at={}",
                            std::process::id(),
                            Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
                        )
                        .with_context(|| {
                            format!("write metadata lock file failed: {}", path.display())
                        })?;
                        return Ok(Self { path });
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        if remove_stale_lock_if_needed(&path)? {
                            continue;
                        }
                        if Instant::now() >= deadline {
                            anyhow::bail!(
                                "timed out waiting for model-version DuckLake metadata lock: {}",
                                path.display()
                            );
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!("create metadata lock file failed: {}", path.display())
                        });
                    }
                }
            }
        }
    }

    impl Drop for MetadataFileLock {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn metadata_lock_path(metadata_path: &Path) -> PathBuf {
        let file_name = metadata_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("metadata.ducklake");
        metadata_path.with_file_name(format!("{}.lock", file_name))
    }

    fn remove_stale_lock_if_needed(path: &Path) -> anyhow::Result<bool> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("read metadata lock file failed: {}", path.display()))?;
        let modified = metadata.modified().unwrap_or(SystemTime::now());
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        if age < METADATA_LOCK_STALE_AFTER {
            return Ok(false);
        }
        fs::remove_file(path)
            .with_context(|| format!("remove stale metadata lock failed: {}", path.display()))?;
        Ok(true)
    }

    fn opt_u64_to_i64(value: Option<u64>, label: &str) -> anyhow::Result<Option<i64>> {
        value.map(|value| u64_to_i64(value, label)).transpose()
    }

    fn u64_to_i64(value: u64, label: &str) -> anyhow::Result<i64> {
        i64::try_from(value).with_context(|| format!("{} does not fit in i64: {}", label, value))
    }

    fn i64_to_u64(value: i64, label: &str) -> duckdb::Result<u64> {
        u64::try_from(value).map_err(|_| {
            duckdb::Error::FromSqlConversionFailure(
                0,
                duckdb::types::Type::BigInt,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is negative: {}", label, value),
                )),
            )
        })
    }

    fn opt_i64_to_u64(value: Option<i64>, label: &str) -> duckdb::Result<Option<u64>> {
        value.map(|value| i64_to_u64(value, label)).transpose()
    }

    fn i64_to_u32(value: i64, label: &str) -> duckdb::Result<u32> {
        u32::try_from(value).map_err(|_| {
            duckdb::Error::FromSqlConversionFailure(
                0,
                duckdb::types::Type::Int,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is out of u32 range: {}", label, value),
                )),
            )
        })
    }

    pub use ModelVersionDuckLakeStore as Store;
}

#[cfg(not(feature = "model-version-ducklake"))]
mod imp {
    use super::*;

    pub struct ModelVersionDuckLakeStore;

    impl ModelVersionDuckLakeStore {
        pub fn open(_cfg: ModelVersionDuckLakeConfig) -> anyhow::Result<Self> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`; \
                 rebuild with --features model-version-ducklake"
            )
        }

        pub fn open_writer(cfg: ModelVersionDuckLakeConfig) -> anyhow::Result<Self> {
            Self::open(cfg)
        }

        pub fn open_readonly(cfg: ModelVersionDuckLakeConfig) -> anyhow::Result<Self> {
            Self::open(cfg)
        }

        pub fn register_release(
            &self,
            _release: &ModelReleaseRecord,
            _files: &[ModelReleaseFile],
            _parent_release_id: Option<&str>,
            _manifest_json: &serde_json::Value,
            _extra_metadata: &serde_json::Value,
        ) -> anyhow::Result<ModelReleaseRegistration> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn list_releases(
            &self,
            _project_name: Option<&str>,
        ) -> anyhow::Result<ModelReleaseListResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn update_release_status(
            &self,
            _release_id: &str,
            _status: ModelReleaseStatus,
            _reason: Option<&str>,
        ) -> anyhow::Result<()> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn release_events(
            &self,
            _release_id: &str,
        ) -> anyhow::Result<ModelReleaseEventsResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn reconcile_release(
            &self,
            _release_id: &str,
            _publish_if_complete: bool,
            _fail_if_unusable: bool,
        ) -> anyhow::Result<ModelReleaseReconcileReport> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn repair_release_source_manifest_to_package(
            &self,
            _release_id: &str,
        ) -> anyhow::Result<Option<ModelReleaseRecord>> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn get_release(&self, _release_id: &str) -> anyhow::Result<ModelReleaseRecord> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn annotate_release_quality(
            &self,
            _release_id: &str,
            _release_quality: Option<ModelReleaseQuality>,
            _release_quality_reason: Option<&str>,
            _validation_flags: &[String],
            _spec_info_fallback_count: Option<u64>,
        ) -> anyhow::Result<ModelReleaseRecord> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn catalog_migration_report(
            &self,
            _project_name: &str,
        ) -> anyhow::Result<ModelVersionCatalogMigrationReport> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn index_release_components(
            &self,
            _release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelComponentSnapshotStats> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn ensure_release_components_indexed(
            &self,
            _release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelComponentSnapshotStats> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn diff_releases(
            &self,
            _from_release_id: &str,
            _to_release_id: &str,
            _limit: usize,
            _change_type_filter: Option<&str>,
        ) -> anyhow::Result<ModelComponentDiffResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn compare_readiness(
            &self,
            _from_release_id: &str,
            _to_release_id: &str,
        ) -> anyhow::Result<ModelReleasePairReadinessResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn index_release_units(
            &self,
            _release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelUnitIndexStats> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn ensure_release_units_indexed(
            &self,
            _release: &ModelReleaseRecord,
        ) -> anyhow::Result<ModelUnitIndexStats> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn diff_units(
            &self,
            _from_release_id: &str,
            _to_release_id: &str,
            _limit: usize,
            _unit_noun_filter: Option<&str>,
        ) -> anyhow::Result<ModelUnitDiffResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn component_unit_impacts(
            &self,
            _from_release_id: &str,
            _to_release_id: &str,
            _limit: usize,
            _component_key_filter: Option<&str>,
        ) -> anyhow::Result<ModelComponentUnitImpactResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn index_release_mesh_assets(
            &self,
            _release: &ModelReleaseRecord,
            _mesh_root: &std::path::Path,
            _mesh_base_url: Option<&str>,
            _materialize: bool,
        ) -> anyhow::Result<ModelReleaseMeshAssetIndexStats> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn get_release_mesh_assets(
            &self,
            _release_id: &str,
            _limit: usize,
            _missing_only: bool,
        ) -> anyhow::Result<ModelReleaseMeshAssetIndexResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }

        pub fn release_scene(
            &self,
            _release_id: &str,
            _limit: usize,
            _offset: usize,
            _component_key: Option<&str>,
        ) -> anyhow::Result<ModelReleaseSceneResponse> {
            anyhow::bail!(
                "model-version DuckLake commands require feature `model-version-ducklake`"
            )
        }
    }

    pub use ModelVersionDuckLakeStore as Store;
}

pub use imp::Store as ModelVersionDuckLakeStore;
