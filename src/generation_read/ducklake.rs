use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use aios_core::{RefnoEnum, Transform};
use async_trait::async_trait;
use duckdb::{Connection, params};

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
use crate::version_store::DuckLakeAuthority;

#[derive(Clone)]
pub struct DuckLakeVersionedReadBackend {
    authority: DuckLakeAuthority,
    pool_size: usize,
}

impl DuckLakeVersionedReadBackend {
    pub fn new(authority: DuckLakeAuthority, pool_size: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(pool_size > 0, "DuckLake session pool_size 必须大于 0");
        Ok(Self {
            authority,
            pool_size,
        })
    }
}

pub struct DuckLakeVersionedReadSession {
    manifest: Arc<InputVersionManifest>,
    connections: Arc<Vec<Mutex<Connection>>>,
    next_connection: AtomicUsize,
    metrics: Mutex<SessionMetricsSnapshot>,
}

#[async_trait]
impl GenerationReadBackend for DuckLakeVersionedReadBackend {
    fn backend_kind(&self) -> GenerationReadBackendKind {
        GenerationReadBackendKind::DuckLake
    }

    async fn open_session(
        &self,
        manifest: Arc<InputVersionManifest>,
    ) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
        manifest.verify_hash()?;
        let authority = self.authority.clone();
        let snapshot_id = manifest.authoritative_snapshot_id;
        let authoritative_manifest = tokio::task::spawn_blocking(move || {
            if !authority.snapshot_exists(snapshot_id)? {
                return Err(GenerationReadError::SnapshotUnavailable { snapshot_id }.into());
            }
            authority.read_manifest(snapshot_id)
        })
        .await
        .map_err(|error| backend_error("open_session.join", error))?
        .map_err(|error| match error.downcast::<GenerationReadError>() {
            Ok(error) => error,
            Err(error) => backend_error("open_session.manifest", error),
        })?;
        if authoritative_manifest.manifest_hash != manifest.manifest_hash {
            return Err(GenerationReadError::ManifestMismatch {
                snapshot_id,
                expected: manifest.manifest_hash.clone(),
                actual: authoritative_manifest.manifest_hash,
            });
        }

        let mut connections = Vec::with_capacity(self.pool_size);
        for _ in 0..self.pool_size {
            let authority = self.authority.clone();
            let connection =
                tokio::task::spawn_blocking(move || authority.open_pinned_connection(snapshot_id))
                    .await
                    .map_err(|error| backend_error("open_session.pool.join", error))?
                    .map_err(|error| backend_error("open_session.pool", error))?;
            connections.push(Mutex::new(connection));
        }

        Ok(Arc::new(DuckLakeVersionedReadSession {
            manifest,
            connections: Arc::new(connections),
            next_connection: AtomicUsize::new(0),
            metrics: Mutex::new(SessionMetricsSnapshot::default()),
        }))
    }
}

impl VersionedReadSession for DuckLakeVersionedReadSession {
    fn manifest(&self) -> &InputVersionManifest {
        &self.manifest
    }

    fn backend_kind(&self) -> GenerationReadBackendKind {
        GenerationReadBackendKind::DuckLake
    }

    fn metrics(&self) -> SessionMetricsSnapshot {
        self.metrics
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl ElementRead for DuckLakeVersionedReadSession {
    async fn load_elements(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<ElementSnapshot>> {
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
        let found = self
            .with_connection("element.load", move |connection| {
                install_requested_refnos(connection, &refno_strings)?;
                query_elements(
                    connection,
                    "JOIN requested_refnos requested USING (refno) ORDER BY element.dbnum, element.refno",
                )
            })
            .await?;
        self.record(
            "element.load",
            requested,
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
        let query = query.clone();
        let requested = query.dbnums.len();
        let elements = self
            .with_connection("element.query", move |connection| {
                let mut clauses = Vec::new();
                if !query.dbnums.is_empty() {
                    clauses.push(format!(
                        "dbnum IN ({})",
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
                        "upper(noun) IN ({})",
                        query
                            .nouns
                            .iter()
                            .map(|noun| sql_string(&noun.to_ascii_uppercase()))
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                if let Some(has_children) = query.has_children {
                    clauses.push(format!("has_children = {has_children}"));
                }
                let suffix = if clauses.is_empty() {
                    "ORDER BY dbnum, refno".to_string()
                } else {
                    format!("WHERE {} ORDER BY dbnum, refno", clauses.join(" AND "))
                };
                query_elements(connection, &suffix)
            })
            .await?;
        self.record(
            "element.query",
            requested,
            elements.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(elements)
    }
}

#[async_trait]
impl AttributeRead for DuckLakeVersionedReadSession {
    async fn load_attribute_sets(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<AttributeSet>> {
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
        let found = self
            .with_connection("attribute.load", move |connection| {
                install_requested_refnos(connection, &refno_strings)?;
                let mut statement = connection.prepare(
                    "SELECT element.refno, element.attr_codec_version, element.attr_payload, element.attr_hash \
                     FROM element JOIN requested_refnos requested USING (refno) ORDER BY element.refno",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, u16>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut out = Vec::with_capacity(rows.len());
                for (refno, codec_version, payload, projected_hash) in rows {
                    let parsed_refno = parse_refno(&refno, "attribute.refno")?;
                    let attributes =
                        decode_attribute_set_payload(&payload).map_err(|error| {
                            GenerationReadError::PayloadCorrupt {
                                refno: parsed_refno,
                                detail: format!("attribute decode failed: {error}"),
                            }
                        })?;
                    if attributes.refno != parsed_refno
                        || attributes.codec_version != codec_version
                        || attributes.canonical_hash != projected_hash
                    {
                        return Err(GenerationReadError::PayloadCorrupt {
                            refno: parsed_refno,
                            detail: "attribute projection mismatch".to_string(),
                        }
                        .into());
                    }
                    attributes.verify()?;
                    out.push((attributes.refno, attributes));
                }
                Ok(out)
            })
            .await?;
        self.record(
            "attribute.load",
            requested,
            found.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(BatchLookup::from_found(refnos, found))
    }
}

#[async_trait]
impl HierarchyRead for DuckLakeVersionedReadSession {
    async fn load_hierarchy_rows(&self, dbnums: &[u32]) -> GenerationReadResult<Vec<HierarchyRow>> {
        let started = Instant::now();
        if dbnums.is_empty() {
            return Ok(Vec::new());
        }
        let dbnums = dbnums.iter().copied().collect::<BTreeSet<_>>();
        let requested = dbnums.len();
        let rows: Vec<HierarchyRow> = self
            .with_connection("hierarchy.load", move |connection| {
                let mut statement = connection.prepare(&format!(
                    "SELECT dbnum, parent_refno, child_refno, ordinal FROM hierarchy_edge \
                     WHERE dbnum IN ({}) ORDER BY dbnum, parent_refno, ordinal",
                    dbnums
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ))?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, u32>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.into_iter()
                    .map(|(dbnum, parent, child, ordinal)| {
                        Ok(HierarchyRow {
                            dbnum,
                            parent: parse_refno(&parent, "hierarchy.parent")?,
                            child: parse_refno(&child, "hierarchy.child")?,
                            ordinal,
                        })
                    })
                    .collect()
            })
            .await?;
        self.record(
            "hierarchy.load",
            requested,
            rows.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(rows)
    }
}

#[async_trait]
impl CatalogGraphRead for DuckLakeVersionedReadSession {
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
        let found = self
            .with_connection("catalog.load", move |connection| {
                install_requested_refnos(connection, &refno_strings)?;
                let elements = query_elements(
                    connection,
                    "JOIN requested_refnos requested USING (refno) ORDER BY element.refno",
                )?;

                let mut references_by_source: BTreeMap<
                    RefnoEnum,
                    Vec<AttributeReference>,
                > = BTreeMap::new();
                let mut statement = connection.prepare(
                    "SELECT edge.dbnum, edge.source_refno, edge.attribute_name, edge.target_refno, edge.ordinal \
                     FROM reference_edge edge JOIN requested_refnos requested \
                     ON edge.source_refno = requested.refno \
                     ORDER BY edge.source_refno, edge.attribute_name, edge.ordinal",
                )?;
                let reference_rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, u32>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for (dbnum, source, attribute_name, target, ordinal) in reference_rows {
                    let edge = AttributeReference {
                        dbnum,
                        source: parse_refno(&source, "catalog.reference.source")?,
                        attribute_name,
                        target: parse_refno(&target, "catalog.reference.target")?,
                        ordinal,
                    };
                    references_by_source
                        .entry(edge.source)
                        .or_default()
                        .push(edge);
                }

                let mut children_by_parent: BTreeMap<RefnoEnum, Vec<(u32, RefnoEnum)>> =
                    BTreeMap::new();
                let mut statement = connection.prepare(
                    "SELECT edge.parent_refno, edge.child_refno, edge.ordinal \
                     FROM hierarchy_edge edge JOIN requested_refnos requested \
                     ON edge.parent_refno = requested.refno ORDER BY edge.parent_refno, edge.ordinal",
                )?;
                let child_rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, u32>(2)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for (parent, child, ordinal) in child_rows {
                    children_by_parent
                        .entry(parse_refno(&parent, "catalog.child.parent")?)
                        .or_default()
                        .push((ordinal, parse_refno(&child, "catalog.child.child")?));
                }

                let mut db_types = BTreeMap::new();
                let mut statement = connection.prepare("SELECT dbnum, db_type FROM db_catalog")?;
                for row in statement
                    .query_map([], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)))?
                {
                    let (dbnum, db_type) = row?;
                    db_types.insert(dbnum, db_type);
                }

                let mut out = Vec::with_capacity(elements.len());
                for element in elements {
                    let db_type = db_types.get(&element.dbnum).cloned().ok_or_else(|| {
                        GenerationReadError::MissingRequiredData {
                            capability: "catalog.db_type",
                            refnos: vec![element.refno],
                        }
                    })?;
                    let node = CatalogNode {
                        refno: element.refno,
                        dbnum: element.dbnum,
                        db_type,
                        noun: element.noun,
                        owner: element.owner,
                        children: children_by_parent
                            .remove(&element.refno)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(_, child)| child)
                            .collect(),
                        outbound: references_by_source
                            .remove(&element.refno)
                            .unwrap_or_default(),
                    };
                    out.push((node.refno, node));
                }
                Ok(out)
            })
            .await?;
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
impl TransformRead for DuckLakeVersionedReadSession {
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
        let found = self
            .with_connection("transform.load", move |connection| {
                install_requested_refnos(connection, &refno_strings)?;
                let mut statement = connection.prepare(
                    "SELECT transform.dbnum, transform.refno, transform.local_transform, \
                            transform.world_transform, transform.transform_hash \
                     FROM transform JOIN requested_refnos requested USING (refno) \
                     ORDER BY transform.refno",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<Vec<u8>>>(2)?,
                            row.get::<_, Vec<u8>>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut out = Vec::with_capacity(rows.len());
                for (dbnum, refno, local, world, expected_hash) in rows {
                    let parsed_refno = parse_refno(&refno, "transform.refno")?;
                    let snapshot = TransformSnapshot {
                        refno: parsed_refno,
                        dbnum,
                        local: local
                            .as_deref()
                            .map(bincode::deserialize::<Transform>)
                            .transpose()
                            .map_err(|error| GenerationReadError::PayloadCorrupt {
                                refno: parsed_refno,
                                detail: format!("local transform decode failed: {error}"),
                            })?,
                        world: bincode::deserialize(&world).map_err(|error| {
                            GenerationReadError::PayloadCorrupt {
                                refno: parsed_refno,
                                detail: format!("world transform decode failed: {error}"),
                            }
                        })?,
                    };
                    let actual_hash = hash_serializable(&snapshot);
                    if actual_hash != expected_hash {
                        return Err(GenerationReadError::PayloadCorrupt {
                            refno: parsed_refno,
                            detail: format!(
                                "transform hash mismatch expected={expected_hash} actual={actual_hash}"
                            ),
                        }
                        .into());
                    }
                    out.push((snapshot.refno, snapshot));
                }
                Ok(out)
            })
            .await?;
        self.record(
            "transform.load",
            requested,
            found.len(),
            started.elapsed().as_micros() as u64,
        );
        Ok(BatchLookup::from_found(refnos, found))
    }
}

impl DuckLakeVersionedReadSession {
    async fn with_connection<T, F>(
        &self,
        operation: &'static str,
        task: F,
    ) -> GenerationReadResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let connections = Arc::clone(&self.connections);
        let index = self.next_connection.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        tokio::task::spawn_blocking(move || {
            let connection = connections[index]
                .lock()
                .map_err(|_| anyhow::anyhow!("DuckLake session connection mutex poisoned"))?;
            task(&connection)
        })
        .await
        .map_err(|error| backend_error(operation, error))?
        .map_err(|error| match error.downcast::<GenerationReadError>() {
            Ok(error) => error,
            Err(error) => backend_error(operation, error),
        })
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

fn install_requested_refnos(connection: &Connection, refnos: &[String]) -> anyhow::Result<()> {
    connection.execute_batch(
        "DROP TABLE IF EXISTS requested_refnos; \
         CREATE TEMP TABLE requested_refnos(refno VARCHAR);",
    )?;
    let mut statement = connection.prepare("INSERT INTO requested_refnos VALUES (?)")?;
    for refno in refnos {
        statement.execute(params![refno])?;
    }
    Ok(())
}

fn query_elements(connection: &Connection, suffix: &str) -> anyhow::Result<Vec<ElementSnapshot>> {
    let sql = format!(
        "SELECT element.dbnum, element.refno, element.owner_refno, element.noun, \
                element.name, element.has_children FROM element {suffix}"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(
            |(dbnum, refno, owner, noun, name, has_children)| -> anyhow::Result<_> {
                Ok(ElementSnapshot {
                    dbnum,
                    refno: parse_refno(&refno, "element.refno")?,
                    owner: parse_refno(&owner, "element.owner")?,
                    noun,
                    name,
                    has_children,
                })
            },
        )
        .collect()
}

fn parse_refno(value: &str, operation: &'static str) -> anyhow::Result<RefnoEnum> {
    let refno = RefnoEnum::from(value);
    // owner 等字段允许 unset（常见字面量 "0_0"）；其它非法 refno 仍失败。
    if refno.is_valid() || refno.is_unset() {
        Ok(refno)
    } else {
        Err(backend_error(operation, format!("invalid refno {value:?}")).into())
    }
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn backend_error(operation: &'static str, error: impl std::fmt::Display) -> GenerationReadError {
    GenerationReadError::BackendQuery {
        backend: "ducklake",
        operation,
        message: error.to_string(),
    }
}
