use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use aios_core::RefnoEnum;
use duckdb::{Connection, OptionalExt, params};
use serde::{Deserialize, Serialize};

use crate::generation_read::{
    AttributeSet, DataVersion, ElementSnapshot, HierarchyRow, InputVersionManifest,
    TransformSnapshot, encode_attribute_set_payload, hash_serializable,
};

use super::parse_staging::SealedParseStage;
use super::schema::{
    CREATE_SCHEMA_SQL, DUCKLAKE_CATALOG_ALIAS, MIGRATE_V1_TO_V2_SQL, PARTITION_SCHEMA_SQL,
    VERSION_STORE_SCHEMA_VERSION,
};

const PARSE_STAGE_CATALOG_ALIAS: &str = "parse_stage_commit";

#[derive(Debug, Clone)]
pub struct DuckLakeExtensionConfig {
    pub ducklake_extension: PathBuf,
    pub sqlite_extension: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DuckLakeConfig {
    pub metadata_catalog: PathBuf,
    pub data_path: PathBuf,
    pub temp_directory: PathBuf,
    pub memory_limit: String,
    pub threads: usize,
    pub extensions: DuckLakeExtensionConfig,
}

impl DuckLakeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.memory_limit.trim().is_empty(),
            "DuckDB memory_limit 不能为空"
        );
        anyhow::ensure!(self.threads > 0, "DuckDB threads 必须大于 0");
        anyhow::ensure!(
            self.extensions.ducklake_extension.is_file(),
            "缺少离线 ducklake extension: {}",
            self.extensions.ducklake_extension.display()
        );
        anyhow::ensure!(
            self.extensions.sqlite_extension.is_file(),
            "SQLite catalog 需要离线 sqlite_scanner extension: {}",
            self.extensions.sqlite_extension.display()
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityDbVersion {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source: String,
    pub commit_fingerprint: String,
    pub source_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionStoreElement {
    pub element: ElementSnapshot,
    pub attributes: AttributeSet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbCatalogEntry {
    pub dbnum: u32,
    pub ref0: Option<u32>,
    pub db_type: String,
    pub project: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityCommit {
    pub global_fingerprint: String,
    pub db_versions: Vec<AuthorityDbVersion>,
    /// 完整替换这些 dbnum 的当前事实；DuckLake snapshot 仍保留替换前历史。
    #[serde(default)]
    pub replace_dbnums: BTreeSet<u32>,
    pub upsert_elements: Vec<VersionStoreElement>,
    pub delete_refnos: BTreeMap<u32, Vec<RefnoEnum>>,
    pub hierarchy_rows: Vec<HierarchyRow>,
    pub transforms: Vec<TransformSnapshot>,
    pub db_catalog: Vec<DbCatalogEntry>,
    pub bootstrap_current_state: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityCommitOutcome {
    pub snapshot_id: u64,
    pub manifest: InputVersionManifest,
    pub idempotent: bool,
}

#[derive(Clone)]
pub struct DuckLakeAuthority {
    config: Arc<DuckLakeConfig>,
    connection: Arc<Mutex<Connection>>,
}

impl DuckLakeAuthority {
    pub fn open(mut config: DuckLakeConfig) -> anyhow::Result<Self> {
        config.data_path = absolute_path(&config.data_path)?;
        config.validate()?;
        create_parent(&config.metadata_catalog)?;
        std::fs::create_dir_all(&config.data_path)?;
        std::fs::create_dir_all(&config.temp_directory)?;

        let connection = open_connection(&config, None)?;
        let authority = Self {
            config: Arc::new(config),
            connection: Arc::new(Mutex::new(connection)),
        };
        authority.ensure_schema()?;
        Ok(authority)
    }

    pub fn open_readonly(mut config: DuckLakeConfig) -> anyhow::Result<Self> {
        config.data_path = absolute_path(&config.data_path)?;
        config.validate()?;
        anyhow::ensure!(
            config.metadata_catalog.is_file(),
            "DuckLake metadata catalog 不存在: {}",
            config.metadata_catalog.display()
        );
        let connection = open_connection_readonly(&config, None)?;
        let current: Option<String> = connection
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'schema_version' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let expected_schema = VERSION_STORE_SCHEMA_VERSION.to_string();
        anyhow::ensure!(
            current.as_deref() == Some(expected_schema.as_str()),
            "DuckLake version store schema 未就绪: current={current:?} expected={VERSION_STORE_SCHEMA_VERSION}"
        );
        Ok(Self {
            config: Arc::new(config),
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn config(&self) -> &DuckLakeConfig {
        &self.config
    }

    pub fn commit(&self, commit: AuthorityCommit) -> anyhow::Result<AuthorityCommitOutcome> {
        validate_commit(&commit)?;
        let extra_info = commit_extra_info(&commit.global_fingerprint);

        let existing_matches = self.resolve_all_snapshots_by_extra_info(&extra_info)?;
        anyhow::ensure!(
            existing_matches.len() <= 1,
            "提交指纹匹配多个 snapshot: fingerprint={} matches={existing_matches:?}",
            commit.global_fingerprint
        );
        if let Some(snapshot_id) = existing_matches.first().copied() {
            let manifest = self.read_manifest(snapshot_id)?;
            return Ok(AuthorityCommitOutcome {
                snapshot_id,
                manifest,
                idempotent: true,
            });
        }

        {
            let mut connection = self.lock_connection()?;
            connection.execute_batch("BEGIN TRANSACTION;")?;
            let result = apply_commit(&mut connection, &commit, &extra_info);
            match result {
                Ok(()) => connection.execute_batch("COMMIT;")?,
                Err(error) => {
                    let _ = connection.execute_batch("ROLLBACK;");
                    return Err(error);
                }
            }
        }

        let matches = self.resolve_all_snapshots_by_extra_info(&extra_info)?;
        anyhow::ensure!(
            matches.len() == 1,
            "提交指纹必须唯一解析一个 snapshot: fingerprint={} matches={matches:?}",
            commit.global_fingerprint
        );
        let snapshot_id = matches[0];
        let manifest = self.read_manifest(snapshot_id)?;
        Ok(AuthorityCommitOutcome {
            snapshot_id,
            manifest,
            idempotent: false,
        })
    }

    /// 将单个 dbnum 的 sealed parse stage 原子发布为新的 DuckLake snapshot。
    ///
    /// stage 以只读 catalog attach；目标 dbnum 的事实整体替换，其他 dbnum 的
    /// `data_version_state` 与 manifest 项保持不变。同一 stage fingerprint 重试时
    /// 返回首次提交的 snapshot，不重复发布。
    pub fn commit_staged_db(
        &self,
        stage: &SealedParseStage,
    ) -> anyhow::Result<AuthorityCommitOutcome> {
        stage.verify()?;
        let global_fingerprint =
            hash_serializable(&("parse-stage-authority-v1", &stage.fingerprint));
        let extra_info = commit_extra_info(&global_fingerprint);
        let existing_matches = self.resolve_all_snapshots_by_extra_info(&extra_info)?;
        anyhow::ensure!(
            existing_matches.len() <= 1,
            "parse stage 指纹匹配多个 snapshot: stage={} matches={existing_matches:?}",
            stage.fingerprint
        );
        if let Some(snapshot_id) = existing_matches.first().copied() {
            if let Some(bound_snapshot) = stage.authority_snapshot_id {
                anyhow::ensure!(
                    bound_snapshot == snapshot_id,
                    "parse stage authority binding 不一致: stage={bound_snapshot} actual={snapshot_id}"
                );
            }
            return self.validate_staged_outcome(
                stage,
                &global_fingerprint,
                snapshot_id,
                true,
                None,
            );
        }
        anyhow::ensure!(
            stage.authority_snapshot_id.is_none(),
            "parse stage 已绑定 snapshot={}，但 DuckLake 中无法按 fingerprint 解析",
            stage.authority_snapshot_id.unwrap_or_default()
        );

        let before_versions;
        {
            let mut connection = self.lock_connection()?;
            before_versions = read_data_versions(&connection)?;
            attach_parse_stage(&connection, &stage.path)?;
            connection.execute_batch("BEGIN TRANSACTION;")?;
            let result =
                apply_staged_commit(&mut connection, stage, &global_fingerprint, &extra_info);
            match result {
                Ok(()) => {
                    if let Err(error) = connection.execute_batch("COMMIT;") {
                        let _ = connection.execute_batch("ROLLBACK;");
                        let _ = detach_parse_stage(&connection);
                        return Err(error.into());
                    }
                }
                Err(error) => {
                    let _ = connection.execute_batch("ROLLBACK;");
                    let _ = detach_parse_stage(&connection);
                    return Err(error);
                }
            }
            detach_parse_stage(&connection)?;
        }

        let matches = self.resolve_all_snapshots_by_extra_info(&extra_info)?;
        anyhow::ensure!(
            matches.len() == 1,
            "parse stage 提交必须唯一解析一个 snapshot: stage={} matches={matches:?}",
            stage.fingerprint
        );
        self.validate_staged_outcome(
            stage,
            &global_fingerprint,
            matches[0],
            false,
            Some(before_versions),
        )
    }

    pub fn read_manifest(&self, snapshot_id: u64) -> anyhow::Result<InputVersionManifest> {
        let connection = open_connection(&self.config, Some(snapshot_id))?;
        let mut statement = connection.prepare(
            "SELECT dbnum, sesno, commit_fingerprint \
             FROM data_version_state ORDER BY dbnum",
        )?;
        let versions = statement
            .query_map([], |row| {
                Ok(DataVersion {
                    dbnum: row.get(0)?,
                    sesno: row.get(1)?,
                    commit_fingerprint: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let history_start_snapshot =
            read_history_start_snapshot(&connection)?.unwrap_or(snapshot_id);
        InputVersionManifest::new(snapshot_id, history_start_snapshot, versions)
            .map_err(anyhow::Error::from)
    }

    pub fn committed_watermark(&self, dbnum: u32) -> anyhow::Result<u32> {
        let connection = self.lock_connection()?;
        Ok(connection
            .query_row(
                "SELECT sesno FROM data_version_state WHERE dbnum = ? LIMIT 1",
                params![i64::from(dbnum)],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn has_committed_versions(&self) -> anyhow::Result<bool> {
        let connection = self.lock_connection()?;
        let count: u64 =
            connection.query_row("SELECT count(*) FROM data_version_state", [], |row| {
                row.get(0)
            })?;
        Ok(count > 0)
    }

    pub fn latest_snapshot_id(&self) -> anyhow::Result<u64> {
        let connection = self.lock_connection()?;
        latest_snapshot_id(&connection)
    }

    pub fn previous_data_snapshot_id(&self, snapshot_id: u64) -> anyhow::Result<Option<u64>> {
        let connection = self.lock_connection()?;
        Ok(connection
            .query_row(
                &format!(
                    "SELECT max(snapshot_id) FROM {}.snapshots() \
                     WHERE snapshot_id < ? AND commit_extra_info LIKE '%data-version%'",
                    DUCKLAKE_CATALOG_ALIAS
                ),
                params![snapshot_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn snapshot_exists(&self, snapshot_id: u64) -> anyhow::Result<bool> {
        let connection = self.lock_connection()?;
        Ok(connection
            .query_row(
                &format!(
                    "SELECT snapshot_id FROM {}.snapshots() WHERE snapshot_id = ?",
                    DUCKLAKE_CATALOG_ALIAS
                ),
                params![snapshot_id as i64],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn open_pinned_connection(&self, snapshot_id: u64) -> anyhow::Result<Connection> {
        open_connection(&self.config, Some(snapshot_id))
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        let mut connection = self.lock_connection()?;
        connection.execute_batch(CREATE_SCHEMA_SQL)?;
        let current: Option<String> = connection
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'schema_version' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(current) = current {
            if current == "1" && VERSION_STORE_SCHEMA_VERSION == 2 {
                connection.execute_batch("BEGIN TRANSACTION;")?;
                let result = (|| -> anyhow::Result<()> {
                    connection.execute_batch(MIGRATE_V1_TO_V2_SQL)?;
                    connection.execute(
                        &format!(
                            "CALL {}.set_commit_message(?, ?, extra_info => ?)",
                            DUCKLAKE_CATALOG_ALIAS
                        ),
                        params![
                            "plant-model-gen",
                            "migrate version store schema v1 to v2",
                            r#"{"kind":"schema","version":2}"#
                        ],
                    )?;
                    Ok(())
                })();
                match result {
                    Ok(()) => connection.execute_batch("COMMIT;")?,
                    Err(error) => {
                        let _ = connection.execute_batch("ROLLBACK;");
                        return Err(error);
                    }
                }
                return Ok(());
            }
            anyhow::ensure!(
                current == VERSION_STORE_SCHEMA_VERSION.to_string(),
                "不支持的 version store schema: current={current} expected={VERSION_STORE_SCHEMA_VERSION}"
            );
            return Ok(());
        }

        connection.execute_batch("BEGIN TRANSACTION;")?;
        let result = (|| -> anyhow::Result<()> {
            connection.execute_batch(PARTITION_SCHEMA_SQL)?;
            connection.execute(
                "INSERT INTO store_metadata(key, value) VALUES ('schema_version', ?)",
                params![VERSION_STORE_SCHEMA_VERSION.to_string()],
            )?;
            connection.execute(
                &format!(
                    "CALL {}.set_commit_message(?, ?, extra_info => ?)",
                    DUCKLAKE_CATALOG_ALIAS
                ),
                params![
                    "plant-model-gen",
                    "initialize version store schema",
                    r#"{"kind":"schema"}"#
                ],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => connection.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn lock_connection(&self) -> anyhow::Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("DuckLake authority connection mutex poisoned"))
    }

    fn resolve_all_snapshots_by_extra_info(&self, extra_info: &str) -> anyhow::Result<Vec<u64>> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT snapshot_id FROM {}.snapshots() \
             WHERE commit_extra_info = ? ORDER BY snapshot_id",
            DUCKLAKE_CATALOG_ALIAS
        ))?;
        Ok(statement
            .query_map(params![extra_info], |row| row.get::<_, u64>(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn validate_staged_outcome(
        &self,
        stage: &SealedParseStage,
        global_fingerprint: &str,
        snapshot_id: u64,
        idempotent: bool,
        before_versions: Option<BTreeMap<u32, DataVersion>>,
    ) -> anyhow::Result<AuthorityCommitOutcome> {
        let manifest = self.read_manifest(snapshot_id)?;
        manifest.verify_hash()?;
        let expected_version = DataVersion {
            dbnum: stage.dbnum,
            sesno: stage.version.to_sesno,
            commit_fingerprint: stage.fingerprint.clone(),
        };
        anyhow::ensure!(
            manifest.versions.get(&stage.dbnum) == Some(&expected_version),
            "pinned manifest 与 parse stage 版本不一致: dbnum={}",
            stage.dbnum
        );
        if let Some(mut expected_versions) = before_versions {
            expected_versions.insert(stage.dbnum, expected_version);
            anyhow::ensure!(
                manifest.versions == expected_versions,
                "单库提交改变了其他 dbnum 的 manifest"
            );
        }
        validate_pinned_stage_snapshot(
            &self.open_pinned_connection(snapshot_id)?,
            stage,
            global_fingerprint,
        )?;
        Ok(AuthorityCommitOutcome {
            snapshot_id,
            manifest,
            idempotent,
        })
    }
}

fn attach_parse_stage(connection: &Connection, path: &Path) -> anyhow::Result<()> {
    connection.execute_batch(&format!(
        "ATTACH '{}' AS {} (READ_ONLY);",
        escape_path(path),
        PARSE_STAGE_CATALOG_ALIAS
    ))?;
    Ok(())
}

fn detach_parse_stage(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(&format!("DETACH {};", PARSE_STAGE_CATALOG_ALIAS))?;
    Ok(())
}

fn read_data_versions(connection: &Connection) -> anyhow::Result<BTreeMap<u32, DataVersion>> {
    let mut statement = connection.prepare(
        "SELECT dbnum, sesno, commit_fingerprint FROM data_version_state ORDER BY dbnum",
    )?;
    Ok(statement
        .query_map([], |row| {
            let version = DataVersion {
                dbnum: row.get(0)?,
                sesno: row.get(1)?,
                commit_fingerprint: row.get(2)?,
            };
            Ok((version.dbnum, version))
        })?
        .collect::<Result<BTreeMap<_, _>, _>>()?)
}

fn attached_stage_metadata(connection: &Connection, key: &str) -> anyhow::Result<String> {
    Ok(connection.query_row(
        &format!(
            "SELECT value FROM {}.stage_metadata WHERE key = ? LIMIT 1",
            PARSE_STAGE_CATALOG_ALIAS
        ),
        params![key],
        |row| row.get(0),
    )?)
}

fn validate_attached_stage(
    connection: &Connection,
    stage: &SealedParseStage,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        attached_stage_metadata(connection, "run_id")? == stage.run_id,
        "attached parse stage run_id 不一致"
    );
    anyhow::ensure!(
        attached_stage_metadata(connection, "dbnum")? == stage.dbnum.to_string(),
        "attached parse stage dbnum 不一致"
    );
    anyhow::ensure!(
        attached_stage_metadata(connection, "fingerprint")? == stage.fingerprint,
        "attached parse stage fingerprint 不一致"
    );
    anyhow::ensure!(
        attached_stage_metadata(connection, "from_sesno")? == stage.version.from_sesno.to_string()
            && attached_stage_metadata(connection, "to_sesno")?
                == stage.version.to_sesno.to_string()
            && attached_stage_metadata(connection, "source")? == stage.version.source,
        "attached parse stage 版本 metadata 不一致"
    );
    let expected_source_hash = stage.version.source_hash.as_deref().unwrap_or_default();
    anyhow::ensure!(
        attached_stage_metadata(connection, "source_hash")? == expected_source_hash,
        "attached parse stage source_hash 不一致"
    );
    let state = attached_stage_metadata(connection, "state")?;
    anyhow::ensure!(
        matches!(
            state.as_str(),
            "sealed" | "authority_committed" | "replica_applied"
        ),
        "attached parse stage 尚未 sealed: state={state}"
    );

    for (table, expected) in [
        ("stage_element", stage.counts.elements),
        ("stage_hierarchy_edge", stage.counts.hierarchy_edges),
        ("stage_reference_edge", stage.counts.reference_edges),
        ("stage_transform", stage.counts.transforms),
        ("stage_db_catalog", stage.counts.catalog_entries),
    ] {
        let actual: u64 = connection.query_row(
            &format!(
                "SELECT count(*) FROM {}.{table} WHERE dbnum = ?",
                PARSE_STAGE_CATALOG_ALIAS
            ),
            params![i64::from(stage.dbnum)],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            actual == expected,
            "attached parse stage count 不一致: table={table} expected={expected} actual={actual}"
        );
        let total: u64 = connection.query_row(
            &format!("SELECT count(*) FROM {}.{table}", PARSE_STAGE_CATALOG_ALIAS),
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            total == actual,
            "attached parse stage 存在跨 dbnum 数据: table={table} target={} total={total}",
            stage.dbnum
        );
    }
    anyhow::ensure!(
        stage.counts.elements > 0
            && stage.counts.attributes == stage.counts.elements
            && stage.counts.transforms == stage.counts.elements
            && stage.counts.catalog_entries == 1,
        "attached parse stage 覆盖不完整"
    );
    Ok(())
}

fn apply_staged_commit(
    connection: &mut Connection,
    stage: &SealedParseStage,
    global_fingerprint: &str,
    extra_info: &str,
) -> anyhow::Result<()> {
    validate_attached_stage(connection, stage)?;
    let version = AuthorityDbVersion {
        dbnum: stage.dbnum,
        from_sesno: stage.version.from_sesno,
        to_sesno: stage.version.to_sesno,
        source: stage.version.source.clone(),
        commit_fingerprint: stage.fingerprint.clone(),
        source_hash: stage.version.source_hash.clone(),
    };
    validate_continuity(connection, std::slice::from_ref(&version))?;

    let prior_refnos = {
        let mut statement =
            connection.prepare("SELECT refno FROM element WHERE dbnum = ? ORDER BY refno")?;
        statement
            .query_map(params![i64::from(stage.dbnum)], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for table in [
        "hierarchy_edge",
        "reference_edge",
        "transform",
        "element",
        "element_tombstone",
        "db_catalog",
    ] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE dbnum = ?"),
            params![i64::from(stage.dbnum)],
        )?;
    }
    for refno in prior_refnos {
        connection.execute(
            "INSERT INTO element_tombstone VALUES (?, ?, ?, ?)",
            params![
                i64::from(stage.dbnum),
                refno,
                i64::from(stage.version.to_sesno),
                &stage.fingerprint
            ],
        )?;
    }

    connection.execute(
        &format!(
            "INSERT INTO element \
             SELECT dbnum, refno, owner_refno, noun, name, has_children, \
                    attr_codec_version, attr_payload, attr_hash \
             FROM {}.stage_element WHERE dbnum = ?",
            PARSE_STAGE_CATALOG_ALIAS
        ),
        params![i64::from(stage.dbnum)],
    )?;
    connection.execute(
        &format!(
            "INSERT INTO hierarchy_edge \
             SELECT dbnum, parent_refno, child_refno, ordinal \
             FROM {}.stage_hierarchy_edge WHERE dbnum = ?",
            PARSE_STAGE_CATALOG_ALIAS
        ),
        params![i64::from(stage.dbnum)],
    )?;
    connection.execute(
        &format!(
            "INSERT INTO reference_edge \
             SELECT dbnum, source_refno, attribute_name, target_refno, ordinal \
             FROM {}.stage_reference_edge WHERE dbnum = ?",
            PARSE_STAGE_CATALOG_ALIAS
        ),
        params![i64::from(stage.dbnum)],
    )?;
    connection.execute(
        &format!(
            "INSERT INTO transform \
             SELECT dbnum, refno, local_transform, world_transform, transform_hash \
             FROM {}.stage_transform WHERE dbnum = ?",
            PARSE_STAGE_CATALOG_ALIAS
        ),
        params![i64::from(stage.dbnum)],
    )?;
    connection.execute(
        &format!(
            "INSERT INTO db_catalog \
             SELECT dbnum, ref0, db_type, project \
             FROM {}.stage_db_catalog WHERE dbnum = ?",
            PARSE_STAGE_CATALOG_ALIAS
        ),
        params![i64::from(stage.dbnum)],
    )?;

    connection.execute(
        "DELETE FROM data_version_state WHERE dbnum = ?",
        params![i64::from(stage.dbnum)],
    )?;
    connection.execute(
        "INSERT INTO data_version_state VALUES (?, ?, ?, ?, ?, ?, current_timestamp)",
        params![
            i64::from(stage.dbnum),
            i64::from(stage.version.to_sesno),
            i64::from(stage.version.from_sesno),
            &stage.fingerprint,
            &stage.version.source,
            &stage.version.source_hash
        ],
    )?;

    connection.execute("DELETE FROM version_manifest", [])?;
    let current_versions = read_data_versions(connection)?;
    for version in current_versions.values() {
        connection.execute(
            "INSERT INTO version_manifest VALUES (?, ?, ?, ?)",
            params![
                global_fingerprint,
                i64::from(version.dbnum),
                i64::from(version.sesno),
                &version.commit_fingerprint
            ],
        )?;
    }

    let history_start: Option<String> = connection
        .query_row(
            "SELECT value FROM store_metadata \
             WHERE key = 'history_start_fingerprint' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if history_start.is_none() {
        connection.execute(
            "INSERT INTO store_metadata(key, value) \
             VALUES ('history_start_fingerprint', ?)",
            params![global_fingerprint],
        )?;
    }
    connection.execute(
        &format!(
            "CALL {}.set_commit_message(?, ?, extra_info => ?)",
            DUCKLAKE_CATALOG_ALIAS
        ),
        params![
            "plant-model-gen",
            format!(
                "parse stage commit dbnum={} fingerprint={}",
                stage.dbnum, stage.fingerprint
            ),
            extra_info
        ],
    )?;
    Ok(())
}

fn validate_pinned_stage_snapshot(
    connection: &Connection,
    stage: &SealedParseStage,
    global_fingerprint: &str,
) -> anyhow::Result<()> {
    for (table, expected) in [
        ("element", stage.counts.elements),
        ("hierarchy_edge", stage.counts.hierarchy_edges),
        ("reference_edge", stage.counts.reference_edges),
        ("transform", stage.counts.transforms),
        ("db_catalog", stage.counts.catalog_entries),
    ] {
        let actual: u64 = connection.query_row(
            &format!("SELECT count(*) FROM {table} WHERE dbnum = ?"),
            params![i64::from(stage.dbnum)],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            actual == expected,
            "pinned snapshot count 不一致: table={table} dbnum={} expected={expected} actual={actual}",
            stage.dbnum
        );
    }
    let version: (u32, String) = connection.query_row(
        "SELECT sesno, commit_fingerprint FROM data_version_state \
         WHERE dbnum = ? LIMIT 1",
        params![i64::from(stage.dbnum)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    anyhow::ensure!(
        version.0 == stage.version.to_sesno && version.1 == stage.fingerprint,
        "pinned snapshot data_version_state 与 stage 不一致"
    );
    let bad_manifest_rows: u64 = connection.query_row(
        "SELECT count(*) FROM version_manifest m \
         LEFT JOIN data_version_state s ON s.dbnum = m.dbnum \
         WHERE m.manifest_fingerprint != ? OR s.dbnum IS NULL \
            OR m.sesno != s.sesno OR m.commit_fingerprint != s.commit_fingerprint",
        params![global_fingerprint],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        bad_manifest_rows == 0,
        "pinned snapshot manifest 存在 {bad_manifest_rows} 条不一致记录"
    );
    let manifest_count: u64 =
        connection.query_row("SELECT count(*) FROM version_manifest", [], |row| {
            row.get(0)
        })?;
    let version_count: u64 =
        connection.query_row("SELECT count(*) FROM data_version_state", [], |row| {
            row.get(0)
        })?;
    anyhow::ensure!(
        manifest_count == version_count,
        "pinned snapshot manifest 覆盖不足: manifest={manifest_count} versions={version_count}"
    );
    Ok(())
}

fn apply_commit(
    connection: &mut Connection,
    commit: &AuthorityCommit,
    extra_info: &str,
) -> anyhow::Result<()> {
    validate_continuity(connection, &commit.db_versions)?;

    for dbnum in &commit.replace_dbnums {
        let version = commit
            .db_versions
            .iter()
            .find(|version| version.dbnum == *dbnum)
            .ok_or_else(|| anyhow::anyhow!("替换 dbnum={dbnum} 没有对应版本提交"))?;
        let prior_refnos = {
            let mut statement =
                connection.prepare("SELECT refno FROM element WHERE dbnum = ? ORDER BY refno")?;
            statement
                .query_map(params![i64::from(*dbnum)], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        connection.execute(
            "DELETE FROM hierarchy_edge WHERE dbnum = ?",
            params![i64::from(*dbnum)],
        )?;
        connection.execute(
            "DELETE FROM reference_edge WHERE dbnum = ?",
            params![i64::from(*dbnum)],
        )?;
        connection.execute(
            "DELETE FROM transform WHERE dbnum = ?",
            params![i64::from(*dbnum)],
        )?;
        connection.execute(
            "DELETE FROM element WHERE dbnum = ?",
            params![i64::from(*dbnum)],
        )?;
        connection.execute(
            "DELETE FROM element_tombstone WHERE dbnum = ?",
            params![i64::from(*dbnum)],
        )?;
        for refno in prior_refnos {
            connection.execute(
                "INSERT INTO element_tombstone VALUES (?, ?, ?, ?)",
                params![
                    i64::from(*dbnum),
                    refno,
                    i64::from(version.to_sesno),
                    &version.commit_fingerprint
                ],
            )?;
        }
    }

    for (dbnum, refnos) in &commit.delete_refnos {
        for refno in refnos {
            let refno = refno.to_string();
            connection.execute(
                "DELETE FROM hierarchy_edge WHERE dbnum = ? AND (parent_refno = ? OR child_refno = ?)",
                params![i64::from(*dbnum), &refno, &refno],
            )?;
            connection.execute(
                "DELETE FROM reference_edge WHERE dbnum = ? AND source_refno = ?",
                params![i64::from(*dbnum), &refno],
            )?;
            connection.execute(
                "DELETE FROM transform WHERE dbnum = ? AND refno = ?",
                params![i64::from(*dbnum), &refno],
            )?;
            connection.execute(
                "DELETE FROM element WHERE dbnum = ? AND refno = ?",
                params![i64::from(*dbnum), &refno],
            )?;
            let version = commit
                .db_versions
                .iter()
                .find(|version| version.dbnum == *dbnum)
                .ok_or_else(|| anyhow::anyhow!("删除 dbnum={dbnum} 没有对应版本提交"))?;
            connection.execute(
                "INSERT INTO element_tombstone VALUES (?, ?, ?, ?)",
                params![
                    i64::from(*dbnum),
                    &refno,
                    i64::from(version.to_sesno),
                    &version.commit_fingerprint
                ],
            )?;
        }
    }

    let upsert_total = commit.upsert_elements.len();
    for (idx, item) in commit.upsert_elements.iter().enumerate() {
        anyhow::ensure!(
            item.element.refno.is_valid()
                && (item.element.owner.is_valid() || item.element.owner.is_unset()),
            "element refno/owner 非法: refno={} owner={}",
            item.element.refno,
            item.element.owner
        );
        anyhow::ensure!(
            item.element.refno == item.attributes.refno,
            "element/attributes refno 不一致"
        );
        item.attributes.verify()?;
        let refno = item.element.refno.to_string();
        let owner = item.element.owner.to_string();
        let payload = encode_attribute_set_payload(&item.attributes)?;
        connection.execute(
            "DELETE FROM element WHERE dbnum = ? AND refno = ?",
            params![i64::from(item.element.dbnum), &refno],
        )?;
        connection.execute(
            "DELETE FROM element_tombstone WHERE dbnum = ? AND refno = ?",
            params![i64::from(item.element.dbnum), &refno],
        )?;
        connection.execute(
            "INSERT INTO element VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                i64::from(item.element.dbnum),
                &refno,
                &owner,
                &item.element.noun,
                &item.element.name,
                item.element.has_children,
                i64::from(item.attributes.codec_version),
                payload,
                &item.attributes.canonical_hash
            ],
        )?;
        let done = idx + 1;
        if done == upsert_total || done % 2000 == 0 {
            eprintln!("DuckLake commit: upsert elements {done}/{upsert_total}");
        }

        connection.execute(
            "DELETE FROM reference_edge WHERE dbnum = ? AND source_refno = ?",
            params![i64::from(item.element.dbnum), &refno],
        )?;
        connection.execute(
            "DELETE FROM hierarchy_edge WHERE dbnum = ? AND parent_refno = ?",
            params![i64::from(item.element.dbnum), &refno],
        )?;
        for edge in item.attributes.reference_edges(item.element.dbnum) {
            connection.execute(
                "INSERT INTO reference_edge VALUES (?, ?, ?, ?, ?)",
                params![
                    i64::from(edge.dbnum),
                    edge.source.to_string(),
                    edge.attribute_name,
                    edge.target.to_string(),
                    i64::from(edge.ordinal)
                ],
            )?;
        }
    }

    let mut replaced_children = BTreeSet::new();
    for row in &commit.hierarchy_rows {
        if replaced_children.insert((row.dbnum, row.child)) {
            connection.execute(
                "DELETE FROM hierarchy_edge WHERE dbnum = ? AND child_refno = ?",
                params![i64::from(row.dbnum), row.child.to_string()],
            )?;
        }
        connection.execute(
            "INSERT INTO hierarchy_edge VALUES (?, ?, ?, ?)",
            params![
                i64::from(row.dbnum),
                row.parent.to_string(),
                row.child.to_string(),
                i64::from(row.ordinal)
            ],
        )?;
    }

    for transform in &commit.transforms {
        let refno = transform.refno.to_string();
        let local = transform
            .local
            .as_ref()
            .map(bincode::serialize)
            .transpose()?;
        let world = bincode::serialize(&transform.world)?;
        let transform_hash = crate::generation_read::hash_serializable(transform);
        connection.execute(
            "DELETE FROM transform WHERE dbnum = ? AND refno = ?",
            params![i64::from(transform.dbnum), &refno],
        )?;
        connection.execute(
            "INSERT INTO transform VALUES (?, ?, ?, ?, ?)",
            params![
                i64::from(transform.dbnum),
                &refno,
                local,
                world,
                transform_hash
            ],
        )?;
    }

    for entry in &commit.db_catalog {
        connection.execute(
            "DELETE FROM db_catalog WHERE dbnum = ?",
            params![i64::from(entry.dbnum)],
        )?;
        connection.execute(
            "INSERT INTO db_catalog VALUES (?, ?, ?, ?)",
            params![
                i64::from(entry.dbnum),
                entry.ref0.map(i64::from),
                &entry.db_type,
                &entry.project
            ],
        )?;
    }

    for version in &commit.db_versions {
        connection.execute(
            "DELETE FROM data_version_state WHERE dbnum = ?",
            params![i64::from(version.dbnum)],
        )?;
        connection.execute(
            "INSERT INTO data_version_state VALUES (?, ?, ?, ?, ?, ?, current_timestamp)",
            params![
                i64::from(version.dbnum),
                i64::from(version.to_sesno),
                i64::from(version.from_sesno),
                &version.commit_fingerprint,
                &version.source,
                &version.source_hash
            ],
        )?;
    }

    connection.execute("DELETE FROM version_manifest", [])?;
    let manifest_fingerprint = commit.global_fingerprint.clone();
    let current_versions = {
        let mut statement = connection.prepare(
            "SELECT dbnum, sesno, commit_fingerprint FROM data_version_state ORDER BY dbnum",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (dbnum, sesno, fingerprint) in current_versions {
        connection.execute(
            "INSERT INTO version_manifest VALUES (?, ?, ?, ?)",
            params![
                &manifest_fingerprint,
                i64::from(dbnum),
                i64::from(sesno),
                fingerprint
            ],
        )?;
    }

    if commit.bootstrap_current_state {
        let existing: Option<String> = connection
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'history_start_fingerprint' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            existing
                .as_ref()
                .is_none_or(|value| value == &commit.global_fingerprint),
            "history_start_snapshot 已由另一提交建立"
        );
        if existing.is_none() {
            connection.execute(
                "INSERT INTO store_metadata(key, value) VALUES ('history_start_fingerprint', ?)",
                params![&commit.global_fingerprint],
            )?;
        }
    }

    connection.execute(
        &format!(
            "CALL {}.set_commit_message(?, ?, extra_info => ?)",
            DUCKLAKE_CATALOG_ALIAS
        ),
        params![
            "plant-model-gen",
            format!("data version commit {}", commit.global_fingerprint),
            extra_info
        ],
    )?;
    Ok(())
}

fn validate_commit(commit: &AuthorityCommit) -> anyhow::Result<()> {
    anyhow::ensure!(
        !commit.global_fingerprint.trim().is_empty(),
        "global_fingerprint 不能为空"
    );
    anyhow::ensure!(!commit.db_versions.is_empty(), "至少提交一个 dbnum");
    let mut dbnums = BTreeSet::new();
    for version in &commit.db_versions {
        anyhow::ensure!(version.dbnum > 0, "dbnum 必须非零");
        anyhow::ensure!(
            version.to_sesno > 0 && version.from_sesno <= version.to_sesno,
            "非法 sesno 区间: {}..={}",
            version.from_sesno,
            version.to_sesno
        );
        anyhow::ensure!(
            !version.commit_fingerprint.trim().is_empty(),
            "dbnum={} 缺少 commit fingerprint",
            version.dbnum
        );
        anyhow::ensure!(dbnums.insert(version.dbnum), "dbnum 重复提交");
    }
    for dbnum in commit.delete_refnos.keys() {
        anyhow::ensure!(
            dbnums.contains(dbnum),
            "删除 dbnum={dbnum} 不在本次版本提交中"
        );
        anyhow::ensure!(
            !commit.replace_dbnums.contains(dbnum),
            "dbnum={dbnum} 不能同时使用完整替换和显式删除"
        );
    }
    for dbnum in &commit.replace_dbnums {
        anyhow::ensure!(
            dbnums.contains(dbnum),
            "完整替换 dbnum={dbnum} 不在本次版本提交中"
        );
    }
    let mut upsert_keys = BTreeSet::new();
    for item in &commit.upsert_elements {
        anyhow::ensure!(
            item.element.refno == item.attributes.refno,
            "element/attributes refno 不一致"
        );
        anyhow::ensure!(
            dbnums.contains(&item.element.dbnum),
            "element dbnum={} 不在本次版本提交中",
            item.element.dbnum
        );
        anyhow::ensure!(
            upsert_keys.insert((item.element.dbnum, item.element.refno)),
            "element 重复 upsert: dbnum={} refno={}",
            item.element.dbnum,
            item.element.refno
        );
        item.attributes.verify()?;
    }
    for (dbnum, refnos) in &commit.delete_refnos {
        let mut delete_keys = BTreeSet::new();
        for refno in refnos {
            anyhow::ensure!(
                refno.is_valid(),
                "delete_refnos 包含非法 refno: dbnum={dbnum} refno={refno}"
            );
            anyhow::ensure!(
                delete_keys.insert(*refno),
                "delete_refnos 重复: dbnum={dbnum} refno={refno}"
            );
            anyhow::ensure!(
                !upsert_keys.contains(&(*dbnum, *refno)),
                "同一 element 不能同时 delete/upsert: dbnum={dbnum} refno={refno}"
            );
        }
    }
    let mut hierarchy_keys = BTreeSet::new();
    let mut hierarchy_ordinals = BTreeSet::new();
    let mut hierarchy_parents = BTreeMap::new();
    for row in &commit.hierarchy_rows {
        anyhow::ensure!(
            dbnums.contains(&row.dbnum),
            "hierarchy dbnum={} 不在本次版本提交中",
            row.dbnum
        );
        anyhow::ensure!(
            row.parent.is_valid() && row.child.is_valid() && row.parent != row.child,
            "hierarchy 不允许自环: {}",
            row.parent
        );
        anyhow::ensure!(
            hierarchy_keys.insert((row.dbnum, row.parent, row.child)),
            "hierarchy edge 重复: {} -> {}",
            row.parent,
            row.child
        );
        anyhow::ensure!(
            hierarchy_ordinals.insert((row.dbnum, row.parent, row.ordinal)),
            "hierarchy ordinal 重复: parent={} ordinal={}",
            row.parent,
            row.ordinal
        );
        if let Some(existing) = hierarchy_parents.insert((row.dbnum, row.child), row.parent) {
            anyhow::ensure!(
                existing == row.parent,
                "hierarchy child={} 同时属于 parent={} 和 {}",
                row.child,
                existing,
                row.parent
            );
        }
    }
    let mut transform_keys = BTreeSet::new();
    for transform in &commit.transforms {
        anyhow::ensure!(
            transform.refno.is_valid(),
            "transform refno 非法: {}",
            transform.refno
        );
        anyhow::ensure!(
            dbnums.contains(&transform.dbnum),
            "transform dbnum={} 不在本次版本提交中",
            transform.dbnum
        );
        anyhow::ensure!(
            transform_keys.insert((transform.dbnum, transform.refno)),
            "transform 重复: dbnum={} refno={}",
            transform.dbnum,
            transform.refno
        );
    }
    let mut catalog_dbnums = BTreeSet::new();
    for entry in &commit.db_catalog {
        anyhow::ensure!(
            !entry.db_type.trim().is_empty(),
            "db_catalog dbnum={} 缺少 db_type",
            entry.dbnum
        );
        anyhow::ensure!(
            dbnums.contains(&entry.dbnum),
            "db_catalog dbnum={} 不在本次版本提交中",
            entry.dbnum
        );
        anyhow::ensure!(
            catalog_dbnums.insert(entry.dbnum),
            "db_catalog dbnum={} 重复",
            entry.dbnum
        );
    }
    Ok(())
}

fn validate_continuity(
    connection: &Connection,
    versions: &[AuthorityDbVersion],
) -> anyhow::Result<()> {
    for version in versions {
        let watermark: Option<u32> = connection
            .query_row(
                "SELECT sesno FROM data_version_state WHERE dbnum = ? LIMIT 1",
                params![i64::from(version.dbnum)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(watermark) = watermark {
            anyhow::ensure!(
                version.to_sesno >= watermark,
                "dbnum={} 提交终点 {} 回退权威水位 {}",
                version.dbnum,
                version.to_sesno,
                watermark
            );
        }
        if version.source.eq_ignore_ascii_case("incremental")
            && let Some(watermark) = watermark
        {
            anyhow::ensure!(
                version.to_sesno > watermark,
                "dbnum={} 增量终点 {} 未推进权威水位 {}",
                version.dbnum,
                version.to_sesno,
                watermark
            );
            anyhow::ensure!(
                version.from_sesno <= watermark.saturating_add(1),
                "dbnum={} 增量 {}..={} 不衔接权威水位 {}",
                version.dbnum,
                version.from_sesno,
                version.to_sesno,
                watermark
            );
        }
    }
    Ok(())
}

fn open_connection(
    config: &DuckLakeConfig,
    snapshot_id: Option<u64>,
) -> anyhow::Result<Connection> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(&format!(
        "SET memory_limit = '{}'; SET threads = {}; SET temp_directory = '{}';",
        escape_sql_literal(&config.memory_limit),
        config.threads,
        escape_path(&config.temp_directory)
    ))?;
    connection.execute_batch(&format!(
        "LOAD '{}'; LOAD '{}';",
        escape_path(&config.extensions.sqlite_extension),
        escape_path(&config.extensions.ducklake_extension)
    ))?;
    let snapshot_clause = snapshot_id
        .map(|snapshot_id| format!(", SNAPSHOT_VERSION {snapshot_id}, CREATE_IF_NOT_EXISTS false"))
        .unwrap_or_default();
    connection.execute_batch(&format!(
        "ATTACH 'ducklake:sqlite:{}' AS {} (DATA_PATH '{}'{snapshot_clause}); USE {};",
        escape_path(&config.metadata_catalog),
        DUCKLAKE_CATALOG_ALIAS,
        escape_path(&config.data_path),
        DUCKLAKE_CATALOG_ALIAS
    ))?;
    Ok(connection)
}

fn open_connection_readonly(
    config: &DuckLakeConfig,
    snapshot_id: Option<u64>,
) -> anyhow::Result<Connection> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(&format!(
        "SET memory_limit = '{}'; SET threads = {}; SET temp_directory = '{}';",
        escape_sql_literal(&config.memory_limit),
        config.threads,
        escape_path(&config.temp_directory)
    ))?;
    connection.execute_batch(&format!(
        "LOAD '{}'; LOAD '{}';",
        escape_path(&config.extensions.sqlite_extension),
        escape_path(&config.extensions.ducklake_extension)
    ))?;
    let snapshot_clause = snapshot_id
        .map(|snapshot_id| format!(", SNAPSHOT_VERSION {snapshot_id}"))
        .unwrap_or_default();
    connection.execute_batch(&format!(
        "ATTACH 'ducklake:sqlite:{}' AS {} (DATA_PATH '{}', READ_ONLY{}); USE {};",
        escape_path(&config.metadata_catalog),
        DUCKLAKE_CATALOG_ALIAS,
        escape_path(&config.data_path),
        snapshot_clause,
        DUCKLAKE_CATALOG_ALIAS
    ))?;
    Ok(connection)
}

fn latest_snapshot_id(connection: &Connection) -> anyhow::Result<u64> {
    Ok(connection.query_row(
        &format!(
            "SELECT max(snapshot_id) FROM {}.snapshots()",
            DUCKLAKE_CATALOG_ALIAS
        ),
        [],
        |row| row.get(0),
    )?)
}

fn read_history_start_snapshot(connection: &Connection) -> anyhow::Result<Option<u64>> {
    let fingerprint: Option<String> = connection
        .query_row(
            "SELECT value FROM store_metadata WHERE key = 'history_start_fingerprint' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(fingerprint) = fingerprint else {
        return Ok(None);
    };
    let extra_info = commit_extra_info(&fingerprint);
    Ok(connection
        .query_row(
            &format!(
                "SELECT snapshot_id FROM {}.snapshots() WHERE commit_extra_info = ? LIMIT 1",
                DUCKLAKE_CATALOG_ALIAS
            ),
            params![extra_info],
            |row| row.get(0),
        )
        .optional()?)
}

fn commit_extra_info(fingerprint: &str) -> String {
    serde_json::json!({
        "kind": "data-version",
        "fingerprint": fingerprint,
    })
    .to_string()
}

fn create_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn escape_path(path: &Path) -> String {
    escape_sql_literal(&path.to_string_lossy().replace('\\', "/"))
}

fn absolute_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_read::AttributeSet;
    use crate::version_store::{DuckLakeParseStager, ParseStageVersion, ParsedFactBatch};

    #[test]
    fn absolute_path_resolves_relative_and_preserves_absolute() {
        let current_dir = std::env::current_dir().unwrap();
        let relative = Path::new("runtime/ducklake/data");
        let absolute = current_dir.join(relative);

        assert_eq!(absolute_path(relative).unwrap(), absolute);
        assert_eq!(absolute_path(&absolute).unwrap(), absolute);
    }
    use aios_core::{AttrVal, NamedAttrMap};

    fn commit(versions: Vec<AuthorityDbVersion>) -> AuthorityCommit {
        AuthorityCommit {
            global_fingerprint: "fixture".to_string(),
            db_versions: versions,
            replace_dbnums: BTreeSet::new(),
            upsert_elements: Vec::new(),
            delete_refnos: BTreeMap::new(),
            hierarchy_rows: Vec::new(),
            transforms: Vec::new(),
            db_catalog: Vec::new(),
            bootstrap_current_state: false,
        }
    }

    #[test]
    fn authority_rejects_duplicate_dbnum_and_invalid_ranges_before_commit() {
        let version = AuthorityDbVersion {
            dbnum: 1,
            from_sesno: 10,
            to_sesno: 11,
            source: "incremental".to_string(),
            commit_fingerprint: "db-1".to_string(),
            source_hash: None,
        };
        assert!(validate_commit(&commit(vec![version.clone(), version])).is_err());

        let invalid = AuthorityDbVersion {
            dbnum: 2,
            from_sesno: 12,
            to_sesno: 11,
            source: "incremental".to_string(),
            commit_fingerprint: "db-2".to_string(),
            source_hash: None,
        };
        assert!(validate_commit(&commit(vec![invalid])).is_err());
    }

    #[test]
    fn commit_fingerprint_is_encoded_as_unique_extra_info() {
        assert_ne!(commit_extra_info("a"), commit_extra_info("b"));
        assert_eq!(
            commit_extra_info("a"),
            r#"{"fingerprint":"a","kind":"data-version"}"#
        );
    }

    #[test]
    fn continuity_rejects_rollback_and_incremental_gap() {
        let connection = Connection::open_in_memory().expect("connection");
        connection
            .execute_batch(
                "CREATE TABLE data_version_state (
                    dbnum UINTEGER, sesno UINTEGER, from_sesno UINTEGER,
                    commit_fingerprint VARCHAR, source VARCHAR, source_hash VARCHAR,
                    committed_at TIMESTAMP
                );
                INSERT INTO data_version_state VALUES
                    (7997, 20, 1, 'old', 'total', NULL, current_timestamp);",
            )
            .expect("schema");
        let version = |from_sesno, to_sesno, source: &str| AuthorityDbVersion {
            dbnum: 7997,
            from_sesno,
            to_sesno,
            source: source.to_string(),
            commit_fingerprint: format!("{source}-{from_sesno}-{to_sesno}"),
            source_hash: None,
        };
        assert!(validate_continuity(&connection, &[version(1, 19, "total")]).is_err());
        assert!(validate_continuity(&connection, &[version(22, 22, "incremental")]).is_err());
        validate_continuity(&connection, &[version(21, 21, "incremental")])
            .expect("contiguous incremental");
    }

    #[tokio::test]
    async fn sealed_stage_can_be_attached_read_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stager =
            DuckLakeParseStager::open(temp.path(), "authority-attach", 7997).expect("open stage");
        let refno = RefnoEnum::from("10/1");
        let mut named = NamedAttrMap::new("WORLD");
        named.insert(
            "REFNO".to_string(),
            AttrVal::RefU64Type(refno.refno()).into(),
        );
        stager
            .write_batch(&ParsedFactBatch {
                batch_id: "chunk".to_string(),
                dbnum: 7997,
                elements: vec![VersionStoreElement {
                    element: ElementSnapshot {
                        refno,
                        dbnum: 7997,
                        owner: RefnoEnum::default(),
                        noun: "WORLD".to_string(),
                        name: "/WORLD".to_string(),
                        has_children: false,
                    },
                    attributes: AttributeSet::from_named_attr_map(refno, &named),
                }],
                hierarchy_rows: Vec::new(),
                pline_facts: Vec::new(),
                db_catalog: vec![DbCatalogEntry {
                    dbnum: 7997,
                    ref0: Some(10),
                    db_type: "DESI".to_string(),
                    project: "fixture".to_string(),
                }],
            })
            .expect("write stage");
        stager.finalize_transforms().await.expect("finalize");
        let stage = stager
            .seal(ParseStageVersion {
                from_sesno: 1,
                to_sesno: 20,
                source: "total".to_string(),
                source_hash: Some("fixture".to_string()),
            })
            .expect("seal");
        drop(stager);

        let connection = Connection::open_in_memory().expect("authority connection");
        attach_parse_stage(&connection, &stage.path).expect("attach");
        validate_attached_stage(&connection, &stage).expect("validate");
        detach_parse_stage(&connection).expect("detach");
    }
}
