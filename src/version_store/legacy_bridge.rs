//! 将 legacy Surreal current-state apply 提升为 DuckLake 权威 snapshot，并原子复制
//! 到版本化 Surreal 读副本。该桥只属于迁移窗口；生成读取始终以权威 manifest 为准。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::authority::{
    AuthorityCommit, AuthorityDbVersion, DbCatalogEntry, DuckLakeAuthority, VersionStoreElement,
};
use super::bootstrap::SurrealCurrentStateBootstrapSource;
use super::replica::{ReplicaApplyBatch, SurrealReplicaStore};

#[derive(Debug, Clone)]
pub struct LegacyAuthorityPublishRequest {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source: String,
    pub commit_fingerprint: String,
    pub source_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyAuthorityPublishReport {
    pub authoritative_snapshot_id: u64,
    pub manifest_hash: String,
    pub replica_version_time: String,
    pub idempotent: bool,
}

/// 必须在 legacy 数据 mutation 已成功、legacy committed anchor 尚未发布时调用。
/// 任一权威提交或副本复制错误都会阻止上层发布 committed anchor。
pub async fn publish_legacy_applied_state(
    request: LegacyAuthorityPublishRequest,
) -> anyhow::Result<LegacyAuthorityPublishReport> {
    anyhow::ensure!(request.dbnum > 0, "authority publish dbnum 必须非零");
    anyhow::ensure!(
        request.to_sesno > 0 && request.from_sesno <= request.to_sesno,
        "authority publish sesno 区间非法"
    );
    anyhow::ensure!(
        !request.commit_fingerprint.trim().is_empty(),
        "authority publish fingerprint 不能为空"
    );

    let config = crate::options::get_db_option_ext().ducklake_config();
    let authority = tokio::task::spawn_blocking(move || DuckLakeAuthority::open(config))
        .await
        .map_err(|error| anyhow::anyhow!("open DuckLake authority task join failed: {error}"))??;
    let authority_for_state = authority.clone();
    let had_committed_versions =
        tokio::task::spawn_blocking(move || authority_for_state.has_committed_versions())
            .await
            .map_err(|error| anyhow::anyhow!("read authority state task join failed: {error}"))??;
    anyhow::ensure!(
        had_committed_versions,
        "DuckLake authority 尚未完成 current-state bootstrap；请先执行 model-version bootstrap-generation-read"
    );

    let source = SurrealCurrentStateBootstrapSource::new(32)?;
    let state = source
        .load_selected_current_state(BTreeMap::from([(request.dbnum, request.to_sesno)]))
        .await?;
    let global_fingerprint = crate::generation_read::hash_serializable(&(
        "legacy-authority-publish-v2",
        request.dbnum,
        request.from_sesno,
        request.to_sesno,
        &request.source,
        &request.commit_fingerprint,
        &request.source_hash,
        &state,
    ));
    let authority_commit = AuthorityCommit {
        global_fingerprint,
        db_versions: vec![AuthorityDbVersion {
            dbnum: request.dbnum,
            from_sesno: request.from_sesno,
            to_sesno: request.to_sesno,
            source: request.source,
            commit_fingerprint: request.commit_fingerprint,
            source_hash: request.source_hash,
        }],
        replace_dbnums: BTreeSet::from([request.dbnum]),
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
        bootstrap_current_state: false,
    };

    let authority_for_commit = authority.clone();
    let outcome =
        tokio::task::spawn_blocking(move || authority_for_commit.commit(authority_commit))
            .await
            .map_err(|error| {
                anyhow::anyhow!("DuckLake authority commit task join failed: {error}")
            })??;

    let replica = SurrealReplicaStore;
    if let Some(binding) = replica.binding(outcome.snapshot_id).await? {
        anyhow::ensure!(
            binding.manifest_hash == outcome.manifest.manifest_hash,
            "已存在的 replica binding 与 authority manifest 不一致"
        );
        return Ok(LegacyAuthorityPublishReport {
            authoritative_snapshot_id: outcome.snapshot_id,
            manifest_hash: outcome.manifest.manifest_hash,
            replica_version_time: binding.replica_version_time,
            idempotent: true,
        });
    }

    let replica_watermark = replica.current_watermark().await?;
    let authority_for_previous = authority.clone();
    let previous_authority = tokio::task::spawn_blocking(move || {
        authority_for_previous.previous_data_snapshot_id(outcome.snapshot_id)
    })
    .await
    .map_err(|error| anyhow::anyhow!("read previous authority snapshot task failed: {error}"))??;
    match previous_authority {
        Some(previous) => anyhow::ensure!(
            replica_watermark == previous,
            "replica 水位不连续: watermark={replica_watermark} expected={previous}"
        ),
        None => anyhow::ensure!(
            replica_watermark == 0,
            "首个 authority snapshot 要求空 replica 水位，actual={replica_watermark}"
        ),
    }

    let replica_batch = ReplicaApplyBatch {
        authoritative_snapshot_id: outcome.snapshot_id,
        previous_snapshot_id: previous_authority,
        manifest: outcome.manifest.clone(),
        replace_dbnums: BTreeSet::from([request.dbnum]),
        upsert_elements: state.elements,
        delete_refnos: BTreeMap::new(),
        hierarchy_rows: state.hierarchy_rows,
        transforms: state.transforms,
        db_catalog: state.db_catalog,
        payload_hash: String::new(),
    }
    .seal()?;
    let binding = replica.apply(&replica_batch).await?;
    Ok(LegacyAuthorityPublishReport {
        authoritative_snapshot_id: outcome.snapshot_id,
        manifest_hash: outcome.manifest.manifest_hash,
        replica_version_time: binding.replica_version_time,
        idempotent: outcome.idempotent,
    })
}
