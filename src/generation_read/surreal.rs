use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use aios_core::rs_surreal::PlantTransform;
use aios_core::{NamedAttrMap, RefnoEnum, SPdmsElement, SurrealQueryExt, project_primary_db};
use async_trait::async_trait;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use super::error::{GenerationReadError, GenerationReadResult};
use super::traits::{
    AttributeRead, CatalogGraphRead, ElementRead, GenerationReadBackend, HierarchyRead,
    TransformRead, VersionedReadSession,
};
use super::types::{
    AttributeSet, BatchLookup, CatalogNode, ElementQuery, ElementSnapshot,
    GenerationReadBackendKind, HierarchyRow, InputVersionManifest, SessionMetricsSnapshot,
    TransformSnapshot,
};

const QUERY_CHUNK_SIZE: usize = 500;

/// Surreal main-table generation reader.
///
/// `read_at=None` is reserved for initialization against an isolated staging
/// database. Incremental, catch-up, and repair runs pass one data-anchor time;
/// every query emitted by the session then carries the same `VERSION` suffix.
#[derive(Debug, Clone, Default)]
pub struct SurrealVersionedReadBackend {
    read_at: Option<String>,
}

impl SurrealVersionedReadBackend {
    pub fn new(read_at: Option<String>) -> Self {
        Self { read_at }
    }
}

pub struct SurrealVersionedReadSession {
    manifest: Arc<InputVersionManifest>,
    version_suffix: String,
    metrics: Mutex<SessionMetricsSnapshot>,
    attribute_cache: Mutex<BTreeMap<RefnoEnum, AttributeSet>>,
}

#[async_trait]
impl GenerationReadBackend for SurrealVersionedReadBackend {
    fn backend_kind(&self) -> GenerationReadBackendKind {
        GenerationReadBackendKind::Surreal
    }

    async fn open_session(
        &self,
        manifest: Arc<InputVersionManifest>,
    ) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
        let version_suffix = main_table_version_suffix(self.read_at.as_deref())?;
        Ok(Arc::new(SurrealVersionedReadSession {
            manifest,
            version_suffix,
            metrics: Mutex::new(SessionMetricsSnapshot::default()),
            attribute_cache: Mutex::new(BTreeMap::new()),
        }))
    }
}

impl VersionedReadSession for SurrealVersionedReadSession {
    fn manifest(&self) -> &InputVersionManifest {
        &self.manifest
    }

    fn backend_kind(&self) -> GenerationReadBackendKind {
        GenerationReadBackendKind::Surreal
    }

    fn metrics(&self) -> SessionMetricsSnapshot {
        self.metrics
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl ElementRead for SurrealVersionedReadSession {
    async fn load_elements(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<ElementSnapshot>> {
        let started = Instant::now();
        if refnos.is_empty() {
            return Ok(BatchLookup::default());
        }
        let requested = refnos.iter().copied().collect::<BTreeSet<_>>();
        let rows = self.load_pe_rows_by_refnos(&requested).await?;
        let found = rows
            .into_iter()
            .map(element_from_pe)
            .collect::<GenerationReadResult<Vec<_>>>()?;
        self.record(
            "element.load",
            requested.len(),
            found.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(BatchLookup::from_found(
            refnos,
            found.into_iter().map(|element| (element.refno, element)),
        ))
    }

    async fn query_elements(
        &self,
        query: &ElementQuery,
    ) -> GenerationReadResult<Vec<ElementSnapshot>> {
        let started = Instant::now();
        let rows = self.load_pe_rows_by_query(query).await?;
        let elements = rows
            .into_iter()
            .map(element_from_pe)
            .collect::<GenerationReadResult<Vec<_>>>()?;
        self.record(
            "element.query",
            query.dbnums.len(),
            elements.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(elements)
    }
}

#[async_trait]
impl AttributeRead for SurrealVersionedReadSession {
    async fn load_attribute_sets(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<AttributeSet>> {
        let started = Instant::now();
        if refnos.is_empty() {
            return Ok(BatchLookup::default());
        }
        let requested = refnos.iter().copied().collect::<BTreeSet<_>>();
        let mut found = self
            .attribute_cache
            .lock()
            .map(|cache| {
                requested
                    .iter()
                    .filter_map(|refno| cache.get(refno).cloned().map(|value| (*refno, value)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cached = found
            .iter()
            .map(|(refno, _)| *refno)
            .collect::<BTreeSet<_>>();
        let missing = requested
            .iter()
            .filter(|refno| !cached.contains(refno))
            .copied()
            .collect::<BTreeSet<_>>();
        if missing.is_empty() {
            return Ok(BatchLookup::from_found(refnos, found));
        }
        let pe_rows = self.load_pe_rows_by_refnos(&missing).await?;
        let mut loaded = Vec::with_capacity(pe_rows.len());

        for chunk in pe_rows.chunks(QUERY_CHUNK_SIZE) {
            let record_ids = chunk
                .iter()
                .map(|pe| pe.refno.to_table_key(&pe.noun))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT * FROM [{record_ids}] ORDER BY id{};",
                self.version_suffix
            );
            let mut response = project_primary_db()
                .query(sql)
                .await
                .map_err(|error| backend_error("attribute.load", error))?
                .check()
                .map_err(|error| backend_error("attribute.load", error))?;
            let attributes: Vec<NamedAttrMap> = response
                .take(0)
                .map_err(|error| backend_error("attribute.decode", error))?;
            for attributes in attributes {
                let refno = RefnoEnum::from(attributes.get_refno_or_default());
                if missing.contains(&refno) {
                    loaded.push((refno, AttributeSet::from_named_attr_map(refno, &attributes)));
                }
            }
        }

        self.record(
            "attribute.load",
            missing.len(),
            loaded.len(),
            started.elapsed().as_micros() as u64,
        );
        if let Ok(mut cache) = self.attribute_cache.lock() {
            cache.extend(loaded.iter().cloned());
        }
        found.extend(loaded);
        Ok(BatchLookup::from_found(refnos, found))
    }
}

#[async_trait]
impl HierarchyRead for SurrealVersionedReadSession {
    async fn load_hierarchy_rows(&self, dbnums: &[u32]) -> GenerationReadResult<Vec<HierarchyRow>> {
        let started = Instant::now();
        if dbnums.is_empty() {
            return Ok(Vec::new());
        }
        let query = ElementQuery {
            dbnums: dbnums.iter().copied().collect(),
            ..ElementQuery::default()
        };
        let pe_rows = self.load_pe_rows_by_query(&query).await?;
        let known = pe_rows.iter().map(|pe| pe.refno).collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        for pe in pe_rows {
            let dbnum = checked_u32(pe.dbnum, "hierarchy.dbnum")?;
            for (ordinal, child) in pe.children.unwrap_or_default().into_iter().enumerate() {
                let child = RefnoEnum::from(child);
                // A partial CATA closure can contain links to records which were
                // intentionally not materialized. They are catalog references,
                // not hierarchy nodes in this generation manifest.
                if known.contains(&child) {
                    rows.push(HierarchyRow {
                        dbnum,
                        parent: pe.refno,
                        child,
                        ordinal: ordinal as u32,
                    });
                }
            }
        }
        self.record(
            "hierarchy.load",
            dbnums.len(),
            rows.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(rows)
    }
}

#[async_trait]
impl CatalogGraphRead for SurrealVersionedReadSession {
    async fn load_catalog_nodes(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<CatalogNode>> {
        let started = Instant::now();
        if refnos.is_empty() {
            return Ok(BatchLookup::default());
        }
        let requested = refnos.iter().copied().collect::<BTreeSet<_>>();
        let pe_rows = self.load_pe_rows_by_refnos(&requested).await?;
        let attributes = self.load_attribute_sets(refnos).await?;
        let requested_dbnums = pe_rows
            .iter()
            .filter_map(|pe| u32::try_from(pe.dbnum).ok())
            .collect::<BTreeSet<_>>();
        let db_types = self.load_db_types(&requested_dbnums).await?;
        let mut found = Vec::with_capacity(pe_rows.len());

        for pe in pe_rows {
            let dbnum = checked_u32(pe.dbnum, "catalog.dbnum")?;
            let attributes = attributes.found.get(&pe.refno).ok_or_else(|| {
                GenerationReadError::MissingRequiredData {
                    capability: "catalog.attributes",
                    refnos: vec![pe.refno],
                }
            })?;
            let db_type = db_types.get(&dbnum).cloned().ok_or_else(|| {
                GenerationReadError::MissingRequiredData {
                    capability: "catalog.db_type",
                    refnos: vec![pe.refno],
                }
            })?;
            let node = CatalogNode {
                refno: pe.refno,
                dbnum,
                db_type,
                noun: pe.noun,
                owner: pe.owner,
                children: pe
                    .children
                    .unwrap_or_default()
                    .into_iter()
                    .map(RefnoEnum::from)
                    .collect(),
                outbound: attributes.reference_edges(dbnum),
            };
            found.push((node.refno, node));
        }

        self.record(
            "catalog.load",
            requested.len(),
            found.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(BatchLookup::from_found(refnos, found))
    }
}

#[async_trait]
impl TransformRead for SurrealVersionedReadSession {
    async fn load_transforms(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<TransformSnapshot>> {
        let started = Instant::now();
        if refnos.is_empty() {
            return Ok(BatchLookup::default());
        }
        let requested = refnos.iter().copied().collect::<BTreeSet<_>>();
        let pe_rows = self.load_pe_rows_by_refnos(&requested).await?;
        let dbnums = pe_rows
            .into_iter()
            .filter_map(|pe| u32::try_from(pe.dbnum).ok().map(|dbnum| (pe.refno, dbnum)))
            .collect::<BTreeMap<_, _>>();
        let mut found = Vec::new();

        for chunk in requested
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .chunks(QUERY_CHUNK_SIZE)
        {
            let ids = chunk
                .iter()
                .map(|refno| refno.to_table_key("pe_transform"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT meta::id(id) AS refno, local_trans.d AS local, \
                 world_trans.d AS world FROM [{ids}]{};",
                self.version_suffix
            );
            let mut response = project_primary_db()
                .query(sql)
                .await
                .map_err(|error| backend_error("transform.load", error))?
                .check()
                .map_err(|error| backend_error("transform.load", error))?;
            let rows: Vec<MainTransformRow> = response
                .take(0)
                .map_err(|error| backend_error("transform.decode", error))?;
            for row in rows {
                let Some(world) = row.world else {
                    continue;
                };
                let Some(dbnum) = dbnums.get(&row.refno).copied() else {
                    continue;
                };
                let snapshot = TransformSnapshot {
                    refno: row.refno,
                    dbnum,
                    local: row.local.map(|value| value.0),
                    world: world.0,
                };
                found.push((snapshot.refno, snapshot));
            }
        }

        self.record(
            "transform.load",
            requested.len(),
            found.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(BatchLookup::from_found(refnos, found))
    }
}

impl SurrealVersionedReadSession {
    async fn load_pe_rows_by_refnos(
        &self,
        refnos: &BTreeSet<RefnoEnum>,
    ) -> GenerationReadResult<Vec<SPdmsElement>> {
        let mut rows = Vec::new();
        let ordered = refnos.iter().copied().collect::<Vec<_>>();
        for chunk in ordered.chunks(QUERY_CHUNK_SIZE) {
            let ids = chunk
                .iter()
                .map(|refno| refno.to_pe_key())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT * FROM [{ids}] WHERE deleted = false OR deleted = NONE \
                 ORDER BY dbnum, id{};",
                self.version_suffix
            );
            rows.extend(self.query_pe_rows(sql, "element.load_rows").await?);
        }
        Ok(rows)
    }

    async fn load_pe_rows_by_query(
        &self,
        query: &ElementQuery,
    ) -> GenerationReadResult<Vec<SPdmsElement>> {
        let mut clauses = vec!["(deleted = false OR deleted = NONE)".to_string()];
        if !query.dbnums.is_empty() {
            clauses.push(format!(
                "dbnum IN [{}]",
                query
                    .dbnums
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if !query.nouns.is_empty() {
            clauses.push(format!(
                "string::uppercase(noun) IN [{}]",
                query
                    .nouns
                    .iter()
                    .map(|noun| surreal_string(&noun.to_ascii_uppercase()))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if let Some(has_children) = query.has_children {
            let predicate = if has_children {
                "array::len(children) > 0"
            } else {
                "array::len(children) = 0"
            };
            clauses.push(predicate.to_string());
        }
        let sql = format!(
            "SELECT * FROM pe WHERE {} ORDER BY dbnum, id{};",
            clauses.join(" AND "),
            self.version_suffix
        );
        self.query_pe_rows(sql, "element.query_rows").await
    }

    async fn query_pe_rows(
        &self,
        sql: String,
        operation: &'static str,
    ) -> GenerationReadResult<Vec<SPdmsElement>> {
        let mut response = project_primary_db()
            .query(sql)
            .await
            .map_err(|error| backend_error(operation, error))?
            .check()
            .map_err(|error| backend_error(operation, error))?;
        response
            .take(0)
            .map_err(|error| backend_error("element.decode_rows", error))
    }

    async fn load_db_types(
        &self,
        dbnums: &BTreeSet<u32>,
    ) -> GenerationReadResult<BTreeMap<u32, String>> {
        let sql = format!(
            "SELECT dbnum, db_type FROM dbnum_info_table WHERE dbnum IN [{}] \
             ORDER BY dbnum{};",
            dbnums
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
            self.version_suffix
        );
        let mut response = project_primary_db()
            .query(sql)
            .await
            .map_err(|error| backend_error("catalog.db_types", error))?
            .check()
            .map_err(|error| backend_error("catalog.db_types", error))?;
        let rows: Vec<DbTypeRow> = response
            .take(0)
            .map_err(|error| backend_error("catalog.db_types.decode", error))?;
        let mut db_types = BTreeMap::new();
        for row in rows {
            if let Ok(dbnum) = u32::try_from(row.dbnum)
                && !row.db_type.trim().is_empty()
            {
                db_types.entry(dbnum).or_insert(row.db_type);
            }
        }
        Ok(db_types)
    }

    fn record(&self, capability: &str, requested: usize, returned: usize, elapsed_micros: u64) {
        let Ok(mut metrics) = self.metrics.lock() else {
            return;
        };
        *metrics
            .backend_calls
            .entry(capability.to_string())
            .or_default() += 1;
        *metrics
            .requested_keys
            .entry(capability.to_string())
            .or_default() += requested as u64;
        *metrics
            .returned_rows
            .entry(capability.to_string())
            .or_default() += returned as u64;
        *metrics
            .elapsed_micros
            .entry(capability.to_string())
            .or_default() += elapsed_micros;
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct MainTransformRow {
    refno: RefnoEnum,
    #[serde(default)]
    local: Option<PlantTransform>,
    #[serde(default)]
    world: Option<PlantTransform>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct DbTypeRow {
    dbnum: i64,
    #[serde(default)]
    db_type: String,
}

fn element_from_pe(pe: SPdmsElement) -> GenerationReadResult<ElementSnapshot> {
    let children = pe
        .children
        .unwrap_or_default()
        .into_iter()
        .map(RefnoEnum::from)
        .collect::<Vec<_>>();
    Ok(ElementSnapshot {
        refno: pe.refno,
        dbnum: checked_u32(pe.dbnum, "element.dbnum")?,
        owner: pe.owner,
        noun: pe.noun,
        name: pe.name,
        has_children: !children.is_empty(),
        children,
    })
}

fn checked_u32(value: i32, field: &'static str) -> GenerationReadResult<u32> {
    u32::try_from(value).map_err(|_| GenerationReadError::BackendQuery {
        backend: "surreal-main",
        operation: field,
        message: format!("value {value} is outside u32"),
    })
}

fn surreal_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn main_table_version_suffix(read_at: Option<&str>) -> GenerationReadResult<String> {
    let Some(read_at) = read_at else {
        return Ok(String::new());
    };
    if read_at.is_empty() || read_at.contains('\'') || read_at.contains('\0') {
        return Err(GenerationReadError::InvalidReadSpec(
            "read_at contains invalid characters".to_string(),
        ));
    }
    // Surreal 3.x requires VERSION after WHERE/ORDER BY.
    Ok(format!(" VERSION d'{read_at}'"))
}

fn backend_error(operation: &'static str, error: impl std::fmt::Display) -> GenerationReadError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let history_expired = lower.contains("invalidargument")
        || lower.contains("invalid argument")
        || lower.contains("below the garbage collection")
        || lower.contains("full_history_ts_low")
        || lower.contains("retention")
            && (lower.contains("version") || lower.contains("history") || lower.contains("gc"));
    if history_expired {
        return GenerationReadError::HistoryExpired { operation, message };
    }
    GenerationReadError::BackendQuery {
        backend: "surreal-main",
        operation,
        message,
    }
}
