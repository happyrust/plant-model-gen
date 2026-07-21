use std::collections::BTreeMap;

use aios_core::{
    RefnoEnum, SurrealQueryExt, get_named_attmap, project_primary_db, rs_surreal::PlantTransform,
};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};

use crate::generation_read::{
    AttributeSet, ElementSnapshot, HierarchyRow, InputVersionManifest, TransformSnapshot,
};

use super::replica::{
    ReplicaApplyBatch, ReplicaDbCatalogEntry, ReplicaElement, SurrealReplicaStore,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapState {
    pub dbnum_sesnos: BTreeMap<u32, u32>,
    pub elements: Vec<ReplicaElement>,
    pub hierarchy_rows: Vec<HierarchyRow>,
    pub transforms: Vec<TransformSnapshot>,
    pub db_catalog: Vec<ReplicaDbCatalogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapReport {
    pub authoritative_snapshot_id: u64,
    pub history_start_snapshot: u64,
    pub manifest_hash: String,
    pub element_count: usize,
    pub hierarchy_edge_count: usize,
    pub transform_count: usize,
    pub replica_version_time: String,
}

#[async_trait]
pub trait BootstrapSource: Send + Sync {
    async fn load_current_committed_state(&self) -> anyhow::Result<BootstrapState>;
}

#[derive(Debug, Clone)]
pub struct SurrealCurrentStateBootstrapSource {
    attribute_concurrency: usize,
}

impl Default for SurrealCurrentStateBootstrapSource {
    fn default() -> Self {
        Self {
            attribute_concurrency: 64,
        }
    }
}

impl SurrealCurrentStateBootstrapSource {
    pub fn new(attribute_concurrency: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(
            attribute_concurrency > 0,
            "attribute_concurrency 必须大于 0"
        );
        Ok(Self {
            attribute_concurrency,
        })
    }

    /// 从 Surreal 当前态读取指定 dbnum 的完整事实。调用方负责提供已经确定的
    /// 权威 sesno；该入口供 legacy apply 后、DuckLake commit 前的迁移桥使用。
    ///
    /// 大库（如 AMS 单 dbnum 十几万 PE）必须分页拉取，避免单次 WS 响应过大导致
    /// Connection reset。
    pub async fn load_selected_current_state(
        &self,
        dbnum_sesnos: BTreeMap<u32, u32>,
    ) -> anyhow::Result<BootstrapState> {
        self.load_selected_current_state_limited(dbnum_sesnos, None)
            .await
    }

    pub async fn load_selected_current_state_limited(
        &self,
        dbnum_sesnos: BTreeMap<u32, u32>,
        max_elements: Option<usize>,
    ) -> anyhow::Result<BootstrapState> {
        const PE_PAGE: usize = 1000;
        const TRANSFORM_PAGE: usize = 2000;
        const ATTR_PROGRESS_EVERY: usize = 5000;

        anyhow::ensure!(!dbnum_sesnos.is_empty(), "selected current state 不能为空");
        let dbnums = dbnum_sesnos.keys().copied().collect::<Vec<_>>();
        let dbnum_list = dbnums
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");

        let mut response = project_primary_db()
            .query(format!(
                "SELECT dbnum, db_type, file_name FROM dbnum_info_table \
                 WHERE dbnum IN [{dbnum_list}] ORDER BY dbnum;"
            ))
            .await?
            .check()?;
        let db_rows: Vec<BootstrapDbCatalogRow> = response.take(0)?;

        let mut pe_rows = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let remaining = max_elements.map(|limit| limit.saturating_sub(pe_rows.len()));
            if remaining == Some(0) {
                break;
            }
            let page_limit = remaining.map(|n| n.min(PE_PAGE)).unwrap_or(PE_PAGE);
            let sql = match &cursor {
                Some(last_key) => format!(
                    "SELECT id AS element_id, owner, name, noun, dbnum, children \
                     FROM pe WHERE dbnum IN [{dbnum_list}] AND deleted != true \
                     AND id > {last_key} ORDER BY id LIMIT {page_limit};"
                ),
                None => format!(
                    "SELECT id AS element_id, owner, name, noun, dbnum, children \
                     FROM pe WHERE dbnum IN [{dbnum_list}] AND deleted != true \
                     ORDER BY id LIMIT {page_limit};"
                ),
            };
            let mut page_response = project_primary_db().query(sql).await?.check()?;
            let page: Vec<BootstrapPeRow> = page_response.take(0)?;
            if page.is_empty() {
                break;
            }
            let page_len = page.len();
            cursor = page.last().map(|row| row.element_id.to_pe_key());
            pe_rows.extend(page);
            println!(
                "generation-read bootstrap: loaded pe rows={} (+{page_len})",
                pe_rows.len()
            );
            if page_len < page_limit {
                break;
            }
            if max_elements.is_some_and(|limit| pe_rows.len() >= limit) {
                println!(
                    "generation-read bootstrap: reached max-elements={}",
                    max_elements.unwrap()
                );
                break;
            }
        }

        let attribute_refnos: Vec<RefnoEnum> = pe_rows.iter().map(|row| row.element_id).collect();
        let total_attrs = attribute_refnos.len();
        println!(
            "generation-read bootstrap: loading attributes count={total_attrs} concurrency={}",
            self.attribute_concurrency
        );
        let mut attributes = BTreeMap::new();
        let mut loaded_attrs = 0usize;
        for chunk in attribute_refnos.chunks(ATTR_PROGRESS_EVERY.max(self.attribute_concurrency)) {
            let attr_pairs =
                futures::stream::iter(chunk.iter().copied())
                    .map(|refno| async move {
                        get_named_attmap(refno).await.map(|attrs| (refno, attrs))
                    })
                    .buffer_unordered(self.attribute_concurrency)
                    .try_collect::<Vec<_>>()
                    .await?;
            loaded_attrs += attr_pairs.len();
            attributes.extend(attr_pairs);
            println!(
                "generation-read bootstrap: attributes {}/{}",
                loaded_attrs, total_attrs
            );
        }

        let mut elements = Vec::with_capacity(pe_rows.len());
        let mut hierarchy_rows = Vec::new();
        for row in pe_rows {
            let dbnum = u32::try_from(row.dbnum)?;
            let children = row.children.unwrap_or_default();
            for (ordinal, child) in children.iter().enumerate() {
                hierarchy_rows.push(HierarchyRow {
                    dbnum,
                    parent: row.element_id,
                    child: RefnoEnum::from(child.clone()),
                    ordinal: ordinal as u32,
                });
            }
            let attributes = attributes
                .get(&row.element_id)
                .ok_or_else(|| anyhow::anyhow!("current state 缺少 ATT: {}", row.element_id))?;
            let owner = row
                .owner
                .filter(|owner| owner.is_valid())
                .unwrap_or(row.element_id);
            elements.push(ReplicaElement {
                element: ElementSnapshot {
                    refno: row.element_id,
                    dbnum,
                    owner,
                    noun: row.noun.unwrap_or_default(),
                    name: row.name.unwrap_or_default(),
                    has_children: !children.is_empty(),
                },
                attributes: AttributeSet::from_named_attr_map(row.element_id, attributes),
            });
        }

        let selected_refnos = elements
            .iter()
            .map(|item| item.element.refno)
            .collect::<std::collections::BTreeSet<_>>();
        let dbnum_by_refno = elements
            .iter()
            .map(|item| (item.element.refno, item.element.dbnum))
            .collect::<BTreeMap<_, _>>();

        // 小集合（smoke / max-elements）按主键点查，避免扫全库 pe_transform。
        let mut transform_rows = Vec::new();
        if selected_refnos.len() <= 10_000 {
            let keys: Vec<_> = selected_refnos.iter().copied().collect();
            for chunk in keys.chunks(200) {
                let id_list = chunk
                    .iter()
                    .map(|refno| {
                        let key = refno.to_string();
                        format!("pe_transform:`{key}`")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT id AS transform_id, local_trans.d AS local, world_trans.d AS world \
                     FROM pe_transform WHERE id IN [{id_list}];"
                );
                let mut page_response = project_primary_db().query(sql).await?.check()?;
                let page: Vec<BootstrapTransformRow> = page_response.take(0)?;
                transform_rows.extend(page);
            }
            println!(
                "generation-read bootstrap: loaded transforms={} (keyed)",
                transform_rows.len()
            );
        } else {
            let mut transform_cursor: Option<String> = None;
            loop {
                let sql = match &transform_cursor {
                    Some(last_key) => format!(
                        "SELECT id AS transform_id, local_trans.d AS local, world_trans.d AS world \
                         FROM pe_transform WHERE dbnum IN [{dbnum_list}] AND id > {last_key} \
                         ORDER BY id LIMIT {TRANSFORM_PAGE};"
                    ),
                    None => format!(
                        "SELECT id AS transform_id, local_trans.d AS local, world_trans.d AS world \
                         FROM pe_transform WHERE dbnum IN [{dbnum_list}] \
                         ORDER BY id LIMIT {TRANSFORM_PAGE};"
                    ),
                };
                let mut page_response = project_primary_db().query(sql).await?.check()?;
                let page: Vec<BootstrapTransformRow> = page_response.take(0)?;
                if page.is_empty() {
                    break;
                }
                let page_len = page.len();
                transform_cursor = page.last().map(|row| {
                    let key = row.transform_id.to_string();
                    format!("pe_transform:`{key}`")
                });
                transform_rows.extend(page);
                println!(
                    "generation-read bootstrap: loaded transforms={} (+{page_len})",
                    transform_rows.len()
                );
                if page_len < TRANSFORM_PAGE {
                    break;
                }
            }
        }

        let mut transforms = transform_rows
            .into_iter()
            .filter_map(|row| {
                let refno = row.transform_id;
                let dbnum = dbnum_by_refno.get(&refno).copied()?;
                if !selected_refnos.contains(&refno) {
                    return None;
                }
                row.world.map(|world| TransformSnapshot {
                    refno,
                    dbnum,
                    local: row.local.map(|value| value.0),
                    world: world.0,
                })
            })
            .collect::<Vec<_>>();
        transforms.sort_unstable_by_key(|transform| (transform.dbnum, transform.refno));
        // 只保留两端都在当前元素集合内的层级边（max-elements 截断后常见）。
        hierarchy_rows.retain(|row| {
            selected_refnos.contains(&row.parent) && selected_refnos.contains(&row.child)
        });
        hierarchy_rows.sort_unstable_by_key(|row| (row.dbnum, row.parent, row.ordinal, row.child));

        let mut db_catalog = BTreeMap::new();
        for row in db_rows {
            let dbnum = u32::try_from(row.dbnum)?;
            db_catalog
                .entry(dbnum)
                .or_insert_with(|| ReplicaDbCatalogEntry {
                    dbnum,
                    db_type: row.db_type.unwrap_or_default(),
                    project: row.file_name.unwrap_or_default(),
                });
        }

        Ok(BootstrapState {
            dbnum_sesnos,
            elements,
            hierarchy_rows,
            transforms,
            db_catalog: db_catalog.into_values().collect(),
        })
    }

    /// 从若干 root refno 出发，按 `pe.children` + 属性引用做不动点闭包
    /// （SPRE→SPCO→CATR→SCOM→GMRE…），用于单 BRAN/SITE smoke bootstrap。
    pub async fn load_refno_closure_state(
        &self,
        roots: &[RefnoEnum],
    ) -> anyhow::Result<BootstrapState> {
        anyhow::ensure!(!roots.is_empty(), "root refno 列表不能为空");
        let mut pending: std::collections::VecDeque<RefnoEnum> = roots.iter().copied().collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut pe_rows = Vec::new();
        let mut attributes = BTreeMap::new();
        let mut skipped_missing = 0usize;
        let mut attr_edges_enqueued = 0usize;

        // 设计子树与元件库引用一起 BFS；单层 expand 会漏 GMRE/SCOM，触发 E-DATA-001。
        while let Some(refno) = pending.pop_front() {
            if !seen.insert(refno) {
                continue;
            }
            let sql = format!(
                "SELECT id AS element_id, owner, name, noun, dbnum, children \
                 FROM {} WHERE deleted != true;",
                refno.to_pe_key()
            );
            let mut response = project_primary_db().query(sql).await?.check()?;
            let page: Vec<BootstrapPeRow> = response.take(0)?;
            let Some(row) = page.into_iter().next() else {
                if roots.iter().any(|root| *root == refno) {
                    anyhow::bail!("root/closure 缺少 pe 行: {refno}");
                }
                println!("generation-read bootstrap: skip missing pe={refno}");
                seen.remove(&refno);
                skipped_missing += 1;
                continue;
            };
            for child in row.children.as_deref().unwrap_or(&[]) {
                let child_ref = RefnoEnum::from(child.clone());
                if child_ref.is_valid() && !seen.contains(&child_ref) {
                    pending.push_back(child_ref);
                }
            }
            let dbnum = u32::try_from(row.dbnum).unwrap_or_default();
            pe_rows.push(row);

            let named = get_named_attmap(refno).await?;
            let set = AttributeSet::from_named_attr_map(refno, &named);
            for edge in set.reference_edges(dbnum) {
                if !is_bootstrap_closure_attr(&edge.attribute_name) {
                    continue;
                }
                if edge.target.is_valid() && !edge.target.is_unset() && !seen.contains(&edge.target)
                {
                    pending.push_back(edge.target);
                    attr_edges_enqueued += 1;
                }
            }
            attributes.insert(refno, named);
        }
        println!(
            "generation-read bootstrap: closure pe rows={} from {} roots (attr_edges_enqueued={attr_edges_enqueued}, skipped_missing={skipped_missing})",
            pe_rows.len(),
            roots.len()
        );

        let mut dbnums = std::collections::BTreeSet::new();
        for row in &pe_rows {
            dbnums.insert(u32::try_from(row.dbnum)?);
        }
        let dbnum_list = dbnums
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let all_sesnos = resolve_bootstrap_dbnum_sesnos(None).await?;
        let mut dbnum_sesnos = BTreeMap::new();
        for dbnum in &dbnums {
            let sesno = all_sesnos
                .get(dbnum)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("closure dbnum={dbnum} 无可用 sesno"))?;
            dbnum_sesnos.insert(*dbnum, sesno);
        }

        let mut response = project_primary_db()
            .query(format!(
                "SELECT dbnum, db_type, file_name FROM dbnum_info_table \
                 WHERE dbnum IN [{dbnum_list}] ORDER BY dbnum;"
            ))
            .await?
            .check()?;
        let db_rows: Vec<BootstrapDbCatalogRow> = response.take(0)?;

        let mut elements = Vec::with_capacity(pe_rows.len());
        let mut hierarchy_rows = Vec::new();
        for row in &pe_rows {
            let dbnum = u32::try_from(row.dbnum)?;
            let children = row.children.clone().unwrap_or_default();
            for (ordinal, child) in children.iter().enumerate() {
                let child_ref = RefnoEnum::from(child.clone());
                if !seen.contains(&child_ref) {
                    continue;
                }
                hierarchy_rows.push(HierarchyRow {
                    dbnum,
                    parent: row.element_id,
                    child: child_ref,
                    ordinal: ordinal as u32,
                });
            }
            let attrs = attributes
                .get(&row.element_id)
                .ok_or_else(|| anyhow::anyhow!("closure 缺少 ATT: {}", row.element_id))?;
            let owner = row
                .owner
                .filter(|owner| owner.is_valid())
                .unwrap_or(row.element_id);
            elements.push(ReplicaElement {
                element: ElementSnapshot {
                    refno: row.element_id,
                    dbnum,
                    owner,
                    noun: row.noun.clone().unwrap_or_default(),
                    name: row.name.clone().unwrap_or_default(),
                    has_children: !children.is_empty(),
                },
                attributes: AttributeSet::from_named_attr_map(row.element_id, attrs),
            });
        }

        let selected_refnos = elements
            .iter()
            .map(|item| item.element.refno)
            .collect::<std::collections::BTreeSet<_>>();
        let dbnum_by_refno = elements
            .iter()
            .map(|item| (item.element.refno, item.element.dbnum))
            .collect::<BTreeMap<_, _>>();

        let mut transform_rows = Vec::new();
        let keys: Vec<_> = selected_refnos.iter().copied().collect();
        for chunk in keys.chunks(200) {
            let id_list = chunk
                .iter()
                .map(|refno| format!("pe_transform:`{}`", refno))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id AS transform_id, local_trans.d AS local, world_trans.d AS world \
                 FROM pe_transform WHERE id IN [{id_list}];"
            );
            let mut page_response = project_primary_db().query(sql).await?.check()?;
            let page: Vec<BootstrapTransformRow> = page_response.take(0)?;
            transform_rows.extend(page);
        }

        let mut transforms = transform_rows
            .into_iter()
            .filter_map(|row| {
                let refno = row.transform_id;
                let dbnum = dbnum_by_refno.get(&refno).copied()?;
                row.world.map(|world| TransformSnapshot {
                    refno,
                    dbnum,
                    local: row.local.map(|value| value.0),
                    world: world.0,
                })
            })
            .collect::<Vec<_>>();
        transforms.sort_unstable_by_key(|transform| (transform.dbnum, transform.refno));
        hierarchy_rows.sort_unstable_by_key(|row| (row.dbnum, row.parent, row.ordinal, row.child));

        let mut db_catalog = BTreeMap::new();
        for row in db_rows {
            let dbnum = u32::try_from(row.dbnum)?;
            db_catalog
                .entry(dbnum)
                .or_insert_with(|| ReplicaDbCatalogEntry {
                    dbnum,
                    db_type: row.db_type.unwrap_or_default(),
                    project: row.file_name.unwrap_or_default(),
                });
        }

        Ok(BootstrapState {
            dbnum_sesnos,
            elements,
            hierarchy_rows,
            transforms,
            db_catalog: db_catalog.into_values().collect(),
        })
    }
}

#[async_trait]
impl BootstrapSource for SurrealCurrentStateBootstrapSource {
    async fn load_current_committed_state(&self) -> anyhow::Result<BootstrapState> {
        let before = load_committed_sesnos().await?;
        anyhow::ensure!(!before.is_empty(), "Surreal 当前态没有已提交 data anchor");
        let state = self.load_selected_current_state(before.clone()).await?;

        let after = load_committed_sesnos().await?;
        anyhow::ensure!(
            before == after,
            "bootstrap 读取期间已提交水位发生变化，拒绝建立不一致 snapshot"
        );
        Ok(state)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BootstrapOptions {
    /// 仅写入 DuckLake 权威层，跳过 Surreal replica apply。
    /// 用于遗留站点首次 smoke（配合 generation_read_backend=ducklake）。
    pub authority_only: bool,
}

#[cfg(feature = "generation-read-ducklake")]
pub async fn bootstrap_current_state(
    source: &dyn BootstrapSource,
    authority: &super::authority::DuckLakeAuthority,
    replica: &SurrealReplicaStore,
) -> anyhow::Result<BootstrapReport> {
    bootstrap_current_state_with_options(source, authority, replica, BootstrapOptions::default())
        .await
}

/// 从已解析的 current-state 建立首个 DuckLake snapshot（可选绑定 Surreal replica）。
#[cfg(feature = "generation-read-ducklake")]
pub async fn bootstrap_state(
    state: BootstrapState,
    authority: &super::authority::DuckLakeAuthority,
    replica: &SurrealReplicaStore,
    options: BootstrapOptions,
) -> anyhow::Result<BootstrapReport> {
    bootstrap_state_inner(state, authority, replica, options).await
}

#[cfg(feature = "generation-read-ducklake")]
pub async fn bootstrap_current_state_with_options(
    source: &dyn BootstrapSource,
    authority: &super::authority::DuckLakeAuthority,
    replica: &SurrealReplicaStore,
    options: BootstrapOptions,
) -> anyhow::Result<BootstrapReport> {
    let state = source.load_current_committed_state().await?;
    bootstrap_state_inner(state, authority, replica, options).await
}

#[cfg(feature = "generation-read-ducklake")]
async fn bootstrap_state_inner(
    state: BootstrapState,
    authority: &super::authority::DuckLakeAuthority,
    replica: &SurrealReplicaStore,
    options: BootstrapOptions,
) -> anyhow::Result<BootstrapReport> {
    use super::authority::{
        AuthorityCommit, AuthorityDbVersion, DbCatalogEntry, VersionStoreElement,
    };

    anyhow::ensure!(
        !state.dbnum_sesnos.is_empty(),
        "current-state bootstrap 没有已提交 dbnum"
    );

    let mut db_versions = Vec::with_capacity(state.dbnum_sesnos.len());
    for (dbnum, sesno) in &state.dbnum_sesnos {
        let db_payload = BootstrapDbPayload {
            dbnum: *dbnum,
            sesno: *sesno,
            elements: state
                .elements
                .iter()
                .filter(|item| item.element.dbnum == *dbnum)
                .collect(),
            hierarchy_rows: state
                .hierarchy_rows
                .iter()
                .filter(|row| row.dbnum == *dbnum)
                .collect(),
            transforms: state
                .transforms
                .iter()
                .filter(|row| row.dbnum == *dbnum)
                .collect(),
        };
        let fingerprint = crate::generation_read::hash_serializable(&db_payload);
        db_versions.push(AuthorityDbVersion {
            dbnum: *dbnum,
            from_sesno: *sesno,
            to_sesno: *sesno,
            source: "bootstrap".to_string(),
            commit_fingerprint: fingerprint,
            source_hash: None,
        });
    }

    let global_fingerprint =
        crate::generation_read::hash_serializable(&("current-state-bootstrap-v1", &db_versions));
    let authority_commit = AuthorityCommit {
        global_fingerprint,
        db_versions,
        replace_dbnums: state.dbnum_sesnos.keys().copied().collect(),
        upsert_elements: state
            .elements
            .iter()
            .cloned()
            .map(|item| VersionStoreElement {
                element: item.element,
                attributes: item.attributes,
            })
            .collect(),
        delete_refnos: BTreeMap::new(),
        hierarchy_rows: state.hierarchy_rows.clone(),
        transforms: state.transforms.clone(),
        db_catalog: state
            .db_catalog
            .iter()
            .map(|entry| DbCatalogEntry {
                dbnum: entry.dbnum,
                ref0: None,
                db_type: entry.db_type.clone(),
                project: entry.project.clone(),
            })
            .collect(),
        bootstrap_current_state: true,
    };

    let authority = authority.clone();
    let outcome = tokio::task::spawn_blocking(move || authority.commit(authority_commit))
        .await
        .map_err(|error| anyhow::anyhow!("DuckLake bootstrap task join failed: {error}"))??;
    validate_source_versions(&state.dbnum_sesnos, &outcome.manifest)?;

    if options.authority_only {
        return Ok(BootstrapReport {
            authoritative_snapshot_id: outcome.snapshot_id,
            history_start_snapshot: outcome.manifest.history_start_snapshot,
            manifest_hash: outcome.manifest.manifest_hash,
            element_count: state.elements.len(),
            hierarchy_edge_count: state.hierarchy_rows.len(),
            transform_count: state.transforms.len(),
            replica_version_time: "authority-only".to_string(),
        });
    }

    let replica_batch = ReplicaApplyBatch {
        authoritative_snapshot_id: outcome.snapshot_id,
        previous_snapshot_id: None,
        manifest: outcome.manifest.clone(),
        replace_dbnums: state.dbnum_sesnos.keys().copied().collect(),
        upsert_elements: state.elements.clone(),
        delete_refnos: BTreeMap::new(),
        hierarchy_rows: state.hierarchy_rows.clone(),
        transforms: state.transforms.clone(),
        db_catalog: state.db_catalog.clone(),
        payload_hash: String::new(),
    }
    .seal()?;
    let binding = replica.apply(&replica_batch).await?;
    let replica_manifest = replica.manifest_at(&binding).await?;
    anyhow::ensure!(
        replica_manifest.manifest_hash == outcome.manifest.manifest_hash,
        "bootstrap 双向 manifest 校验失败: ducklake={} surreal={}",
        outcome.manifest.manifest_hash,
        replica_manifest.manifest_hash
    );

    Ok(BootstrapReport {
        authoritative_snapshot_id: outcome.snapshot_id,
        history_start_snapshot: outcome.manifest.history_start_snapshot,
        manifest_hash: outcome.manifest.manifest_hash,
        element_count: state.elements.len(),
        hierarchy_edge_count: state.hierarchy_rows.len(),
        transform_count: state.transforms.len(),
        replica_version_time: binding.replica_version_time,
    })
}

#[cfg(not(feature = "generation-read-ducklake"))]
pub async fn bootstrap_current_state(
    _source: &dyn BootstrapSource,
    _authority: &(),
    _replica: &SurrealReplicaStore,
) -> anyhow::Result<BootstrapReport> {
    anyhow::bail!("current-state bootstrap 需要 generation-read-ducklake feature")
}

#[derive(Serialize)]
struct BootstrapDbPayload<'a> {
    dbnum: u32,
    sesno: u32,
    elements: Vec<&'a ReplicaElement>,
    hierarchy_rows: Vec<&'a HierarchyRow>,
    transforms: Vec<&'a TransformSnapshot>,
}

fn validate_source_versions(
    source: &BTreeMap<u32, u32>,
    manifest: &InputVersionManifest,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        source.len() == manifest.versions.len(),
        "bootstrap manifest dbnum 数量不一致"
    );
    for (dbnum, sesno) in source {
        let version = manifest
            .versions
            .get(dbnum)
            .ok_or_else(|| anyhow::anyhow!("bootstrap manifest 缺少 dbnum={dbnum}"))?;
        anyhow::ensure!(
            version.sesno == *sesno,
            "bootstrap dbnum={dbnum} sesno mismatch source={sesno} manifest={}",
            version.sesno
        );
    }
    Ok(())
}

/// smoke bootstrap 只跟随生成几何必需的引用边；排除 OWNER/REFNO/ISPE 等会扇出整库的边。
fn is_bootstrap_closure_attr(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "SPRE"
            | "CAT"
            | "CATR"
            | "CATU"
            | "CATA"
            | "GMRE"
            | "GSTR"
            | "NGMR"
            | "GMSE"
            | "PTRE"
            | "DTRE"
            | "LSTU"
            | "CCORRE"
    )
}

#[derive(Debug, Deserialize, SurrealValue)]
struct BootstrapPeRow {
    element_id: RefnoEnum,
    #[serde(default)]
    owner: Option<RefnoEnum>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    noun: Option<String>,
    dbnum: i64,
    #[serde(default)]
    children: Option<Vec<RecordId>>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct BootstrapDbCatalogRow {
    dbnum: i64,
    #[serde(default)]
    db_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct BootstrapTransformRow {
    transform_id: RefnoEnum,
    #[serde(default)]
    local: Option<PlantTransform>,
    #[serde(default)]
    world: Option<PlantTransform>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CommittedSesnoRow {
    dbnum: i64,
    sesno: i64,
}

async fn load_committed_sesnos() -> anyhow::Result<BTreeMap<u32, u32>> {
    match load_committed_sesnos_from_anchors().await {
        Ok(map) if !map.is_empty() => Ok(map),
        Ok(_) => load_sesnos_from_dbnum_info().await,
        Err(error) => {
            tracing::warn!("sesno_version_anchor 不可用，回退 dbnum_info_table: {error:#}");
            load_sesnos_from_dbnum_info().await
        }
    }
}

/// 解析 bootstrap 目标 dbnum→sesno。`selected` 为空时取全部已提交水位。
pub async fn resolve_bootstrap_dbnum_sesnos(
    selected: Option<&[u32]>,
) -> anyhow::Result<BTreeMap<u32, u32>> {
    let mut map = load_committed_sesnos().await?;
    if let Some(selected) = selected {
        anyhow::ensure!(!selected.is_empty(), "selected dbnum 列表不能为空");
        let mut filtered = BTreeMap::new();
        for dbnum in selected {
            let sesno = map.get(dbnum).copied().ok_or_else(|| {
                anyhow::anyhow!("dbnum={dbnum} 无可用 sesno（anchor/dbnum_info 均缺失）")
            })?;
            filtered.insert(*dbnum, sesno);
        }
        map = filtered;
    }
    anyhow::ensure!(
        !map.is_empty(),
        "Surreal 当前态没有可用于 bootstrap 的 dbnum"
    );
    Ok(map)
}

async fn load_committed_sesnos_from_anchors() -> anyhow::Result<BTreeMap<u32, u32>> {
    let sql = "SELECT dbnum, math::max(sesno) AS sesno FROM sesno_version_anchor \
               WHERE source IN ['full', 'incremental'] GROUP BY dbnum ORDER BY dbnum;";
    let mut response = project_primary_db().query(sql).await?.check()?;
    let rows: Vec<CommittedSesnoRow> = response.take(0)?;
    rows.into_iter()
        .map(|row| Ok((u32::try_from(row.dbnum)?, u32::try_from(row.sesno)?)))
        .collect()
}

async fn load_sesnos_from_dbnum_info() -> anyhow::Result<BTreeMap<u32, u32>> {
    let sql = "SELECT dbnum, math::max(sesno) AS sesno FROM dbnum_info_table \
               WHERE sesno != NONE GROUP BY dbnum ORDER BY dbnum;";
    let mut response = project_primary_db().query(sql).await?.check()?;
    let rows: Vec<CommittedSesnoRow> = response.take(0)?;
    rows.into_iter()
        .map(|row| Ok((u32::try_from(row.dbnum)?, u32::try_from(row.sesno)?)))
        .collect()
}
