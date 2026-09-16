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
const QUERY_PAGE_SIZE: usize = 5_000;

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
        let dbnums = rows
            .iter()
            .map(|pe| checked_u32(pe.dbnum, "element.dbnum"))
            .collect::<GenerationReadResult<BTreeSet<_>>>()?;
        self.ensure_hierarchy_coverage(&dbnums).await?;
        let parents = rows.iter().map(|pe| pe.refno).collect::<BTreeSet<_>>();
        let children = self.load_children_by_parents(&parents).await?;
        let found = rows
            .into_iter()
            .map(|pe| {
                let pe_children = children.get(&pe.refno).cloned().unwrap_or_default();
                element_from_pe(pe, pe_children)
            })
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
        let dbnums = rows
            .iter()
            .map(|pe| checked_u32(pe.dbnum, "element.dbnum"))
            .collect::<GenerationReadResult<BTreeSet<_>>>()?;
        self.ensure_hierarchy_coverage(&dbnums).await?;
        let parents = rows.iter().map(|pe| pe.refno).collect::<BTreeSet<_>>();
        let children = self.load_children_by_parents(&parents).await?;
        let elements = rows
            .into_iter()
            .map(|pe| {
                let pe_children = children.get(&pe.refno).cloned().unwrap_or_default();
                element_from_pe(pe, pe_children)
            })
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
        self.ensure_hierarchy_coverage(&dbnums.iter().copied().collect())
            .await?;
        let query = ElementQuery {
            dbnums: dbnums.iter().copied().collect(),
            ..ElementQuery::default()
        };
        let pe_rows = self.load_pe_rows_by_query(&query).await?;
        let mut known = BTreeMap::new();
        for pe in &pe_rows {
            known.insert(pe.refno, checked_u32(pe.dbnum, "hierarchy.dbnum")?);
        }
        let parents = known.keys().copied().collect::<BTreeSet<_>>();
        let mut rows = Vec::new();
        for edge in self.load_hierarchy_edges(&parents).await? {
            if let (Some(dbnum), true) = (known.get(&edge.parent), known.contains_key(&edge.child))
            {
                rows.push(HierarchyRow {
                    dbnum: *dbnum,
                    parent: edge.parent,
                    child: edge.child,
                    ordinal: edge.order,
                });
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
        // CATA 允许 refno 级闭包按需落库；其层级边由同一闭包直接写入 pe_owner，
        // 不能要求整个目录 dbnum 先通过全量 bulk-ready 审计。
        let db_types = self.load_db_types(&requested_dbnums).await?;
        let parents = pe_rows.iter().map(|pe| pe.refno).collect::<BTreeSet<_>>();
        let children = self.load_children_by_parents(&parents).await?;
        let mut found = Vec::with_capacity(pe_rows.len());

        for pe in pe_rows {
            let dbnum = checked_u32(pe.dbnum, "catalog.dbnum")?;
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
                children: children.get(&pe.refno).cloned().unwrap_or_default(),
                outbound: attributes
                    .found
                    .get(&pe.refno)
                    .map(|attributes| attributes.reference_edges(dbnum))
                    .unwrap_or_default(),
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
    async fn ensure_hierarchy_coverage(
        &self,
        dbnums: &BTreeSet<u32>,
    ) -> GenerationReadResult<()> {
        for dbnum in dbnums {
            let maintained_since =
                crate::versioned_db::pe_owner_meta::get_maintained_since(*dbnum)
                    .await
                    .map_err(|error| backend_error("hierarchy.ready", error))?
                    .ok_or_else(|| {
                        backend_error(
                            "hierarchy.ready",
                            format!("dbnum={dbnum} pe_owner 尚未通过全量完整性审计"),
                        )
                    })?;
            if !self.version_suffix.is_empty() {
                let requested_sesno = self
                    .manifest
                    .versions
                    .get(dbnum)
                    .map(|version| version.sesno)
                    .ok_or_else(|| {
                        GenerationReadError::InvalidManifest(format!(
                            "层级读取缺少 dbnum={dbnum} 版本水位"
                        ))
                    })?;
                if requested_sesno < maintained_since {
                    return Err(GenerationReadError::UnsupportedReadAt {
                        backend: "surreal-main/pe_owner",
                        read_at: format!(
                            "dbnum={dbnum}, sesno={requested_sesno}, maintained_since={maintained_since}"
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    async fn load_hierarchy_edges(
        &self,
        parents: &BTreeSet<RefnoEnum>,
    ) -> GenerationReadResult<Vec<HierarchyEdgeRow>> {
        let mut edges = Vec::new();
        for chunk in parents
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .chunks(QUERY_CHUNK_SIZE)
        {
            let keys = chunk
                .iter()
                .map(RefnoEnum::to_pe_key)
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, in AS child, out AS parent, record::id(id)[1] AS ordinal \
                 FROM [{keys}]<-pe_owner ORDER BY out, id{};",
                self.version_suffix
            );
            let mut response = project_primary_db()
                .query(sql)
                .await
                .map_err(|error| backend_error("hierarchy.edges", error))?
                .check()
                .map_err(|error| backend_error("hierarchy.edges", error))?;
            let rows: Vec<HierarchyEdgeDbRow> = response
                .take(0)
                .map_err(|error| backend_error("hierarchy.edges.decode", error))?;
            for row in rows {
                let order =
                    u32::try_from(row.ordinal).map_err(|_| GenerationReadError::BackendQuery {
                        backend: "surreal-main",
                        operation: "hierarchy.order",
                        message: format!("value {} is outside u32", row.ordinal),
                    })?;
                edges.push(HierarchyEdgeRow {
                    parent: row.parent,
                    child: row.child,
                    order,
                });
            }
        }
        Ok(edges)
    }

    async fn load_children_by_parents(
        &self,
        parents: &BTreeSet<RefnoEnum>,
    ) -> GenerationReadResult<BTreeMap<RefnoEnum, Vec<RefnoEnum>>> {
        let mut children = parents
            .iter()
            .copied()
            .map(|parent| (parent, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for edge in self.load_hierarchy_edges(parents).await? {
            children.entry(edge.parent).or_default().push(edge.child);
        }
        Ok(children)
    }

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
                "child_count > 0"
            } else {
                "child_count = 0 OR child_count = NONE"
            };
            clauses.push(predicate.to_string());
        }
        let predicate = clauses.join(" AND ");
        let mut rows = Vec::new();
        let mut offset = 0;
        loop {
            let sql = format!(
                "SELECT * FROM pe WHERE {predicate} ORDER BY dbnum, id LIMIT {QUERY_PAGE_SIZE} START {offset}{};",
                self.version_suffix
            );
            let page = self.query_pe_rows(sql, "element.query_rows").await?;
            let page_len = page.len();
            rows.extend(page);
            if page_len < QUERY_PAGE_SIZE {
                break;
            }
            offset += QUERY_PAGE_SIZE;
        }
        Ok(rows)
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

#[derive(Debug, Deserialize, SurrealValue)]
struct HierarchyEdgeDbRow {
    child: RefnoEnum,
    parent: RefnoEnum,
    ordinal: i64,
}

struct HierarchyEdgeRow {
    child: RefnoEnum,
    parent: RefnoEnum,
    order: u32,
}

fn element_from_pe(
    pe: SPdmsElement,
    children: Vec<RefnoEnum>,
) -> GenerationReadResult<ElementSnapshot> {
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

/// Decide whether a backend message means the requested `VERSION AT` instant
/// fell below the storage GC floor.
///
/// A bare "invalid argument" is the generic Surreal/RocksDB parameter error.
/// Classifying it as expired history reports a retention problem for what is
/// actually a broken query, and sends the caller into a pointless source
/// rescan, so the generic wordings only count alongside a versioning context.
fn is_history_expired(lower: &str) -> bool {
    const GC_FLOOR_MARKERS: [&str; 3] = [
        "full_history_ts_low",
        "below the garbage collection",
        "smaller than full_history_ts",
    ];
    if GC_FLOOR_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return true;
    }
    let generic_marker = lower.contains("invalidargument")
        || lower.contains("invalid argument")
        || lower.contains("retention");
    let versioned_context = lower.contains("version")
        || lower.contains("history")
        || lower.contains("garbage collection");
    generic_marker && versioned_context
}

fn backend_error(operation: &'static str, error: impl std::fmt::Display) -> GenerationReadError {
    let message = error.to_string();
    if is_history_expired(&message.to_ascii_lowercase()) {
        return GenerationReadError::HistoryExpired { operation, message };
    }
    GenerationReadError::BackendQuery {
        backend: "surreal-main",
        operation,
        message,
    }
}
