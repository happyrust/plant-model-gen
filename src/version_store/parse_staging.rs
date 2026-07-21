use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use aios_core::pdms_data::PlinParamData;
use aios_core::transform::source::TransformFactSource;
use aios_core::{NamedAttrMap, RefnoEnum, Transform};
use async_trait::async_trait;
use duckdb::{AccessMode, Config, Connection, OptionalExt, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::generation_read::{
    AttributeSet, HierarchyRow, TransformSnapshot, decode_attribute_set_payload,
    encode_attribute_set_payload, hash_serializable,
};

use super::authority::{DbCatalogEntry, VersionStoreElement};

const STAGE_SCHEMA_VERSION: u16 = 1;

const CREATE_STAGE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS stage_metadata (
    key VARCHAR PRIMARY KEY,
    value VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS stage_chunk (
    batch_id VARCHAR PRIMARY KEY,
    batch_hash VARCHAR NOT NULL,
    element_count UBIGINT NOT NULL,
    hierarchy_count UBIGINT NOT NULL,
    pline_count UBIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS stage_element (
    dbnum UINTEGER NOT NULL,
    refno VARCHAR PRIMARY KEY,
    owner_refno VARCHAR NOT NULL,
    noun VARCHAR NOT NULL,
    name VARCHAR NOT NULL,
    has_children BOOLEAN NOT NULL,
    attr_codec_version USMALLINT NOT NULL,
    attr_payload BLOB NOT NULL,
    attr_hash VARCHAR NOT NULL,
    content_hash VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS stage_hierarchy_edge (
    dbnum UINTEGER NOT NULL,
    parent_refno VARCHAR NOT NULL,
    child_refno VARCHAR PRIMARY KEY,
    ordinal UINTEGER NOT NULL,
    UNIQUE(parent_refno, ordinal)
);

CREATE TABLE IF NOT EXISTS stage_reference_edge (
    dbnum UINTEGER NOT NULL,
    source_refno VARCHAR NOT NULL,
    attribute_name VARCHAR NOT NULL,
    target_refno VARCHAR NOT NULL,
    ordinal UINTEGER NOT NULL,
    PRIMARY KEY(source_refno, attribute_name, ordinal)
);

CREATE TABLE IF NOT EXISTS stage_pline_fact (
    dbnum UINTEGER NOT NULL,
    refno VARCHAR NOT NULL,
    pline_key VARCHAR NOT NULL,
    payload BLOB NOT NULL,
    content_hash VARCHAR NOT NULL,
    PRIMARY KEY(refno, pline_key)
);

CREATE TABLE IF NOT EXISTS stage_transform (
    dbnum UINTEGER NOT NULL,
    refno VARCHAR PRIMARY KEY,
    local_transform BLOB,
    world_transform BLOB NOT NULL,
    transform_hash VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS stage_db_catalog (
    dbnum UINTEGER PRIMARY KEY,
    ref0 UINTEGER,
    db_type VARCHAR NOT NULL,
    project VARCHAR NOT NULL,
    content_hash VARCHAR NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStageState {
    Created,
    Parsing,
    FactsSealed,
    TransformsFinalized,
    Sealed,
    AuthorityCommitted,
    ReplicaApplied,
}

impl ParseStageState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Parsing => "parsing",
            Self::FactsSealed => "facts_sealed",
            Self::TransformsFinalized => "transforms_finalized",
            Self::Sealed => "sealed",
            Self::AuthorityCommitted => "authority_committed",
            Self::ReplicaApplied => "replica_applied",
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "created" => Ok(Self::Created),
            "parsing" => Ok(Self::Parsing),
            "facts_sealed" => Ok(Self::FactsSealed),
            "transforms_finalized" => Ok(Self::TransformsFinalized),
            "sealed" => Ok(Self::Sealed),
            "authority_committed" => Ok(Self::AuthorityCommitted),
            "replica_applied" => Ok(Self::ReplicaApplied),
            other => anyhow::bail!("未知 parse stage state: {other}"),
        }
    }

    fn accepts_fact_writes(self) -> bool {
        matches!(self, Self::Created | Self::Parsing)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedPlineFact {
    pub refno: RefnoEnum,
    pub key: String,
    pub value: PlinParamData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFactBatch {
    pub batch_id: String,
    pub dbnum: u32,
    pub elements: Vec<VersionStoreElement>,
    pub hierarchy_rows: Vec<HierarchyRow>,
    pub pline_facts: Vec<ParsedPlineFact>,
    pub db_catalog: Vec<DbCatalogEntry>,
}

impl ParsedFactBatch {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.batch_id.trim().is_empty(), "batch_id 不能为空");
        anyhow::ensure!(self.dbnum > 0, "batch dbnum 必须非零");
        let mut element_refnos = BTreeSet::new();
        for item in &self.elements {
            anyhow::ensure!(
                item.element.dbnum == self.dbnum,
                "element dbnum={} 与 batch dbnum={} 不一致",
                item.element.dbnum,
                self.dbnum
            );
            anyhow::ensure!(
                item.element.refno == item.attributes.refno,
                "element/attributes refno 不一致: {}",
                item.element.refno
            );
            item.attributes.verify()?;
            anyhow::ensure!(
                element_refnos.insert(item.element.refno),
                "batch 内 element 重复: {}",
                item.element.refno
            );
        }
        let mut hierarchy_children = BTreeSet::new();
        let mut hierarchy_ordinals = BTreeSet::new();
        for row in &self.hierarchy_rows {
            anyhow::ensure!(
                row.dbnum == self.dbnum,
                "hierarchy dbnum={} 与 batch dbnum={} 不一致",
                row.dbnum,
                self.dbnum
            );
            anyhow::ensure!(
                row.parent.is_valid() && row.child.is_valid() && row.parent != row.child,
                "非法 hierarchy edge: {} -> {}",
                row.parent,
                row.child
            );
            anyhow::ensure!(
                hierarchy_children.insert(row.child),
                "batch 内 child={} 存在多个 hierarchy edge",
                row.child
            );
            anyhow::ensure!(
                hierarchy_ordinals.insert((row.parent, row.ordinal)),
                "batch 内 hierarchy ordinal 冲突: parent={} ordinal={}",
                row.parent,
                row.ordinal
            );
        }
        let mut pline_keys = BTreeSet::new();
        for fact in &self.pline_facts {
            anyhow::ensure!(fact.refno.is_valid(), "PLINE refno 非法: {}", fact.refno);
            anyhow::ensure!(!fact.key.trim().is_empty(), "PLINE key 不能为空");
            anyhow::ensure!(
                pline_keys.insert((fact.refno, fact.key.clone())),
                "batch 内 PLINE fact 重复: refno={} key={}",
                fact.refno,
                fact.key
            );
        }
        let mut catalog_dbnums = BTreeSet::new();
        for entry in &self.db_catalog {
            anyhow::ensure!(
                entry.dbnum == self.dbnum,
                "catalog dbnum={} 与 batch dbnum={} 不一致",
                entry.dbnum,
                self.dbnum
            );
            anyhow::ensure!(
                catalog_dbnums.insert(entry.dbnum),
                "batch 内 db_catalog 重复: {}",
                entry.dbnum
            );
        }
        Ok(())
    }

    pub fn canonical_hash(&self) -> String {
        hash_serializable(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseStageVersion {
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source: String,
    pub source_hash: Option<String>,
}

impl ParseStageVersion {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.to_sesno > 0 && self.from_sesno <= self.to_sesno,
            "非法 stage sesno 区间: {}..={}",
            self.from_sesno,
            self.to_sesno
        );
        anyhow::ensure!(!self.source.trim().is_empty(), "stage source 不能为空");
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseStageCounts {
    pub chunks: u64,
    pub elements: u64,
    pub attributes: u64,
    pub hierarchy_edges: u64,
    pub reference_edges: u64,
    pub pline_facts: u64,
    pub transforms: u64,
    pub catalog_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWriteReport {
    pub batch_id: String,
    pub batch_hash: String,
    pub idempotent: bool,
    pub counts: ParseStageCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedParseStage {
    pub path: PathBuf,
    pub run_id: String,
    pub dbnum: u32,
    pub state: ParseStageState,
    pub version: ParseStageVersion,
    pub fingerprint: String,
    pub rolling_hash: String,
    pub counts: ParseStageCounts,
    pub authority_snapshot_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedParsePayload {
    pub elements: Vec<VersionStoreElement>,
    pub hierarchy_rows: Vec<HierarchyRow>,
    pub transforms: Vec<TransformSnapshot>,
    pub db_catalog: Vec<DbCatalogEntry>,
}

impl SealedParseStage {
    /// 重新以只读方式打开 stage，并验证 seal、payload/hash 与 transform 覆盖。
    ///
    /// 调用前应释放 `DuckLakeParseStager` 的写连接；Windows 上 DuckDB 会对文件加锁。
    pub fn verify(&self) -> anyhow::Result<()> {
        self.load_payload().map(|_| ())
    }

    /// 从已 seal 的 stage 加载后续 authority/replica 所需的强类型 payload。
    pub fn load_payload(&self) -> anyhow::Result<StagedParsePayload> {
        let connection = Connection::open_with_flags(
            &self.path,
            Config::default().access_mode(AccessMode::ReadOnly)?,
        )?;
        let actual = read_sealed_descriptor(&connection, &self.path)?;
        anyhow::ensure!(
            actual == *self,
            "parse stage descriptor 与 seal 结果不一致: expected={self:?} actual={actual:?}"
        );
        validate_stage_content(&connection, self.dbnum)?;
        let payload = read_verified_payload(&connection, self.dbnum)?;
        let actual_fingerprint =
            compute_stage_fingerprint(&connection, &self.run_id, self.dbnum, &self.version)?;
        anyhow::ensure!(
            actual_fingerprint == self.fingerprint,
            "parse stage fingerprint 不一致: expected={} actual={actual_fingerprint}",
            self.fingerprint
        );
        Ok(payload)
    }
}

#[derive(Clone)]
pub struct DuckLakeParseStager {
    path: PathBuf,
    run_id: String,
    dbnum: u32,
    connection: Arc<Mutex<Connection>>,
}

impl DuckLakeParseStager {
    pub fn open(
        staging_root: impl AsRef<Path>,
        run_id: impl Into<String>,
        dbnum: u32,
    ) -> anyhow::Result<Self> {
        let run_id = run_id.into();
        validate_run_id(&run_id)?;
        anyhow::ensure!(dbnum > 0, "stage dbnum 必须非零");
        let run_directory = staging_root.as_ref().join(&run_id);
        std::fs::create_dir_all(&run_directory)?;
        let path = run_directory.join(format!("{dbnum}.duckdb"));
        let connection = Connection::open(&path)?;
        connection.execute_batch(CREATE_STAGE_SCHEMA_SQL)?;
        let stager = Self {
            path,
            run_id,
            dbnum,
            connection: Arc::new(Mutex::new(connection)),
        };
        stager.initialize_or_validate_metadata()?;
        Ok(stager)
    }

    pub fn open_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        anyhow::ensure!(path.is_file(), "parse stage 不存在: {}", path.display());
        let connection = Connection::open(&path)?;
        connection.execute_batch(CREATE_STAGE_SCHEMA_SQL)?;
        let run_id = metadata_required(&connection, "run_id")?;
        let dbnum = metadata_required(&connection, "dbnum")?.parse::<u32>()?;
        let stager = Self {
            path,
            run_id,
            dbnum,
            connection: Arc::new(Mutex::new(connection)),
        };
        stager.initialize_or_validate_metadata()?;
        Ok(stager)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn dbnum(&self) -> u32 {
        self.dbnum
    }

    pub fn state(&self) -> anyhow::Result<ParseStageState> {
        let connection = self.lock_connection()?;
        ParseStageState::parse(&metadata_required(&connection, "state")?)
    }

    pub fn counts(&self) -> anyhow::Result<ParseStageCounts> {
        let connection = self.lock_connection()?;
        read_counts(&connection)
    }

    pub fn fact_source(&self) -> StagingTransformFactSource {
        StagingTransformFactSource {
            dbnum: self.dbnum,
            connection: self.connection.clone(),
        }
    }

    pub fn write_batch(&self, batch: &ParsedFactBatch) -> anyhow::Result<ParseWriteReport> {
        batch.validate()?;
        anyhow::ensure!(
            batch.dbnum == self.dbnum,
            "batch dbnum={} 不能写入 stage dbnum={}",
            batch.dbnum,
            self.dbnum
        );
        let batch_hash = batch.canonical_hash();
        let mut connection = self.lock_connection()?;
        let state = ParseStageState::parse(&metadata_required(&connection, "state")?)?;
        anyhow::ensure!(
            state.accepts_fact_writes(),
            "stage state={} 不允许继续写解析事实",
            state.as_str()
        );
        let existing_hash: Option<String> = connection
            .query_row(
                "SELECT batch_hash FROM stage_chunk WHERE batch_id = ?",
                params![&batch.batch_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_hash) = existing_hash {
            anyhow::ensure!(
                existing_hash == batch_hash,
                "batch_id={} 重放内容冲突: existing={} incoming={}",
                batch.batch_id,
                existing_hash,
                batch_hash
            );
            return Ok(ParseWriteReport {
                batch_id: batch.batch_id.clone(),
                batch_hash,
                idempotent: true,
                counts: read_counts(&connection)?,
            });
        }

        connection.execute_batch("BEGIN TRANSACTION;")?;
        let result = write_batch_transaction(&mut connection, batch, &batch_hash);
        match result {
            Ok(()) => connection.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        Ok(ParseWriteReport {
            batch_id: batch.batch_id.clone(),
            batch_hash,
            idempotent: false,
            counts: read_counts(&connection)?,
        })
    }

    pub fn seal_facts(&self) -> anyhow::Result<ParseStageCounts> {
        let connection = self.lock_connection()?;
        let state = ParseStageState::parse(&metadata_required(&connection, "state")?)?;
        if matches!(
            state,
            ParseStageState::FactsSealed
                | ParseStageState::TransformsFinalized
                | ParseStageState::Sealed
                | ParseStageState::AuthorityCommitted
                | ParseStageState::ReplicaApplied
        ) {
            return read_counts(&connection);
        }
        anyhow::ensure!(
            state.accepts_fact_writes(),
            "stage state={} 不能 seal facts",
            state.as_str()
        );
        let counts = read_counts(&connection)?;
        anyhow::ensure!(counts.elements > 0, "parse stage 没有 element");
        anyhow::ensure!(
            counts.catalog_entries == 1,
            "parse stage 必须包含且仅包含一个 db_catalog，实际={}",
            counts.catalog_entries
        );
        set_metadata(&connection, "state", ParseStageState::FactsSealed.as_str())?;
        Ok(counts)
    }

    pub async fn finalize_transforms(&self) -> anyhow::Result<ParseStageCounts> {
        self.seal_facts()?;
        let state = self.state()?;
        if matches!(
            state,
            ParseStageState::TransformsFinalized
                | ParseStageState::Sealed
                | ParseStageState::AuthorityCommitted
                | ParseStageState::ReplicaApplied
        ) {
            return self.counts();
        }
        anyhow::ensure!(
            state == ParseStageState::FactsSealed,
            "stage state={} 不能 finalize transforms",
            state.as_str()
        );

        let (owners, hierarchy) = {
            let connection = self.lock_connection()?;
            (
                load_element_owners(&connection)?,
                load_hierarchy(&connection)?,
            )
        };
        let source: Arc<dyn TransformFactSource> = Arc::new(self.fact_source());
        let transforms = compute_transforms(self.dbnum, &owners, &hierarchy, source).await?;

        let mut connection = self.lock_connection()?;
        connection.execute_batch("BEGIN TRANSACTION;")?;
        let result = (|| -> anyhow::Result<()> {
            connection.execute("DELETE FROM stage_transform", [])?;
            for transform in &transforms {
                let local = transform
                    .local
                    .as_ref()
                    .map(bincode::serialize)
                    .transpose()?;
                let world = bincode::serialize(&transform.world)?;
                connection.execute(
                    "INSERT INTO stage_transform VALUES (?, ?, ?, ?, ?)",
                    params![
                        i64::from(transform.dbnum),
                        transform.refno.to_string(),
                        local,
                        world,
                        hash_serializable(transform)
                    ],
                )?;
            }
            set_metadata(
                &connection,
                "state",
                ParseStageState::TransformsFinalized.as_str(),
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
        let counts = read_counts(&connection)?;
        anyhow::ensure!(
            counts.transforms == counts.elements,
            "transform 覆盖不足: transforms={} elements={}",
            counts.transforms,
            counts.elements
        );
        Ok(counts)
    }

    pub fn seal(&self, version: ParseStageVersion) -> anyhow::Result<SealedParseStage> {
        version.validate()?;
        let connection = self.lock_connection()?;
        let state = ParseStageState::parse(&metadata_required(&connection, "state")?)?;
        if matches!(
            state,
            ParseStageState::Sealed
                | ParseStageState::AuthorityCommitted
                | ParseStageState::ReplicaApplied
        ) {
            let descriptor = read_sealed_descriptor(&connection, &self.path)?;
            anyhow::ensure!(
                descriptor.version == version,
                "sealed stage 版本与请求不一致"
            );
            return Ok(descriptor);
        }
        anyhow::ensure!(
            state == ParseStageState::TransformsFinalized,
            "stage state={} 未完成 transform，不能 seal",
            state.as_str()
        );
        validate_stage_content(&connection, self.dbnum)?;
        let counts = read_counts(&connection)?;
        let rolling_hash = metadata_required(&connection, "rolling_hash")?;
        let fingerprint =
            compute_stage_fingerprint(&connection, &self.run_id, self.dbnum, &version)?;

        connection.execute_batch("BEGIN TRANSACTION;")?;
        let result = (|| -> anyhow::Result<()> {
            set_metadata(&connection, "from_sesno", &version.from_sesno.to_string())?;
            set_metadata(&connection, "to_sesno", &version.to_sesno.to_string())?;
            set_metadata(&connection, "source", &version.source)?;
            set_metadata(
                &connection,
                "source_hash",
                version.source_hash.as_deref().unwrap_or_default(),
            )?;
            set_metadata(&connection, "fingerprint", &fingerprint)?;
            set_metadata(&connection, "state", ParseStageState::Sealed.as_str())?;
            Ok(())
        })();
        match result {
            Ok(()) => connection.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        Ok(SealedParseStage {
            path: self.path.clone(),
            run_id: self.run_id.clone(),
            dbnum: self.dbnum,
            state: ParseStageState::Sealed,
            version,
            fingerprint,
            rolling_hash,
            counts,
            authority_snapshot_id: None,
        })
    }

    pub fn mark_authority_committed(&self, snapshot_id: u64) -> anyhow::Result<SealedParseStage> {
        anyhow::ensure!(snapshot_id > 0, "authority snapshot id 必须非零");
        let connection = self.lock_connection()?;
        let state = ParseStageState::parse(&metadata_required(&connection, "state")?)?;
        anyhow::ensure!(
            matches!(
                state,
                ParseStageState::Sealed
                    | ParseStageState::AuthorityCommitted
                    | ParseStageState::ReplicaApplied
            ),
            "stage state={} 不能标记 authority committed",
            state.as_str()
        );
        if let Some(existing) = metadata_optional(&connection, "authority_snapshot_id")? {
            anyhow::ensure!(
                existing.parse::<u64>()? == snapshot_id,
                "stage 已绑定另一 authority snapshot: {existing}"
            );
        }
        set_metadata(
            &connection,
            "authority_snapshot_id",
            &snapshot_id.to_string(),
        )?;
        if state != ParseStageState::ReplicaApplied {
            set_metadata(
                &connection,
                "state",
                ParseStageState::AuthorityCommitted.as_str(),
            )?;
        }
        read_sealed_descriptor(&connection, &self.path)
    }

    pub fn mark_replica_applied(
        &self,
        replica_version_time: &str,
    ) -> anyhow::Result<SealedParseStage> {
        anyhow::ensure!(
            !replica_version_time.trim().is_empty(),
            "replica_version_time 不能为空"
        );
        let connection = self.lock_connection()?;
        let state = ParseStageState::parse(&metadata_required(&connection, "state")?)?;
        anyhow::ensure!(
            matches!(
                state,
                ParseStageState::AuthorityCommitted | ParseStageState::ReplicaApplied
            ),
            "stage state={} 尚未 authority committed",
            state.as_str()
        );
        if let Some(existing) = metadata_optional(&connection, "replica_version_time")? {
            anyhow::ensure!(
                existing == replica_version_time,
                "stage 已绑定另一 replica version time: {existing}"
            );
        }
        set_metadata(&connection, "replica_version_time", replica_version_time)?;
        set_metadata(
            &connection,
            "state",
            ParseStageState::ReplicaApplied.as_str(),
        )?;
        read_sealed_descriptor(&connection, &self.path)
    }

    fn initialize_or_validate_metadata(&self) -> anyhow::Result<()> {
        let connection = self.lock_connection()?;
        let schema_version = metadata_optional(&connection, "schema_version")?;
        if let Some(schema_version) = schema_version {
            anyhow::ensure!(
                schema_version == STAGE_SCHEMA_VERSION.to_string(),
                "不支持的 parse stage schema: current={schema_version} expected={STAGE_SCHEMA_VERSION}"
            );
            anyhow::ensure!(
                metadata_required(&connection, "run_id")? == self.run_id,
                "parse stage run_id 与路径不一致"
            );
            anyhow::ensure!(
                metadata_required(&connection, "dbnum")? == self.dbnum.to_string(),
                "parse stage dbnum 与路径不一致"
            );
            return Ok(());
        }
        connection.execute_batch("BEGIN TRANSACTION;")?;
        let result = (|| -> anyhow::Result<()> {
            set_metadata(
                &connection,
                "schema_version",
                &STAGE_SCHEMA_VERSION.to_string(),
            )?;
            set_metadata(&connection, "run_id", &self.run_id)?;
            set_metadata(&connection, "dbnum", &self.dbnum.to_string())?;
            set_metadata(&connection, "state", ParseStageState::Created.as_str())?;
            set_metadata(&connection, "rolling_hash", "")?;
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

    fn lock_connection(&self) -> anyhow::Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("parse stage connection mutex poisoned"))
    }
}

#[derive(Clone)]
pub struct StagingTransformFactSource {
    dbnum: u32,
    connection: Arc<Mutex<Connection>>,
}

impl StagingTransformFactSource {
    fn lock_connection(&self) -> anyhow::Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("parse stage source connection mutex poisoned"))
    }
}

#[async_trait]
impl TransformFactSource for StagingTransformFactSource {
    async fn get_attribute(&self, refno: RefnoEnum) -> anyhow::Result<NamedAttrMap> {
        let connection = self.lock_connection()?;
        let row: Option<(u16, Vec<u8>, String)> = connection
            .query_row(
                "SELECT attr_codec_version, attr_payload, attr_hash \
                 FROM stage_element WHERE dbnum = ? AND refno = ?",
                params![i64::from(self.dbnum), refno.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (codec_version, payload, projected_hash) =
            row.ok_or_else(|| anyhow::anyhow!("stage 缺少 attribute: {}", refno))?;
        let attributes = decode_attribute_set_payload(&payload)?;
        attributes.verify()?;
        anyhow::ensure!(
            attributes.refno == refno
                && attributes.codec_version == codec_version
                && attributes.canonical_hash == projected_hash,
            "stage attribute 投影与 payload 不一致: {}",
            refno
        );
        Ok(attributes.to_named_attr_map())
    }

    async fn get_attributes(&self, refnos: &[RefnoEnum]) -> anyhow::Result<Vec<NamedAttrMap>> {
        let mut result = Vec::with_capacity(refnos.len());
        for refno in refnos {
            result.push(self.get_attribute(*refno).await?);
        }
        Ok(result)
    }

    async fn get_children(&self, refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            "SELECT child_refno FROM stage_hierarchy_edge \
             WHERE dbnum = ? AND parent_refno = ? ORDER BY ordinal, child_refno",
        )?;
        let values = statement
            .query_map(params![i64::from(self.dbnum), refno.to_string()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        parse_refnos(values, "stage children")
    }

    async fn get_ancestors(&self, refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let connection = self.lock_connection()?;
        let mut current = refno;
        let mut visited = BTreeSet::from([refno]);
        let mut ancestors = Vec::new();
        loop {
            let owner: Option<String> = connection
                .query_row(
                    "SELECT owner_refno FROM stage_element WHERE dbnum = ? AND refno = ?",
                    params![i64::from(self.dbnum), current.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(owner) = owner else {
                anyhow::bail!("stage ancestor 查询缺少 element: {}", current);
            };
            let owner = parse_refno_allow_unset(&owner, "stage owner")?;
            if owner.is_unset() {
                break;
            }
            if !stage_has_refno(&connection, self.dbnum, owner)? {
                break;
            }
            anyhow::ensure!(visited.insert(owner), "stage hierarchy cycle at {}", owner);
            ancestors.push(owner);
            current = owner;
        }
        ancestors.reverse();
        Ok(ancestors)
    }

    async fn query_pline(
        &self,
        refno: RefnoEnum,
        key: &str,
    ) -> anyhow::Result<Option<PlinParamData>> {
        let connection = self.lock_connection()?;
        let payload: Option<Vec<u8>> = connection
            .query_row(
                "SELECT payload FROM stage_pline_fact \
                 WHERE dbnum = ? AND refno = ? AND pline_key = ?",
                params![i64::from(self.dbnum), refno.to_string(), key],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| bincode::deserialize(&payload).map_err(anyhow::Error::from))
            .transpose()
    }
}

fn write_batch_transaction(
    connection: &mut Connection,
    batch: &ParsedFactBatch,
    batch_hash: &str,
) -> anyhow::Result<()> {
    for item in &batch.elements {
        let content_hash = hash_serializable(item);
        let refno = item.element.refno.to_string();
        let existing_hash: Option<String> = connection
            .query_row(
                "SELECT content_hash FROM stage_element WHERE refno = ?",
                params![&refno],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_hash) = existing_hash {
            anyhow::ensure!(
                existing_hash == content_hash,
                "element 重放冲突: dbnum={} refno={}",
                batch.dbnum,
                item.element.refno
            );
            continue;
        }
        let payload = encode_attribute_set_payload(&item.attributes)?;
        connection.execute(
            "INSERT INTO stage_element VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                i64::from(batch.dbnum),
                &refno,
                item.element.owner.to_string(),
                &item.element.noun,
                &item.element.name,
                item.element.has_children,
                i64::from(item.attributes.codec_version),
                payload,
                &item.attributes.canonical_hash,
                &content_hash
            ],
        )?;
        for edge in item.attributes.reference_edges(batch.dbnum) {
            connection.execute(
                "INSERT INTO stage_reference_edge VALUES (?, ?, ?, ?, ?)",
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

    for row in &batch.hierarchy_rows {
        let existing: Option<(String, u32)> = connection
            .query_row(
                "SELECT parent_refno, ordinal FROM stage_hierarchy_edge WHERE child_refno = ?",
                params![row.child.to_string()],
                |record| Ok((record.get(0)?, record.get(1)?)),
            )
            .optional()?;
        if let Some((parent, ordinal)) = existing {
            anyhow::ensure!(
                parent == row.parent.to_string() && ordinal == row.ordinal,
                "hierarchy 重放冲突: child={} existing_parent={} incoming_parent={}",
                row.child,
                parent,
                row.parent
            );
            continue;
        }
        connection.execute(
            "INSERT INTO stage_hierarchy_edge VALUES (?, ?, ?, ?)",
            params![
                i64::from(row.dbnum),
                row.parent.to_string(),
                row.child.to_string(),
                i64::from(row.ordinal)
            ],
        )?;
    }

    for fact in &batch.pline_facts {
        let content_hash = hash_serializable(fact);
        let existing_hash: Option<String> = connection
            .query_row(
                "SELECT content_hash FROM stage_pline_fact WHERE refno = ? AND pline_key = ?",
                params![fact.refno.to_string(), &fact.key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_hash) = existing_hash {
            anyhow::ensure!(
                existing_hash == content_hash,
                "PLINE fact 重放冲突: refno={} key={}",
                fact.refno,
                fact.key
            );
            continue;
        }
        connection.execute(
            "INSERT INTO stage_pline_fact VALUES (?, ?, ?, ?, ?)",
            params![
                i64::from(batch.dbnum),
                fact.refno.to_string(),
                &fact.key,
                bincode::serialize(&fact.value)?,
                content_hash
            ],
        )?;
    }

    for entry in &batch.db_catalog {
        let content_hash = hash_serializable(entry);
        let existing_hash: Option<String> = connection
            .query_row(
                "SELECT content_hash FROM stage_db_catalog WHERE dbnum = ?",
                params![i64::from(entry.dbnum)],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_hash) = existing_hash {
            anyhow::ensure!(
                existing_hash == content_hash,
                "db_catalog 重放冲突: dbnum={}",
                entry.dbnum
            );
            continue;
        }
        connection.execute(
            "INSERT INTO stage_db_catalog VALUES (?, ?, ?, ?, ?)",
            params![
                i64::from(entry.dbnum),
                entry.ref0.map(i64::from),
                &entry.db_type,
                &entry.project,
                content_hash
            ],
        )?;
    }

    connection.execute(
        "INSERT INTO stage_chunk VALUES (?, ?, ?, ?, ?)",
        params![
            &batch.batch_id,
            batch_hash,
            batch.elements.len() as u64,
            batch.hierarchy_rows.len() as u64,
            batch.pline_facts.len() as u64
        ],
    )?;
    let previous = metadata_required(connection, "rolling_hash")?;
    let rolling_hash = hex::encode(Sha256::digest(
        [previous.as_bytes(), batch_hash.as_bytes()].concat(),
    ));
    set_metadata(connection, "rolling_hash", &rolling_hash)?;
    set_metadata(connection, "state", ParseStageState::Parsing.as_str())?;
    Ok(())
}

async fn compute_transforms(
    dbnum: u32,
    owners: &BTreeMap<RefnoEnum, RefnoEnum>,
    hierarchy: &[HierarchyRow],
    source: Arc<dyn TransformFactSource>,
) -> anyhow::Result<Vec<TransformSnapshot>> {
    let mut children: BTreeMap<RefnoEnum, Vec<(u32, RefnoEnum)>> = BTreeMap::new();
    let mut hierarchy_parent = BTreeMap::new();
    for row in hierarchy {
        anyhow::ensure!(row.dbnum == dbnum, "transform finalize 发现跨库 hierarchy");
        anyhow::ensure!(
            owners.contains_key(&row.parent) && owners.contains_key(&row.child),
            "hierarchy 端点不存在: {} -> {}",
            row.parent,
            row.child
        );
        anyhow::ensure!(
            hierarchy_parent.insert(row.child, row.parent).is_none(),
            "child={} 存在多个父节点",
            row.child
        );
        children
            .entry(row.parent)
            .or_default()
            .push((row.ordinal, row.child));
    }
    for values in children.values_mut() {
        values.sort_by_key(|(ordinal, child)| (*ordinal, *child));
    }

    let mut roots = Vec::new();
    for (refno, owner) in owners {
        if owner.is_unset() || !owners.contains_key(owner) {
            roots.push(*refno);
            continue;
        }
        anyhow::ensure!(
            hierarchy_parent.get(refno) == Some(owner),
            "element={} owner={} 缺少一致的 hierarchy edge",
            refno,
            owner
        );
    }
    roots.sort();
    anyhow::ensure!(!roots.is_empty(), "stage hierarchy 没有根节点，可能存在环");

    let mut queue = VecDeque::new();
    let mut local_by_refno: BTreeMap<RefnoEnum, Option<Transform>> = BTreeMap::new();
    let mut world_by_refno: BTreeMap<RefnoEnum, Transform> = BTreeMap::new();
    for root in roots {
        local_by_refno.insert(root, None);
        world_by_refno.insert(root, Transform::default());
        queue.push_back(root);
    }

    while let Some(parent) = queue.pop_front() {
        let parent_world = world_by_refno
            .get(&parent)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("缺少 parent world transform: {parent}"))?;
        let parent_matrix = parent_world.to_matrix().as_dmat4();
        for (_, child) in children.get(&parent).cloned().unwrap_or_default() {
            anyhow::ensure!(
                !world_by_refno.contains_key(&child),
                "hierarchy cycle 或重复访问: {}",
                child
            );
            let local_matrix =
                aios_core::transform::get_local_mat4_with_source(child, source.clone())
                    .await?
                    .unwrap_or(glam::DMat4::IDENTITY);
            anyhow::ensure!(
                local_matrix.is_finite(),
                "local transform 非有限值: {}",
                child
            );
            let world_matrix = parent_matrix * local_matrix;
            anyhow::ensure!(
                world_matrix.is_finite(),
                "world transform 非有限值: {}",
                child
            );
            local_by_refno.insert(child, Some(Transform::from_matrix(local_matrix.as_mat4())));
            world_by_refno.insert(child, Transform::from_matrix(world_matrix.as_mat4()));
            queue.push_back(child);
        }
    }
    anyhow::ensure!(
        world_by_refno.len() == owners.len(),
        "transform 覆盖不足或 hierarchy 有环: covered={} elements={}",
        world_by_refno.len(),
        owners.len()
    );

    Ok(world_by_refno
        .into_iter()
        .map(|(refno, world)| TransformSnapshot {
            refno,
            dbnum,
            local: local_by_refno.remove(&refno).flatten(),
            world,
        })
        .collect())
}

fn validate_stage_content(connection: &Connection, dbnum: u32) -> anyhow::Result<()> {
    let counts = read_counts(connection)?;
    anyhow::ensure!(counts.elements > 0, "sealed stage 不能为空");
    anyhow::ensure!(
        counts.attributes == counts.elements,
        "AttributeSet 覆盖不足: attributes={} elements={}",
        counts.attributes,
        counts.elements
    );
    anyhow::ensure!(
        counts.transforms == counts.elements,
        "world transform 覆盖不足: transforms={} elements={}",
        counts.transforms,
        counts.elements
    );
    anyhow::ensure!(
        counts.catalog_entries == 1,
        "db_catalog 必须恰好一条，实际={}",
        counts.catalog_entries
    );
    let invalid_hierarchy: u64 = connection.query_row(
        "SELECT count(*) FROM stage_hierarchy_edge h \
         LEFT JOIN stage_element p ON p.refno = h.parent_refno \
         LEFT JOIN stage_element c ON c.refno = h.child_refno \
         WHERE h.dbnum != ? OR p.refno IS NULL OR c.refno IS NULL",
        params![i64::from(dbnum)],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        invalid_hierarchy == 0,
        "hierarchy 存在 {invalid_hierarchy} 个不可解析端点"
    );
    let invalid_references: u64 = connection.query_row(
        "SELECT count(*) FROM stage_reference_edge r \
         LEFT JOIN stage_element s ON s.refno = r.source_refno \
         LEFT JOIN stage_element t ON t.refno = r.target_refno \
         WHERE r.dbnum != ? OR s.refno IS NULL OR t.refno IS NULL",
        params![i64::from(dbnum)],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        invalid_references == 0,
        "reference 存在 {invalid_references} 个不可解析端点"
    );
    Ok(())
}

fn read_verified_payload(
    connection: &Connection,
    expected_dbnum: u32,
) -> anyhow::Result<StagedParsePayload> {
    let element_rows = query_rows(
        connection,
        "SELECT dbnum, refno, owner_refno, noun, name, has_children, attr_codec_version, \
         attr_payload, attr_hash, content_hash FROM stage_element ORDER BY refno",
        |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, u16>(6)?,
                row.get::<_, Vec<u8>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        },
    )?;
    let mut elements = Vec::with_capacity(element_rows.len());
    for (
        dbnum,
        refno,
        owner,
        noun,
        name,
        has_children,
        codec_version,
        attr_payload,
        attr_hash,
        content_hash,
    ) in element_rows
    {
        anyhow::ensure!(
            dbnum == expected_dbnum,
            "stage element 跨库: expected={expected_dbnum} actual={dbnum}"
        );
        let refno = parse_refno(&refno, "stage element")?;
        let owner = parse_refno_allow_unset(&owner, "stage element owner")?;
        let attributes = decode_attribute_set_payload(&attr_payload)?;
        attributes.verify()?;
        anyhow::ensure!(
            attributes.refno == refno
                && attributes.codec_version == codec_version
                && attributes.canonical_hash == attr_hash,
            "stage attribute 投影与 payload 不一致: {refno}"
        );
        let item = VersionStoreElement {
            element: crate::generation_read::ElementSnapshot {
                refno,
                dbnum,
                owner,
                noun,
                name,
                has_children,
            },
            attributes,
        };
        anyhow::ensure!(
            hash_serializable(&item) == content_hash,
            "stage element content hash 不一致: {refno}"
        );
        elements.push(item);
    }

    let hierarchy_rows = load_hierarchy(connection)?;
    anyhow::ensure!(
        hierarchy_rows.iter().all(|row| row.dbnum == expected_dbnum),
        "stage hierarchy 存在跨库记录"
    );

    let transform_rows = query_rows(
        connection,
        "SELECT dbnum, refno, local_transform, world_transform, transform_hash \
         FROM stage_transform ORDER BY refno",
        |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    let mut transforms = Vec::with_capacity(transform_rows.len());
    for (dbnum, refno, local, world, transform_hash) in transform_rows {
        anyhow::ensure!(
            dbnum == expected_dbnum,
            "stage transform 跨库: expected={expected_dbnum} actual={dbnum}"
        );
        let transform = TransformSnapshot {
            refno: parse_refno(&refno, "stage transform")?,
            dbnum,
            local: local
                .map(|payload| bincode::deserialize(&payload))
                .transpose()?,
            world: bincode::deserialize(&world)?,
        };
        anyhow::ensure!(
            hash_serializable(&transform) == transform_hash,
            "stage transform hash 不一致: {}",
            transform.refno
        );
        transforms.push(transform);
    }

    let catalog_rows = query_rows(
        connection,
        "SELECT dbnum, ref0, db_type, project, content_hash \
         FROM stage_db_catalog ORDER BY dbnum",
        |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, Option<u32>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    let mut db_catalog = Vec::with_capacity(catalog_rows.len());
    for (dbnum, ref0, db_type, project, content_hash) in catalog_rows {
        anyhow::ensure!(
            dbnum == expected_dbnum,
            "stage catalog 跨库: expected={expected_dbnum} actual={dbnum}"
        );
        let entry = DbCatalogEntry {
            dbnum,
            ref0,
            db_type,
            project,
        };
        anyhow::ensure!(
            hash_serializable(&entry) == content_hash,
            "stage db_catalog content hash 不一致: dbnum={dbnum}"
        );
        db_catalog.push(entry);
    }

    Ok(StagedParsePayload {
        elements,
        hierarchy_rows,
        transforms,
        db_catalog,
    })
}

fn compute_stage_fingerprint(
    connection: &Connection,
    run_id: &str,
    dbnum: u32,
    version: &ParseStageVersion,
) -> anyhow::Result<String> {
    let elements = query_rows(
        connection,
        "SELECT refno, owner_refno, noun, name, has_children, attr_codec_version, \
         attr_payload, attr_hash, content_hash FROM stage_element ORDER BY refno",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, u16>(5)?,
                row.get::<_, Vec<u8>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        },
    )?;
    let hierarchy = query_rows(
        connection,
        "SELECT parent_refno, child_refno, ordinal FROM stage_hierarchy_edge \
         ORDER BY parent_refno, ordinal, child_refno",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        },
    )?;
    let references = query_rows(
        connection,
        "SELECT source_refno, attribute_name, target_refno, ordinal \
         FROM stage_reference_edge ORDER BY source_refno, attribute_name, ordinal",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
            ))
        },
    )?;
    let transforms = query_rows(
        connection,
        "SELECT refno, local_transform, world_transform, transform_hash \
         FROM stage_transform ORDER BY refno",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    let catalog = query_rows(
        connection,
        "SELECT dbnum, ref0, db_type, project, content_hash FROM stage_db_catalog ORDER BY dbnum",
        |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, Option<u32>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    Ok(hash_serializable(&(
        STAGE_SCHEMA_VERSION,
        run_id,
        dbnum,
        version,
        elements,
        hierarchy,
        references,
        transforms,
        catalog,
    )))
}

fn read_sealed_descriptor(
    connection: &Connection,
    path: &Path,
) -> anyhow::Result<SealedParseStage> {
    let state = ParseStageState::parse(&metadata_required(connection, "state")?)?;
    anyhow::ensure!(
        matches!(
            state,
            ParseStageState::Sealed
                | ParseStageState::AuthorityCommitted
                | ParseStageState::ReplicaApplied
        ),
        "stage 尚未 sealed"
    );
    Ok(SealedParseStage {
        path: path.to_path_buf(),
        run_id: metadata_required(connection, "run_id")?,
        dbnum: metadata_required(connection, "dbnum")?.parse()?,
        state,
        version: ParseStageVersion {
            from_sesno: metadata_required(connection, "from_sesno")?.parse()?,
            to_sesno: metadata_required(connection, "to_sesno")?.parse()?,
            source: metadata_required(connection, "source")?,
            source_hash: metadata_optional(connection, "source_hash")?
                .filter(|value| !value.is_empty()),
        },
        fingerprint: metadata_required(connection, "fingerprint")?,
        rolling_hash: metadata_required(connection, "rolling_hash")?,
        counts: read_counts(connection)?,
        authority_snapshot_id: metadata_optional(connection, "authority_snapshot_id")?
            .map(|value| value.parse())
            .transpose()?,
    })
}

fn load_element_owners(connection: &Connection) -> anyhow::Result<BTreeMap<RefnoEnum, RefnoEnum>> {
    let mut statement =
        connection.prepare("SELECT refno, owner_refno FROM stage_element ORDER BY refno")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(refno, owner)| {
            Ok((
                parse_refno(&refno, "stage element refno")?,
                parse_refno_allow_unset(&owner, "stage element owner")?,
            ))
        })
        .collect()
}

fn load_hierarchy(connection: &Connection) -> anyhow::Result<Vec<HierarchyRow>> {
    let mut statement = connection.prepare(
        "SELECT dbnum, parent_refno, child_refno, ordinal \
         FROM stage_hierarchy_edge ORDER BY parent_refno, ordinal, child_refno",
    )?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(dbnum, parent, child, ordinal)| {
            Ok(HierarchyRow {
                dbnum,
                parent: parse_refno(&parent, "stage hierarchy parent")?,
                child: parse_refno(&child, "stage hierarchy child")?,
                ordinal,
            })
        })
        .collect()
}

fn read_counts(connection: &Connection) -> anyhow::Result<ParseStageCounts> {
    Ok(ParseStageCounts {
        chunks: count_rows(connection, "stage_chunk")?,
        elements: count_rows(connection, "stage_element")?,
        attributes: count_rows(connection, "stage_element")?,
        hierarchy_edges: count_rows(connection, "stage_hierarchy_edge")?,
        reference_edges: count_rows(connection, "stage_reference_edge")?,
        pline_facts: count_rows(connection, "stage_pline_fact")?,
        transforms: count_rows(connection, "stage_transform")?,
        catalog_entries: count_rows(connection, "stage_db_catalog")?,
    })
}

fn count_rows(connection: &Connection, table: &str) -> anyhow::Result<u64> {
    Ok(
        connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })?,
    )
}

fn metadata_required(connection: &Connection, key: &str) -> anyhow::Result<String> {
    metadata_optional(connection, key)?
        .ok_or_else(|| anyhow::anyhow!("parse stage metadata 缺少 key={key}"))
}

fn metadata_optional(connection: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT value FROM stage_metadata WHERE key = ?",
            params![key],
            |row| row.get(0),
        )
        .optional()?)
}

fn set_metadata(connection: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
    connection.execute(
        "INSERT OR REPLACE INTO stage_metadata(key, value) VALUES (?, ?)",
        params![key, value],
    )?;
    Ok(())
}

fn stage_has_refno(connection: &Connection, dbnum: u32, refno: RefnoEnum) -> anyhow::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM stage_element WHERE dbnum = ? AND refno = ?",
            params![i64::from(dbnum), refno.to_string()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn parse_refnos(values: Vec<String>, context: &str) -> anyhow::Result<Vec<RefnoEnum>> {
    values
        .into_iter()
        .map(|value| parse_refno(&value, context))
        .collect()
}

fn parse_refno(value: &str, context: &str) -> anyhow::Result<RefnoEnum> {
    let refno = RefnoEnum::from(value);
    anyhow::ensure!(refno.is_valid(), "{context} 包含非法 refno={value}");
    Ok(refno)
}

fn parse_refno_allow_unset(value: &str, context: &str) -> anyhow::Result<RefnoEnum> {
    let refno = RefnoEnum::from(value);
    anyhow::ensure!(
        refno.is_valid() || refno.is_unset(),
        "{context} 包含非法 refno={value}"
    );
    Ok(refno)
}

fn validate_run_id(run_id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!run_id.trim().is_empty(), "run_id 不能为空");
    anyhow::ensure!(
        !run_id.contains("..")
            && run_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character)),
        "run_id 只能包含 ASCII 字母、数字、点、下划线和连字符"
    );
    Ok(())
}

fn query_rows<T, F>(connection: &Connection, sql: &str, mut map: F) -> anyhow::Result<Vec<T>>
where
    F: FnMut(&duckdb::Row<'_>) -> duckdb::Result<T>,
{
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([], |row| map(row))?
        .collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_read::ElementSnapshot;
    use aios_core::{AttrVal, NamedAttrMap};

    fn refno(value: &str) -> RefnoEnum {
        RefnoEnum::from(value)
    }

    fn fixture_element(
        dbnum: u32,
        refno: RefnoEnum,
        owner: RefnoEnum,
        noun: &str,
        position: Option<[f64; 3]>,
        has_children: bool,
    ) -> VersionStoreElement {
        let mut attributes = NamedAttrMap::new(noun);
        attributes.insert(
            "REFNO".to_string(),
            AttrVal::RefU64Type(refno.refno()).into(),
        );
        attributes.insert(
            "OWNER".to_string(),
            AttrVal::RefU64Type(owner.refno()).into(),
        );
        if let Some(position) = position {
            attributes.insert("POS".to_string(), AttrVal::Vec3Type(position).into());
        }
        VersionStoreElement {
            element: ElementSnapshot {
                refno,
                dbnum,
                owner,
                noun: noun.to_string(),
                name: format!("/{noun}-{refno}"),
                has_children,
            },
            attributes: AttributeSet::from_named_attr_map(refno, &attributes),
        }
    }

    fn catalog(dbnum: u32) -> DbCatalogEntry {
        DbCatalogEntry {
            dbnum,
            ref0: Some(dbnum * 10),
            db_type: "DESI".to_string(),
            project: "fixture".to_string(),
        }
    }

    fn two_element_batch(dbnum: u32) -> ParsedFactBatch {
        let root = refno("10/1");
        let child = refno("10/2");
        ParsedFactBatch {
            batch_id: "chunk-0001".to_string(),
            dbnum,
            elements: vec![
                fixture_element(dbnum, root, RefnoEnum::default(), "WORLD", None, true),
                fixture_element(dbnum, child, root, "EQUI", Some([1.0, 2.0, 3.0]), false),
            ],
            hierarchy_rows: vec![HierarchyRow {
                dbnum,
                parent: root,
                child,
                ordinal: 0,
            }],
            pline_facts: Vec::new(),
            db_catalog: vec![catalog(dbnum)],
        }
    }

    fn version() -> ParseStageVersion {
        ParseStageVersion {
            from_sesno: 1,
            to_sesno: 20,
            source: "total".to_string(),
            source_hash: Some("fixture-source".to_string()),
        }
    }

    #[tokio::test]
    async fn stage_replay_finalize_and_seal_are_fail_closed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stager = DuckLakeParseStager::open(temp.path(), "run-a", 7997).expect("stage");
        let batch = two_element_batch(7997);

        let first = stager.write_batch(&batch).expect("first write");
        assert!(!first.idempotent);
        let replay = stager.write_batch(&batch).expect("idempotent replay");
        assert!(replay.idempotent);

        let mut conflicting = batch.clone();
        conflicting.elements[1].element.name = "/different".to_string();
        assert!(stager.write_batch(&conflicting).is_err());

        let counts = stager.finalize_transforms().await.expect("finalize");
        assert_eq!(counts.elements, 2);
        assert_eq!(counts.transforms, 2);
        let sealed = stager.seal(version()).expect("seal");
        assert_eq!(sealed.state, ParseStageState::Sealed);
        assert!(!sealed.fingerprint.is_empty());
        assert!(stager.write_batch(&batch).is_err());

        let stage_path = stager.path().to_path_buf();
        drop(stager);
        sealed.verify().expect("verify sealed stage");
        let reopened = DuckLakeParseStager::open_path(stage_path).expect("reopen");
        assert_eq!(reopened.state().expect("state"), ParseStageState::Sealed);
        assert_eq!(
            reopened
                .seal(version())
                .expect("idempotent seal")
                .fingerprint,
            sealed.fingerprint
        );
    }

    #[tokio::test]
    async fn sealed_stage_rejects_payload_corruption_on_reopen() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stager = DuckLakeParseStager::open(temp.path(), "run-corrupt", 7997).expect("stage");
        stager
            .write_batch(&two_element_batch(7997))
            .expect("write batch");
        stager.finalize_transforms().await.expect("finalize");
        let sealed = stager.seal(version()).expect("seal");
        drop(stager);

        let connection = Connection::open(&sealed.path).expect("open stage for corruption");
        connection
            .execute(
                "UPDATE stage_element SET attr_payload = ? WHERE refno = ?",
                params![vec![0xff_u8, 0x00], refno("10/2").to_string()],
            )
            .expect("corrupt payload");
        drop(connection);

        assert!(sealed.verify().is_err());
    }

    #[tokio::test]
    async fn dbnum_stages_isolate_same_refno_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = DuckLakeParseStager::open(temp.path(), "run-b", 1).expect("db1");
        let second = DuckLakeParseStager::open(temp.path(), "run-b", 2).expect("db2");
        let root = refno("10/1");
        for (stager, dbnum) in [(&first, 1), (&second, 2)] {
            stager
                .write_batch(&ParsedFactBatch {
                    batch_id: "chunk".to_string(),
                    dbnum,
                    elements: vec![fixture_element(
                        dbnum,
                        root,
                        RefnoEnum::default(),
                        "WORLD",
                        None,
                        false,
                    )],
                    hierarchy_rows: Vec::new(),
                    pline_facts: Vec::new(),
                    db_catalog: vec![catalog(dbnum)],
                })
                .expect("write");
            stager.finalize_transforms().await.expect("finalize");
        }
        let first = first.seal(version()).expect("seal db1");
        let second = second.seal(version()).expect("seal db2");
        assert_ne!(first.path, second.path);
        assert_ne!(first.fingerprint, second.fingerprint);
        assert_eq!(first.counts.elements, 1);
        assert_eq!(second.counts.elements, 1);
    }

    #[tokio::test]
    async fn hierarchy_cycle_blocks_transform_finalize() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stager = DuckLakeParseStager::open(temp.path(), "run-cycle", 7).expect("stage");
        let first = refno("10/1");
        let second = refno("10/2");
        stager
            .write_batch(&ParsedFactBatch {
                batch_id: "cycle".to_string(),
                dbnum: 7,
                elements: vec![
                    fixture_element(7, first, second, "EQUI", None, true),
                    fixture_element(7, second, first, "EQUI", None, true),
                ],
                hierarchy_rows: vec![
                    HierarchyRow {
                        dbnum: 7,
                        parent: first,
                        child: second,
                        ordinal: 0,
                    },
                    HierarchyRow {
                        dbnum: 7,
                        parent: second,
                        child: first,
                        ordinal: 0,
                    },
                ],
                pline_facts: Vec::new(),
                db_catalog: vec![catalog(7)],
            })
            .expect("write");
        assert!(stager.finalize_transforms().await.is_err());
        assert_eq!(stager.state().expect("state"), ParseStageState::FactsSealed);
    }
}
