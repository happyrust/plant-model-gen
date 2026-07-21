use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use aios_core::{RefnoEnum, SurrealQueryExt, Transform, project_primary_db};
use async_trait::async_trait;
use surrealdb::types::SurrealValue;

use super::error::{GenerationReadError, GenerationReadResult};
use super::traits::{
    AttributeRead, CatalogGraphRead, ElementRead, GenerationReadBackend, HierarchyRead,
    TransformRead, VersionedReadSession,
};
use super::types::{
    AttributeReference, AttributeSet, BatchLookup, CatalogNode, ElementQuery, ElementSnapshot,
    GenerationReadBackendKind, HierarchyRow, InputVersionManifest, SessionMetricsSnapshot,
    TransformSnapshot, decode_attribute_set_payload, hash_serializable,
};
use crate::version_store::{ReplicaSnapshotBinding, SurrealReplicaStore};

#[derive(Debug, Clone, Default)]
pub struct SurrealVersionedReadBackend {
    replica: SurrealReplicaStore,
}

impl SurrealVersionedReadBackend {
    pub fn new(replica: SurrealReplicaStore) -> Self {
        Self { replica }
    }
}

pub struct SurrealVersionedReadSession {
    manifest: Arc<InputVersionManifest>,
    binding: ReplicaSnapshotBinding,
    /// Surreal 3.x：`VERSION` 必须在 WHERE/ORDER BY 之后；读最新 watermark 时为空
    ///（非 versioned RocksDB 仅支持当前态，且当前态即最新已 apply 的副本）。
    version_suffix: String,
    metrics: Mutex<SessionMetricsSnapshot>,
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
        let binding = self.replica.validate_manifest(&manifest).await?;
        let replica_manifest = self
            .replica
            .manifest_at(&binding)
            .await
            .map_err(|error| backend_error("open_session.manifest", error))?;
        if replica_manifest.manifest_hash != manifest.manifest_hash {
            return Err(GenerationReadError::ManifestMismatch {
                snapshot_id: manifest.authoritative_snapshot_id,
                expected: manifest.manifest_hash.clone(),
                actual: replica_manifest.manifest_hash,
            });
        }
        let watermark = self
            .replica
            .current_watermark()
            .await
            .map_err(|error| backend_error("open_session.watermark", error))?;
        let version_suffix = replica_version_suffix(&binding, watermark)?;
        Ok(Arc::new(SurrealVersionedReadSession {
            manifest,
            binding,
            version_suffix,
            metrics: Mutex::new(SessionMetricsSnapshot::default()),
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
        let requested_refnos = refnos.iter().copied().collect::<BTreeSet<_>>();
        let rows = self
            .load_element_rows(
                "WHERE refno IN $refnos",
                Some(
                    requested_refnos
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                ),
            )
            .await?;
        let found = rows
            .into_iter()
            .map(element_from_row)
            .collect::<GenerationReadResult<Vec<_>>>()?;
        self.record(
            "element.load",
            requested_refnos.len(),
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
        let mut clauses = Vec::new();
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
            clauses.push(format!("has_children = {has_children}"));
        }
        let filter = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let rows = self.load_element_rows(&filter, None).await?;
        let elements = rows
            .into_iter()
            .map(element_from_row)
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
        let requested_refnos = refnos.iter().copied().collect::<BTreeSet<_>>();
        let rows = self
            .load_element_rows(
                "WHERE refno IN $refnos",
                Some(
                    requested_refnos
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                ),
            )
            .await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            let attributes = decode_attribute_set(&row)?;
            found.push((attributes.refno, attributes));
        }
        self.record(
            "attribute.load",
            requested_refnos.len(),
            found.len(),
            started.elapsed().as_micros() as u64,
        );
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
        let dbnums = dbnums.iter().copied().collect::<BTreeSet<_>>();
        let sql = format!(
            "SELECT dbnum, parent_refno, child_refno, ordinal \
             FROM generation_replica_hierarchy \
             WHERE dbnum IN [{}] ORDER BY dbnum, parent_refno, ordinal{};",
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
            .map_err(|error| backend_error("hierarchy.load", error))?
            .check()
            .map_err(|error| backend_error("hierarchy.load", error))?;
        let rows: Vec<HierarchyReplicaRow> = response
            .take(0)
            .map_err(|error| backend_error("hierarchy.decode", error))?;
        let rows = rows
            .into_iter()
            .map(|row| {
                Ok(HierarchyRow {
                    dbnum: checked_u32(row.dbnum, "hierarchy.dbnum")?,
                    parent: parse_refno(&row.parent_refno, "hierarchy.parent")?,
                    child: parse_refno(&row.child_refno, "hierarchy.child")?,
                    ordinal: checked_u32(row.ordinal, "hierarchy.ordinal")?,
                })
            })
            .collect::<GenerationReadResult<Vec<_>>>()?;
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
        let refno_strings = refnos
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let requested = refno_strings.len();
        let dbnums = self.manifest.dbnums();
        let version = &self.version_suffix;
        let sql = format!(
            "SELECT dbnum, refno, owner_refno, noun, name, has_children, \
                    attr_codec_version, attr_payload_hex, attr_hash \
             FROM generation_replica_element WHERE refno IN $refnos{version};\n\
             SELECT dbnum, source_refno, attribute_name, target_refno, ordinal \
             FROM generation_replica_reference \
             WHERE source_refno IN $refnos ORDER BY source_refno, attribute_name, ordinal{version};\n\
             SELECT dbnum, parent_refno, child_refno, ordinal \
             FROM generation_replica_hierarchy \
             WHERE parent_refno IN $refnos ORDER BY parent_refno, ordinal{version};\n\
             SELECT dbnum, db_type, project FROM generation_replica_db_catalog \
             WHERE dbnum IN [{}]{version};",
            dbnums
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut response = project_primary_db()
            .query(sql)
            .bind(("refnos", refno_strings))
            .await
            .map_err(|error| backend_error("catalog.load", error))?
            .check()
            .map_err(|error| backend_error("catalog.load", error))?;
        let elements: Vec<ElementReplicaRow> = response
            .take(0)
            .map_err(|error| backend_error("catalog.elements", error))?;
        let references: Vec<ReferenceReplicaRow> = response
            .take(1)
            .map_err(|error| backend_error("catalog.references", error))?;
        let children: Vec<HierarchyReplicaRow> = response
            .take(2)
            .map_err(|error| backend_error("catalog.children", error))?;
        let catalogs: Vec<DbCatalogReplicaRow> = response
            .take(3)
            .map_err(|error| backend_error("catalog.db_catalog", error))?;

        let mut db_types = BTreeMap::new();
        for row in catalogs {
            db_types.insert(checked_u32(row.dbnum, "catalog.dbnum")?, row.db_type);
        }

        let mut references_by_source: BTreeMap<RefnoEnum, Vec<AttributeReference>> =
            BTreeMap::new();
        for row in references {
            let source = parse_refno(&row.source_refno, "catalog.reference.source")?;
            references_by_source
                .entry(source)
                .or_default()
                .push(AttributeReference {
                    dbnum: checked_u32(row.dbnum, "catalog.reference.dbnum")?,
                    source,
                    attribute_name: row.attribute_name,
                    target: parse_refno(&row.target_refno, "catalog.reference.target")?,
                    ordinal: checked_u32(row.ordinal, "catalog.reference.ordinal")?,
                });
        }

        let mut children_by_parent: BTreeMap<RefnoEnum, Vec<(u32, RefnoEnum)>> = BTreeMap::new();
        for row in children {
            children_by_parent
                .entry(parse_refno(&row.parent_refno, "catalog.child.parent")?)
                .or_default()
                .push((
                    checked_u32(row.ordinal, "catalog.child.ordinal")?,
                    parse_refno(&row.child_refno, "catalog.child.child")?,
                ));
        }

        let mut found = Vec::with_capacity(elements.len());
        for row in elements {
            let element = element_from_row(row)?;
            let db_type = db_types.get(&element.dbnum).cloned().ok_or_else(|| {
                GenerationReadError::MissingRequiredData {
                    capability: "catalog.db_type",
                    refnos: vec![element.refno],
                }
            })?;
            let mut child_rows = children_by_parent
                .remove(&element.refno)
                .unwrap_or_default();
            child_rows.sort_unstable();
            let node = CatalogNode {
                refno: element.refno,
                dbnum: element.dbnum,
                db_type,
                noun: element.noun,
                owner: element.owner,
                children: child_rows.into_iter().map(|(_, child)| child).collect(),
                outbound: references_by_source
                    .remove(&element.refno)
                    .unwrap_or_default(),
            };
            found.push((node.refno, node));
        }
        self.record(
            "catalog.load",
            requested,
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
        let refno_strings = refnos
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let requested = refno_strings.len();
        let sql = format!(
            "SELECT dbnum, refno, local_transform_hex, world_transform_hex, transform_hash \
             FROM generation_replica_transform WHERE refno IN $refnos{};",
            self.version_suffix
        );
        let mut response = project_primary_db()
            .query(sql)
            .bind(("refnos", refno_strings))
            .await
            .map_err(|error| backend_error("transform.load", error))?
            .check()
            .map_err(|error| backend_error("transform.load", error))?;
        let rows: Vec<TransformReplicaRow> = response
            .take(0)
            .map_err(|error| backend_error("transform.decode_rows", error))?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            let refno = parse_refno(&row.refno, "transform.refno")?;
            let local = row
                .local_transform_hex
                .as_deref()
                .map(|value| decode_transform(value, refno, "local"))
                .transpose()?;
            let world = decode_transform(&row.world_transform_hex, refno, "world")?;
            let snapshot = TransformSnapshot {
                refno,
                dbnum: checked_u32(row.dbnum, "transform.dbnum")?,
                local,
                world,
            };
            let actual = hash_serializable(&snapshot);
            if actual != row.transform_hash {
                return Err(GenerationReadError::PayloadCorrupt {
                    refno: snapshot.refno,
                    detail: format!(
                        "transform hash mismatch expected={} actual={actual}",
                        row.transform_hash
                    ),
                });
            }
            found.push((snapshot.refno, snapshot));
        }
        self.record(
            "transform.load",
            requested,
            found.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(BatchLookup::from_found(refnos, found))
    }
}

impl SurrealVersionedReadSession {
    pub fn replica_version_time(&self) -> &str {
        &self.binding.replica_version_time
    }

    async fn load_element_rows(
        &self,
        filter: &str,
        refnos: Option<Vec<String>>,
    ) -> GenerationReadResult<Vec<ElementReplicaRow>> {
        let sql = format!(
            "SELECT dbnum, refno, owner_refno, noun, name, has_children, \
                    attr_codec_version, attr_payload_hex, attr_hash \
             FROM generation_replica_element {filter} ORDER BY dbnum, refno{};",
            self.version_suffix
        );
        let query = project_primary_db().query(sql);
        let mut response = match refnos {
            Some(refnos) => query.bind(("refnos", refnos)).await,
            None => query.await,
        }
        .map_err(|error| backend_error("element.load_rows", error))?
        .check()
        .map_err(|error| backend_error("element.load_rows", error))?;
        response
            .take(0)
            .map_err(|error| backend_error("element.decode_rows", error))
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

#[derive(Debug, SurrealValue)]
struct ElementReplicaRow {
    dbnum: i64,
    refno: String,
    owner_refno: String,
    noun: String,
    name: String,
    has_children: bool,
    attr_codec_version: i64,
    attr_payload_hex: String,
    attr_hash: String,
}

#[derive(Debug, SurrealValue)]
struct HierarchyReplicaRow {
    dbnum: i64,
    parent_refno: String,
    child_refno: String,
    ordinal: i64,
}

#[derive(Debug, SurrealValue)]
struct ReferenceReplicaRow {
    dbnum: i64,
    source_refno: String,
    attribute_name: String,
    target_refno: String,
    ordinal: i64,
}

#[derive(Debug, SurrealValue)]
struct TransformReplicaRow {
    dbnum: i64,
    refno: String,
    local_transform_hex: Option<String>,
    world_transform_hex: String,
    transform_hash: String,
}

#[derive(Debug, SurrealValue)]
struct DbCatalogReplicaRow {
    dbnum: i64,
    db_type: String,
    #[allow(dead_code)]
    project: String,
}

fn element_from_row(row: ElementReplicaRow) -> GenerationReadResult<ElementSnapshot> {
    Ok(ElementSnapshot {
        refno: parse_refno(&row.refno, "element.refno")?,
        dbnum: checked_u32(row.dbnum, "element.dbnum")?,
        owner: parse_refno(&row.owner_refno, "element.owner")?,
        noun: row.noun,
        name: row.name,
        has_children: row.has_children,
    })
}

fn decode_attribute_set(row: &ElementReplicaRow) -> GenerationReadResult<AttributeSet> {
    let bytes = hex::decode(&row.attr_payload_hex).map_err(|error| {
        GenerationReadError::PayloadCorrupt {
            refno: RefnoEnum::from(row.refno.as_str()),
            detail: format!("invalid payload hex: {error}"),
        }
    })?;
    let attributes = decode_attribute_set_payload(&bytes).map_err(|error| {
        GenerationReadError::PayloadCorrupt {
            refno: RefnoEnum::from(row.refno.as_str()),
            detail: format!("invalid payload binary: {error}"),
        }
    })?;
    let row_refno = parse_refno(&row.refno, "attribute.refno")?;
    if attributes.refno != row_refno
        || i64::from(attributes.codec_version) != row.attr_codec_version
        || attributes.canonical_hash != row.attr_hash
    {
        return Err(GenerationReadError::PayloadCorrupt {
            refno: attributes.refno,
            detail: "projected codec/hash does not match payload".to_string(),
        });
    }
    attributes.verify()?;
    Ok(attributes)
}

fn replica_version_suffix(
    binding: &ReplicaSnapshotBinding,
    watermark: u64,
) -> GenerationReadResult<String> {
    if watermark == binding.authoritative_snapshot_id {
        return Ok(String::new());
    }
    if binding.replica_version_time.contains('\'') {
        return Err(GenerationReadError::BackendQuery {
            backend: "surreal",
            operation: "version_suffix",
            message: "replica_version_time 含非法字符".to_string(),
        });
    }
    // Surreal 3.x：VERSION 必须出现在 WHERE/ORDER BY 之后。
    Ok(format!(" VERSION d'{}'", binding.replica_version_time))
}

fn decode_transform(
    value: &str,
    refno: RefnoEnum,
    kind: &'static str,
) -> GenerationReadResult<Transform> {
    let bytes = hex::decode(value).map_err(|error| GenerationReadError::PayloadCorrupt {
        refno,
        detail: format!("{kind} transform hex invalid: {error}"),
    })?;
    bincode::deserialize(&bytes).map_err(|error| GenerationReadError::PayloadCorrupt {
        refno,
        detail: format!("{kind} transform binary invalid: {error}"),
    })
}

fn parse_refno(value: &str, operation: &'static str) -> GenerationReadResult<RefnoEnum> {
    let refno = RefnoEnum::from(value);
    if refno.is_valid() || refno.is_unset() {
        Ok(refno)
    } else {
        Err(GenerationReadError::BackendQuery {
            backend: "surreal",
            operation,
            message: format!("invalid refno {value:?}"),
        })
    }
}

fn checked_u32(value: i64, operation: &'static str) -> GenerationReadResult<u32> {
    u32::try_from(value).map_err(|_| GenerationReadError::BackendQuery {
        backend: "surreal",
        operation,
        message: format!("value {value} outside u32 range"),
    })
}

fn surreal_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization cannot fail")
}

fn backend_error(operation: &'static str, error: impl std::fmt::Display) -> GenerationReadError {
    GenerationReadError::BackendQuery {
        backend: "surreal",
        operation,
        message: error.to_string(),
    }
}
