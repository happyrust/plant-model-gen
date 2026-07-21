use std::collections::{BTreeMap, BTreeSet};

use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::generation_read::{
    AttributeSet, DataVersion, ElementSnapshot, GenerationReadError, GenerationReadResult,
    HierarchyRow, InputVersionManifest, TransformSnapshot, encode_attribute_set_payload,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaElement {
    pub element: ElementSnapshot,
    pub attributes: AttributeSet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaDbCatalogEntry {
    pub dbnum: u32,
    pub db_type: String,
    pub project: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaApplyBatch {
    pub authoritative_snapshot_id: u64,
    pub previous_snapshot_id: Option<u64>,
    pub manifest: InputVersionManifest,
    #[serde(default)]
    pub replace_dbnums: BTreeSet<u32>,
    pub upsert_elements: Vec<ReplicaElement>,
    pub delete_refnos: BTreeMap<u32, Vec<RefnoEnum>>,
    pub hierarchy_rows: Vec<HierarchyRow>,
    pub transforms: Vec<TransformSnapshot>,
    pub db_catalog: Vec<ReplicaDbCatalogEntry>,
    pub payload_hash: String,
}

impl ReplicaApplyBatch {
    pub fn seal(mut self) -> anyhow::Result<Self> {
        self.validate_structure()?;
        self.payload_hash = canonical_payload_hash(&self)?;
        Ok(self)
    }

    pub fn verify_seal(&self) -> anyhow::Result<()> {
        self.validate_structure()?;
        let expected = canonical_payload_hash(self)?;
        anyhow::ensure!(
            expected == self.payload_hash,
            "replica payload hash mismatch expected={} actual={expected}",
            self.payload_hash
        );
        Ok(())
    }

    fn validate_structure(&self) -> anyhow::Result<()> {
        self.manifest.verify_hash()?;
        anyhow::ensure!(
            self.authoritative_snapshot_id == self.manifest.authoritative_snapshot_id,
            "replica batch snapshot 与 manifest 不一致"
        );
        for dbnum in &self.replace_dbnums {
            anyhow::ensure!(
                self.manifest.versions.contains_key(dbnum),
                "replica 完整替换 dbnum={dbnum} 不在 manifest 中"
            );
            anyhow::ensure!(
                !self.delete_refnos.contains_key(dbnum),
                "replica dbnum={dbnum} 不能同时完整替换和显式删除"
            );
        }
        let mut upsert_keys = BTreeSet::new();
        for item in &self.upsert_elements {
            anyhow::ensure!(
                item.element.refno.is_valid()
                    && (item.element.owner.is_valid() || item.element.owner.is_unset()),
                "replica element refno/owner 非法: refno={} owner={}",
                item.element.refno,
                item.element.owner
            );
            anyhow::ensure!(
                item.element.refno == item.attributes.refno,
                "replica element/attributes refno 不一致"
            );
            anyhow::ensure!(
                self.manifest.versions.contains_key(&item.element.dbnum),
                "replica element dbnum={} 不在 manifest 中",
                item.element.dbnum
            );
            anyhow::ensure!(
                upsert_keys.insert((item.element.dbnum, item.element.refno)),
                "replica element 重复 upsert: dbnum={} refno={}",
                item.element.dbnum,
                item.element.refno
            );
            item.attributes.verify()?;
        }
        for (dbnum, refnos) in &self.delete_refnos {
            anyhow::ensure!(
                self.manifest.versions.contains_key(dbnum),
                "replica delete dbnum={dbnum} 不在 manifest 中"
            );
            let unique = refnos.iter().copied().collect::<BTreeSet<_>>();
            anyhow::ensure!(
                unique.len() == refnos.len(),
                "replica delete dbnum={dbnum} 含重复 refno"
            );
            for refno in refnos {
                anyhow::ensure!(
                    refno.is_valid(),
                    "replica delete dbnum={dbnum} 含非法 refno={refno}"
                );
                anyhow::ensure!(
                    !upsert_keys.contains(&(*dbnum, *refno)),
                    "replica 同一 element 不能同时 delete/upsert: dbnum={dbnum} refno={refno}"
                );
            }
        }
        let mut hierarchy_keys = BTreeSet::new();
        let mut hierarchy_ordinals = BTreeSet::new();
        let mut hierarchy_parents = BTreeMap::new();
        for row in &self.hierarchy_rows {
            anyhow::ensure!(
                self.manifest.versions.contains_key(&row.dbnum),
                "replica hierarchy dbnum={} 不在 manifest 中",
                row.dbnum
            );
            anyhow::ensure!(
                row.parent.is_valid() && row.child.is_valid() && row.parent != row.child,
                "replica hierarchy 非法: {} -> {}",
                row.parent,
                row.child
            );
            anyhow::ensure!(
                hierarchy_keys.insert((row.dbnum, row.parent, row.child)),
                "replica hierarchy edge 重复: {} -> {}",
                row.parent,
                row.child
            );
            anyhow::ensure!(
                hierarchy_ordinals.insert((row.dbnum, row.parent, row.ordinal)),
                "replica hierarchy ordinal 重复: parent={} ordinal={}",
                row.parent,
                row.ordinal
            );
            if let Some(existing) = hierarchy_parents.insert((row.dbnum, row.child), row.parent) {
                anyhow::ensure!(
                    existing == row.parent,
                    "replica hierarchy child={} 同时属于 parent={} 和 {}",
                    row.child,
                    existing,
                    row.parent
                );
            }
        }
        let mut transform_keys = BTreeSet::new();
        for transform in &self.transforms {
            anyhow::ensure!(
                self.manifest.versions.contains_key(&transform.dbnum),
                "replica transform dbnum={} 不在 manifest 中",
                transform.dbnum
            );
            anyhow::ensure!(
                transform.refno.is_valid()
                    && transform_keys.insert((transform.dbnum, transform.refno)),
                "replica transform 非法或重复: dbnum={} refno={}",
                transform.dbnum,
                transform.refno
            );
        }
        let mut catalog_dbnums = BTreeSet::new();
        for entry in &self.db_catalog {
            anyhow::ensure!(
                self.manifest.versions.contains_key(&entry.dbnum),
                "replica db_catalog dbnum={} 不在 manifest 中",
                entry.dbnum
            );
            anyhow::ensure!(
                !entry.db_type.trim().is_empty() && catalog_dbnums.insert(entry.dbnum),
                "replica db_catalog 缺少类型或重复: dbnum={}",
                entry.dbnum
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct ReplicaSnapshotBinding {
    pub authoritative_snapshot_id: u64,
    pub history_start_snapshot: u64,
    #[serde(default)]
    pub previous_snapshot_id: Option<u64>,
    pub replica_version_time: String,
    pub manifest_hash: String,
    pub payload_hash: String,
    pub status: String,
    pub element_count: u64,
    pub hierarchy_edge_count: u64,
    pub transform_count: u64,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct ReplicaWatermarkRow {
    snapshot_id: u64,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct ReplicaManifestRow {
    dbnum: u32,
    sesno: u32,
    commit_fingerprint: String,
}

#[derive(Debug, Clone, Default)]
pub struct SurrealReplicaStore;

impl SurrealReplicaStore {
    pub async fn ensure_schema(&self) -> anyhow::Result<()> {
        let sql = r#"
DEFINE TABLE IF NOT EXISTS generation_replica_element SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE generation_replica_element TYPE int;
DEFINE FIELD IF NOT EXISTS refno ON TABLE generation_replica_element TYPE string;
DEFINE FIELD IF NOT EXISTS owner_refno ON TABLE generation_replica_element TYPE string;
DEFINE FIELD IF NOT EXISTS noun ON TABLE generation_replica_element TYPE string;
DEFINE FIELD IF NOT EXISTS name ON TABLE generation_replica_element TYPE string;
DEFINE FIELD IF NOT EXISTS has_children ON TABLE generation_replica_element TYPE bool;
DEFINE FIELD IF NOT EXISTS attr_codec_version ON TABLE generation_replica_element TYPE int;
DEFINE FIELD IF NOT EXISTS attr_payload_hex ON TABLE generation_replica_element TYPE string;
DEFINE FIELD IF NOT EXISTS attr_hash ON TABLE generation_replica_element TYPE string;
DEFINE INDEX IF NOT EXISTS idx_generation_replica_element_dbnum_noun
    ON TABLE generation_replica_element FIELDS dbnum, noun;

DEFINE TABLE IF NOT EXISTS generation_replica_hierarchy SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE generation_replica_hierarchy TYPE int;
DEFINE FIELD IF NOT EXISTS parent_refno ON TABLE generation_replica_hierarchy TYPE string;
DEFINE FIELD IF NOT EXISTS child_refno ON TABLE generation_replica_hierarchy TYPE string;
DEFINE FIELD IF NOT EXISTS ordinal ON TABLE generation_replica_hierarchy TYPE int;
DEFINE INDEX IF NOT EXISTS idx_generation_replica_hierarchy_parent
    ON TABLE generation_replica_hierarchy FIELDS dbnum, parent_refno, ordinal;

DEFINE TABLE IF NOT EXISTS generation_replica_reference SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE generation_replica_reference TYPE int;
DEFINE FIELD IF NOT EXISTS source_refno ON TABLE generation_replica_reference TYPE string;
DEFINE FIELD IF NOT EXISTS attribute_name ON TABLE generation_replica_reference TYPE string;
DEFINE FIELD IF NOT EXISTS target_refno ON TABLE generation_replica_reference TYPE string;
DEFINE FIELD IF NOT EXISTS ordinal ON TABLE generation_replica_reference TYPE int;
DEFINE INDEX IF NOT EXISTS idx_generation_replica_reference_source
    ON TABLE generation_replica_reference FIELDS dbnum, source_refno;

DEFINE TABLE IF NOT EXISTS generation_replica_transform SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE generation_replica_transform TYPE int;
DEFINE FIELD IF NOT EXISTS refno ON TABLE generation_replica_transform TYPE string;
DEFINE FIELD IF NOT EXISTS local_transform_hex ON TABLE generation_replica_transform TYPE option<string>;
DEFINE FIELD IF NOT EXISTS world_transform_hex ON TABLE generation_replica_transform TYPE string;
DEFINE FIELD IF NOT EXISTS transform_hash ON TABLE generation_replica_transform TYPE string;

DEFINE TABLE IF NOT EXISTS generation_replica_db_catalog SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE generation_replica_db_catalog TYPE int;
DEFINE FIELD IF NOT EXISTS db_type ON TABLE generation_replica_db_catalog TYPE string;
DEFINE FIELD IF NOT EXISTS project ON TABLE generation_replica_db_catalog TYPE string;

DEFINE TABLE IF NOT EXISTS generation_replica_manifest SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE generation_replica_manifest TYPE int;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE generation_replica_manifest TYPE int;
DEFINE FIELD IF NOT EXISTS commit_fingerprint ON TABLE generation_replica_manifest TYPE string;

DEFINE TABLE IF NOT EXISTS replica_snapshot_binding SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS authoritative_snapshot_id ON TABLE replica_snapshot_binding TYPE int;
DEFINE FIELD IF NOT EXISTS history_start_snapshot ON TABLE replica_snapshot_binding TYPE int;
DEFINE FIELD IF NOT EXISTS previous_snapshot_id ON TABLE replica_snapshot_binding TYPE option<int>;
DEFINE FIELD IF NOT EXISTS replica_version_time ON TABLE replica_snapshot_binding TYPE datetime;
DEFINE FIELD IF NOT EXISTS manifest_hash ON TABLE replica_snapshot_binding TYPE string;
DEFINE FIELD IF NOT EXISTS payload_hash ON TABLE replica_snapshot_binding TYPE string;
DEFINE FIELD IF NOT EXISTS status ON TABLE replica_snapshot_binding TYPE string ASSERT $value = 'applied';
DEFINE FIELD IF NOT EXISTS element_count ON TABLE replica_snapshot_binding TYPE int;
DEFINE FIELD IF NOT EXISTS hierarchy_edge_count ON TABLE replica_snapshot_binding TYPE int;
DEFINE FIELD IF NOT EXISTS transform_count ON TABLE replica_snapshot_binding TYPE int;

DEFINE TABLE IF NOT EXISTS replica_apply_watermark SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS snapshot_id ON TABLE replica_apply_watermark TYPE int;
DEFINE FIELD IF NOT EXISTS manifest_hash ON TABLE replica_apply_watermark TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE replica_apply_watermark TYPE datetime;
"#;
        project_primary_db().query(sql).await?.check()?;
        Ok(())
    }

    pub async fn apply(&self, batch: &ReplicaApplyBatch) -> anyhow::Result<ReplicaSnapshotBinding> {
        batch.verify_seal()?;
        self.ensure_schema().await?;
        ensure_snapshot_fits_surreal_int(batch.authoritative_snapshot_id)?;
        if let Some(previous) = batch.previous_snapshot_id {
            ensure_snapshot_fits_surreal_int(previous)?;
        }

        if let Some(existing) = self.binding(batch.authoritative_snapshot_id).await? {
            anyhow::ensure!(
                existing.status == "applied"
                    && existing.manifest_hash == batch.manifest.manifest_hash
                    && existing.payload_hash == batch.payload_hash,
                "snapshot={} 已存在冲突的 replica binding",
                batch.authoritative_snapshot_id
            );
            return Ok(existing);
        }

        let snapshot_id = batch.authoritative_snapshot_id;
        let sql = build_apply_sql(batch)?;

        project_primary_db().query(sql).await?.check()?;
        self.binding(snapshot_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("replica transaction committed without binding"))
    }

    pub async fn current_watermark(&self) -> anyhow::Result<u64> {
        self.ensure_schema().await?;
        let mut response = project_primary_db()
            .query("SELECT snapshot_id FROM replica_apply_watermark:global;")
            .await?
            .check()?;
        let rows: Vec<ReplicaWatermarkRow> = response.take(0)?;
        Ok(rows
            .into_iter()
            .next()
            .map(|row| row.snapshot_id)
            .unwrap_or_default())
    }

    pub async fn binding(
        &self,
        snapshot_id: u64,
    ) -> anyhow::Result<Option<ReplicaSnapshotBinding>> {
        ensure_snapshot_fits_surreal_int(snapshot_id)?;
        let mut response = project_primary_db()
            .query(format!(
                "SELECT authoritative_snapshot_id, history_start_snapshot, previous_snapshot_id, \
                 type::string(replica_version_time) AS replica_version_time, manifest_hash, \
                 payload_hash, status, element_count, hierarchy_edge_count, transform_count \
                 FROM replica_snapshot_binding:{snapshot_id};"
            ))
            .await?
            .check()?;
        let rows: Vec<ReplicaSnapshotBinding> = response.take(0)?;
        Ok(rows.into_iter().next())
    }

    pub async fn validate_manifest(
        &self,
        manifest: &InputVersionManifest,
    ) -> GenerationReadResult<ReplicaSnapshotBinding> {
        manifest.verify_hash()?;
        let watermark =
            self.current_watermark()
                .await
                .map_err(|error| GenerationReadError::BackendQuery {
                    backend: "surreal",
                    operation: "replica_watermark",
                    message: error.to_string(),
                })?;
        if watermark < manifest.authoritative_snapshot_id {
            return Err(GenerationReadError::ReplicaLagging {
                requested_snapshot: manifest.authoritative_snapshot_id,
                replica_watermark: watermark,
            });
        }
        let binding = self
            .binding(manifest.authoritative_snapshot_id)
            .await
            .map_err(|error| GenerationReadError::BackendQuery {
                backend: "surreal",
                operation: "replica_binding",
                message: error.to_string(),
            })?
            .ok_or(GenerationReadError::ReplicaBindingMissing {
                snapshot_id: manifest.authoritative_snapshot_id,
            })?;
        if binding.status != "applied"
            || binding.history_start_snapshot != manifest.history_start_snapshot
        {
            return Err(GenerationReadError::ReplicaBindingMissing {
                snapshot_id: manifest.authoritative_snapshot_id,
            });
        }
        if binding.manifest_hash != manifest.manifest_hash {
            return Err(GenerationReadError::ManifestMismatch {
                snapshot_id: manifest.authoritative_snapshot_id,
                expected: manifest.manifest_hash.clone(),
                actual: binding.manifest_hash,
            });
        }
        Ok(binding)
    }

    pub async fn manifest_at(
        &self,
        binding: &ReplicaSnapshotBinding,
    ) -> anyhow::Result<InputVersionManifest> {
        // Surreal 3.x：VERSION 必须在 ORDER BY 之后。读最新 watermark 时省略 VERSION，
        // 以兼容未开启 versioned 的 RocksDB（当前态即最新已 apply 副本）。
        let watermark = self.current_watermark().await?;
        let version_suffix = if watermark == binding.authoritative_snapshot_id {
            String::new()
        } else {
            format!(
                " VERSION {}",
                surreal_datetime_literal(&binding.replica_version_time)?
            )
        };
        let mut response = project_primary_db()
            .query(format!(
                "SELECT dbnum, sesno, commit_fingerprint \
                 FROM generation_replica_manifest ORDER BY dbnum{version_suffix};"
            ))
            .await?
            .check()?;
        let mut rows: Vec<ReplicaManifestRow> = response.take(0)?;
        rows.sort_by_key(|row| row.dbnum);
        InputVersionManifest::new(
            binding.authoritative_snapshot_id,
            binding.history_start_snapshot,
            rows.into_iter().map(|row| DataVersion {
                dbnum: row.dbnum,
                sesno: row.sesno,
                commit_fingerprint: row.commit_fingerprint,
            }),
        )
        .map_err(anyhow::Error::from)
    }
}

fn build_apply_sql(batch: &ReplicaApplyBatch) -> anyhow::Result<String> {
    let mut sql = String::from("BEGIN TRANSACTION;\n");
    append_continuity_guard(&mut sql, batch);
    append_deletes(&mut sql, batch);
    append_upserts(&mut sql, batch)?;
    append_manifest(&mut sql, &batch.manifest);

    let snapshot_id = batch.authoritative_snapshot_id;
    let previous = batch
        .previous_snapshot_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NONE".to_string());
    sql.push_str(&format!(
        "LET $replica_version_time = time::now();\n\
         UPSERT replica_snapshot_binding:{snapshot_id} SET \
         authoritative_snapshot_id = {snapshot_id}, \
         history_start_snapshot = {}, \
         previous_snapshot_id = {previous}, \
         replica_version_time = $replica_version_time, \
         manifest_hash = {}, payload_hash = {}, status = 'applied', \
         element_count = {}, hierarchy_edge_count = {}, transform_count = {};\n\
         UPSERT replica_apply_watermark:global SET \
         snapshot_id = {snapshot_id}, manifest_hash = {}, updated_at = $replica_version_time;\n\
         COMMIT TRANSACTION;",
        batch.manifest.history_start_snapshot,
        surreal_string(&batch.manifest.manifest_hash),
        surreal_string(&batch.payload_hash),
        batch.upsert_elements.len(),
        batch.hierarchy_rows.len(),
        batch.transforms.len(),
        surreal_string(&batch.manifest.manifest_hash),
    ));
    Ok(sql)
}

fn append_continuity_guard(sql: &mut String, batch: &ReplicaApplyBatch) {
    sql.push_str(
        "LET $current_watermark = (SELECT snapshot_id FROM replica_apply_watermark:global);\n",
    );
    match batch.previous_snapshot_id {
        Some(previous) => sql.push_str(&format!(
            "IF array::len($current_watermark) != 1 OR $current_watermark[0].snapshot_id != {previous} {{ \
             THROW 'REPLICA_SNAPSHOT_SEQUENCE_MISMATCH'; }};\n"
        )),
        None => sql.push_str(
            "IF array::len($current_watermark) != 0 { \
             THROW 'REPLICA_BOOTSTRAP_REQUIRES_EMPTY_WATERMARK'; };\n",
        ),
    }
}

fn append_deletes(sql: &mut String, batch: &ReplicaApplyBatch) {
    for dbnum in &batch.replace_dbnums {
        sql.push_str(&format!(
            "DELETE generation_replica_hierarchy WHERE dbnum = {dbnum};\n\
             DELETE generation_replica_reference WHERE dbnum = {dbnum};\n\
             DELETE generation_replica_transform WHERE dbnum = {dbnum};\n\
             DELETE generation_replica_element WHERE dbnum = {dbnum};\n\
             DELETE generation_replica_db_catalog WHERE dbnum = {dbnum};\n"
        ));
    }
    for (dbnum, refnos) in &batch.delete_refnos {
        for refno in refnos {
            let key = replica_key(*dbnum, *refno);
            sql.push_str(&format!(
                "DELETE generation_replica_hierarchy WHERE dbnum = {dbnum} \
                 AND (parent_refno = {} OR child_refno = {});\n\
                 DELETE generation_replica_reference WHERE dbnum = {dbnum} AND source_refno = {};\n\
                 DELETE generation_replica_transform:{key};\n\
                 DELETE generation_replica_element:{key};\n",
                surreal_string(&refno.to_string()),
                surreal_string(&refno.to_string()),
                surreal_string(&refno.to_string()),
            ));
        }
    }
}

fn append_upserts(sql: &mut String, batch: &ReplicaApplyBatch) -> anyhow::Result<()> {
    for item in &batch.upsert_elements {
        let element = &item.element;
        let key = replica_key(element.dbnum, element.refno);
        let payload = hex::encode(encode_attribute_set_payload(&item.attributes)?);
        sql.push_str(&format!(
            "UPSERT generation_replica_element:{key} SET \
             dbnum = {}, refno = {}, owner_refno = {}, noun = {}, name = {}, \
             has_children = {}, attr_codec_version = {}, attr_payload_hex = {}, attr_hash = {};\n\
             DELETE generation_replica_reference WHERE dbnum = {} AND source_refno = {};\n\
             DELETE generation_replica_hierarchy WHERE dbnum = {} AND parent_refno = {};\n",
            element.dbnum,
            surreal_string(&element.refno.to_string()),
            surreal_string(&element.owner.to_string()),
            surreal_string(&element.noun),
            surreal_string(&element.name),
            element.has_children,
            item.attributes.codec_version,
            surreal_string(&payload),
            surreal_string(&item.attributes.canonical_hash),
            element.dbnum,
            surreal_string(&element.refno.to_string()),
            element.dbnum,
            surreal_string(&element.refno.to_string()),
        ));
        for edge in item.attributes.reference_edges(element.dbnum) {
            let edge_key = format!(
                "[{}, {}, {}, {}]",
                edge.dbnum,
                surreal_string(&edge.source.to_string()),
                surreal_string(&edge.attribute_name),
                edge.ordinal
            );
            sql.push_str(&format!(
                "UPSERT generation_replica_reference:{edge_key} SET \
                 dbnum = {}, source_refno = {}, attribute_name = {}, target_refno = {}, ordinal = {};\n",
                edge.dbnum,
                surreal_string(&edge.source.to_string()),
                surreal_string(&edge.attribute_name),
                surreal_string(&edge.target.to_string()),
                edge.ordinal
            ));
        }
    }

    for row in &batch.hierarchy_rows {
        let key = format!(
            "[{}, {}, {}]",
            row.dbnum,
            surreal_string(&row.parent.to_string()),
            surreal_string(&row.child.to_string())
        );
        sql.push_str(&format!(
            "DELETE generation_replica_hierarchy WHERE dbnum = {} AND child_refno = {};\n\
             UPSERT generation_replica_hierarchy:{key} SET dbnum = {}, parent_refno = {}, \
             child_refno = {}, ordinal = {};\n",
            row.dbnum,
            surreal_string(&row.child.to_string()),
            row.dbnum,
            surreal_string(&row.parent.to_string()),
            surreal_string(&row.child.to_string()),
            row.ordinal
        ));
    }

    for transform in &batch.transforms {
        let key = replica_key(transform.dbnum, transform.refno);
        let local = transform
            .local
            .as_ref()
            .map(bincode::serialize)
            .transpose()?
            .map(hex::encode);
        let world = hex::encode(bincode::serialize(&transform.world)?);
        let hash = crate::generation_read::hash_serializable(transform);
        sql.push_str(&format!(
            "UPSERT generation_replica_transform:{key} SET dbnum = {}, refno = {}, \
             local_transform_hex = {}, world_transform_hex = {}, transform_hash = {};\n",
            transform.dbnum,
            surreal_string(&transform.refno.to_string()),
            local
                .as_deref()
                .map(surreal_string)
                .unwrap_or_else(|| "NONE".to_string()),
            surreal_string(&world),
            surreal_string(&hash)
        ));
    }

    for entry in &batch.db_catalog {
        sql.push_str(&format!(
            "UPSERT generation_replica_db_catalog:{} SET dbnum = {}, db_type = {}, project = {};\n",
            entry.dbnum,
            entry.dbnum,
            surreal_string(&entry.db_type),
            surreal_string(&entry.project)
        ));
    }
    Ok(())
}

fn append_manifest(sql: &mut String, manifest: &InputVersionManifest) {
    sql.push_str("DELETE generation_replica_manifest;\n");
    for version in manifest.versions.values() {
        sql.push_str(&format!(
            "UPSERT generation_replica_manifest:{} SET dbnum = {}, sesno = {}, commit_fingerprint = {};\n",
            version.dbnum,
            version.dbnum,
            version.sesno,
            surreal_string(&version.commit_fingerprint)
        ));
    }
}

fn canonical_payload_hash(batch: &ReplicaApplyBatch) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct Payload<'a> {
        authoritative_snapshot_id: u64,
        previous_snapshot_id: Option<u64>,
        manifest_hash: &'a str,
        replace_dbnums: &'a BTreeSet<u32>,
        upsert_elements: &'a [ReplicaElement],
        delete_refnos: &'a BTreeMap<u32, Vec<RefnoEnum>>,
        hierarchy_rows: &'a [HierarchyRow],
        transforms: &'a [TransformSnapshot],
        db_catalog: &'a [ReplicaDbCatalogEntry],
    }
    let mut upsert_elements = batch.upsert_elements.clone();
    upsert_elements.sort_unstable_by_key(|item| (item.element.dbnum, item.element.refno));
    let mut delete_refnos = batch.delete_refnos.clone();
    for refnos in delete_refnos.values_mut() {
        refnos.sort_unstable();
    }
    let mut hierarchy_rows = batch.hierarchy_rows.clone();
    hierarchy_rows.sort_unstable_by_key(|row| (row.dbnum, row.parent, row.ordinal, row.child));
    let mut transforms = batch.transforms.clone();
    transforms.sort_unstable_by_key(|transform| (transform.dbnum, transform.refno));
    let mut db_catalog = batch.db_catalog.clone();
    db_catalog.sort_unstable_by_key(|entry| entry.dbnum);
    Ok(crate::generation_read::hash_serializable(&Payload {
        authoritative_snapshot_id: batch.authoritative_snapshot_id,
        previous_snapshot_id: batch.previous_snapshot_id,
        manifest_hash: &batch.manifest.manifest_hash,
        replace_dbnums: &batch.replace_dbnums,
        upsert_elements: &upsert_elements,
        delete_refnos: &delete_refnos,
        hierarchy_rows: &hierarchy_rows,
        transforms: &transforms,
        db_catalog: &db_catalog,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> ReplicaApplyBatch {
        let manifest = InputVersionManifest::new(
            20,
            20,
            [DataVersion {
                dbnum: 1,
                sesno: 100,
                commit_fingerprint: "snapshot-20-db-1".to_string(),
            }],
        )
        .expect("manifest");
        ReplicaApplyBatch {
            authoritative_snapshot_id: 20,
            previous_snapshot_id: None,
            manifest,
            replace_dbnums: BTreeSet::new(),
            upsert_elements: Vec::new(),
            delete_refnos: BTreeMap::new(),
            hierarchy_rows: Vec::new(),
            transforms: Vec::new(),
            db_catalog: Vec::new(),
            payload_hash: String::new(),
        }
    }

    #[test]
    fn interrupted_or_tampered_replica_payload_has_no_valid_seal() {
        let mut sealed = batch().seal().expect("seal");
        sealed.previous_snapshot_id = Some(19);
        assert!(sealed.verify_seal().is_err());
    }

    #[test]
    fn replica_batch_rejects_snapshot_manifest_mismatch() {
        let mut mismatched = batch();
        mismatched.authoritative_snapshot_id = 21;
        assert!(mismatched.seal().is_err());
    }

    #[test]
    fn replica_batch_rejects_payload_outside_manifest_coverage() {
        let mut invalid = batch();
        invalid.transforms.push(TransformSnapshot {
            refno: RefnoEnum::from("2/1"),
            dbnum: 2,
            local: None,
            world: aios_core::Transform::IDENTITY,
        });
        assert!(invalid.seal().is_err());
    }

    #[test]
    fn replica_payload_hash_is_independent_of_row_arrival_order() {
        let mut first = batch();
        first.transforms = ["1/1", "1/2"]
            .into_iter()
            .map(|refno| TransformSnapshot {
                refno: RefnoEnum::from(refno),
                dbnum: 1,
                local: None,
                world: aios_core::Transform::IDENTITY,
            })
            .collect();
        let mut second = first.clone();
        second.transforms.reverse();
        assert_eq!(
            first.seal().expect("first seal").payload_hash,
            second.seal().expect("second seal").payload_hash
        );
    }

    #[test]
    fn replica_binding_and_watermark_are_last_inside_one_transaction() {
        let sealed = batch().seal().expect("seal");
        let sql = build_apply_sql(&sealed).expect("sql");
        let binding = sql
            .find("UPSERT replica_snapshot_binding")
            .expect("binding statement");
        let manifest = sql
            .rfind("UPSERT generation_replica_manifest")
            .expect("manifest statement");
        let watermark = sql
            .find("UPSERT replica_apply_watermark")
            .expect("watermark statement");
        assert!(sql.starts_with("BEGIN TRANSACTION;"));
        assert!(manifest < binding);
        assert!(binding < watermark);
        assert!(sql.ends_with("COMMIT TRANSACTION;"));
        assert!(!sql[..binding].contains("replica_snapshot_binding"));
    }
}

fn replica_key(dbnum: u32, refno: RefnoEnum) -> String {
    format!("[{dbnum}, {}]", surreal_string(&refno.to_string()))
}

fn surreal_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization cannot fail")
}

fn surreal_datetime_literal(value: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!value.contains('\''), "replica_version_time 含非法字符");
    Ok(format!("d'{value}'"))
}

fn ensure_snapshot_fits_surreal_int(snapshot_id: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        snapshot_id <= i64::MAX as u64,
        "snapshot_id 超出 SurrealDB int 范围"
    );
    Ok(())
}
