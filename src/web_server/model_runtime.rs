use axum::Json;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::Path as FsPath;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

static RUNTIME_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize, Default)]
pub struct RealtimeInstancesRequest {
    pub refnos: Option<Vec<String>>,
    pub dbnum: Option<u32>,
    pub include_tubings: Option<bool>,
    pub enable_holes: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ParquetIncrementalEnqueueRequest {
    pub dbnum: Option<u32>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ModelUnitVersionQuery {
    pub dbnum: Option<u32>,
}

pub fn ensure_runtime_started() {
    if !RUNTIME_STARTED.swap(true, Ordering::SeqCst) {
        log::warn!("[model-runtime] 当前为占位实现：后台 worker 未启用");
    }
}

pub async fn api_realtime_instances_by_refnos(
    Json(payload): Json<RealtimeInstancesRequest>,
) -> impl IntoResponse {
    let raw_refnos = payload.refnos.unwrap_or_default();
    let requested_count = raw_refnos.len();
    let mut parsed_refnos = Vec::new();
    let mut parse_failed = Vec::new();

    for raw in &raw_refnos {
        match aios_core::RefnoEnum::from_str(raw) {
            Ok(refno) => parsed_refnos.push(refno),
            Err(_) => parse_failed.push(raw.clone()),
        }
    }

    if !parse_failed.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "dbnum": payload.dbnum.unwrap_or_default(),
                "requested_count": requested_count,
                "returned_count": 0,
                "missing_refnos": raw_refnos,
                "instances_by_refno": {},
                "message": format!("无法解析 refno: {}", parse_failed.join(", "))
            })),
        );
    }

    if parsed_refnos.is_empty() {
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "dbnum": payload.dbnum.unwrap_or_default(),
                "requested_count": requested_count,
                "returned_count": 0,
                "missing_refnos": [],
                "instances_by_refno": {},
                "message": "refnos 为空"
            })),
        );
    }

    let enable_holes = payload.enable_holes.unwrap_or(true);
    match query_realtime_instance_entries(&parsed_refnos, enable_holes).await {
        Ok(instances_by_refno) => {
            let missing_refnos = raw_refnos
                .iter()
                .map(|item| normalize_refno_key(item))
                .filter(|key| !instances_by_refno.contains_key(key))
                .collect::<Vec<_>>();
            let returned_count = instances_by_refno.values().map(Vec::len).sum::<usize>();
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "dbnum": payload.dbnum.unwrap_or_default(),
                    "requested_count": requested_count,
                    "returned_count": returned_count,
                    "missing_refnos": missing_refnos,
                    "instances_by_refno": instances_by_refno,
                    "message": format!("返回 {} 个实例", returned_count)
                })),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "dbnum": payload.dbnum.unwrap_or_default(),
                "requested_count": requested_count,
                "returned_count": 0,
                "missing_refnos": raw_refnos,
                "instances_by_refno": {},
                "message": err.to_string()
            })),
        ),
    }
}

pub async fn api_parquet_incremental_enqueue(
    Json(payload): Json<ParquetIncrementalEnqueueRequest>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "success": false,
            "message": "parquet 增量队列暂未启用（占位实现）",
            "dbnum": payload.dbnum,
            "reason": payload.reason
        })),
    )
}

pub async fn api_parquet_version(Path(dbno): Path<u32>) -> impl IntoResponse {
    if let Some(info) = find_parquet_manifest(dbno) {
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "dbnum": dbno,
                "revision": info.revision,
                "updated_at": info.updated_at,
                "manifest_base_dir": "parquet",
                "files_base_dir": "parquet",
                "running": false,
                "pending_count": 0,
                "last_error": null,
                "source": "manifest",
                "project_name": info.project_name,
            })),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "success": false,
            "dbnum": dbno,
            "revision": 0,
            "updated_at": null,
            "manifest_base_dir": null,
            "files_base_dir": null,
            "running": false,
            "pending_count": 0,
            "last_error": format!("dbnum={dbno} 未找到 Parquet manifest"),
            "source": "manifest-missing",
            // Keep the legacy keys for older clients that only inspect dbno/version.
            "dbno": dbno,
            "version": 0,
        })),
    )
}

pub async fn api_model_unit_versions(
    Path(unit_refno): Path<String>,
    Query(query): Query<ModelUnitVersionQuery>,
) -> impl IntoResponse {
    let dbnum = match resolve_model_unit_dbnum(&unit_refno, query.dbnum) {
        Ok(dbnum) => dbnum,
        Err(error) => return model_unit_error(StatusCode::BAD_REQUEST, error),
    };
    let unit_refno = normalize_refno_key(&unit_refno);
    match crate::versioned_db::model_unit_commit::list_model_unit_commits(dbnum, &unit_refno).await
    {
        Ok(commits) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "data": commits.into_iter().map(model_unit_commit_json).collect::<Vec<_>>(),
            })),
        ),
        Err(error) => model_unit_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub async fn api_model_unit_version(
    Path((unit_refno, sesno)): Path<(String, u32)>,
    Query(query): Query<ModelUnitVersionQuery>,
) -> impl IntoResponse {
    let dbnum = match resolve_model_unit_dbnum(&unit_refno, query.dbnum) {
        Ok(dbnum) => dbnum,
        Err(error) => return model_unit_error(StatusCode::BAD_REQUEST, error),
    };
    let unit_refno = normalize_refno_key(&unit_refno);
    match crate::versioned_db::model_unit_commit::model_unit_commit(dbnum, &unit_refno, sesno).await
    {
        Ok(Some(commit)) => (
            StatusCode::OK,
            Json(json!({ "success": true, "data": model_unit_commit_json(commit) })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": format!("未找到模型提交: ({dbnum}, {unit_refno}, {sesno})"),
            })),
        ),
        Err(error) => model_unit_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub async fn api_latest_model_unit_version(
    Path(unit_refno): Path<String>,
    Query(query): Query<ModelUnitVersionQuery>,
) -> impl IntoResponse {
    let dbnum = match resolve_model_unit_dbnum(&unit_refno, query.dbnum) {
        Ok(dbnum) => dbnum,
        Err(error) => return model_unit_error(StatusCode::BAD_REQUEST, error),
    };
    let unit_refno = normalize_refno_key(&unit_refno);
    match crate::versioned_db::model_unit_commit::latest_model_unit_commit(dbnum, &unit_refno).await
    {
        Ok(Some(commit)) => (
            StatusCode::OK,
            Json(json!({ "success": true, "data": model_unit_commit_json(commit) })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": format!("未找到模型提交: ({dbnum}, {unit_refno})"),
            })),
        ),
        Err(error) => model_unit_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

fn model_unit_commit_json(
    commit: crate::versioned_db::model_unit_commit::ModelUnitCommit,
) -> serde_json::Value {
    json!({
        "manifest_url": commit.manifest_url(),
        "commit": commit,
    })
}

fn resolve_model_unit_dbnum(unit_refno: &str, dbnum: Option<u32>) -> anyhow::Result<u32> {
    if let Some(dbnum) = dbnum {
        anyhow::ensure!(dbnum > 0, "dbnum must be non-zero");
        return Ok(dbnum);
    }
    let refno = aios_core::RefnoEnum::from_str(unit_refno)
        .map_err(|error| anyhow::anyhow!("无法解析模型单元 refno={unit_refno}: {error:?}"))?;
    crate::data_interface::db_meta_manager::resolve_dbnum_for_refno(refno)
}

fn model_unit_error(
    status: StatusCode,
    error: anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(json!({ "success": false, "message": error.to_string() })),
    )
}

struct ParquetManifestInfo {
    project_name: String,
    revision: u64,
    updated_at: Option<String>,
}

fn find_parquet_manifest(dbno: u32) -> Option<ParquetManifestInfo> {
    let output_root = crate::versioned_db::db_meta_info::get_output_root();
    let manifest_file = format!("manifest_{dbno}.json");
    let mut candidates = Vec::new();

    let configured_project = aios_core::get_db_option().project_name.trim().to_string();
    if !configured_project.is_empty() {
        candidates.push(output_root.join(&configured_project));
    }

    if let Ok(entries) = fs::read_dir(&output_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if candidates.iter().any(|candidate| candidate == &path) {
                continue;
            }
            candidates.push(path);
        }
    }

    for project_dir in candidates {
        let manifest = project_dir.join("parquet").join(&manifest_file);
        if !manifest.exists() {
            continue;
        }
        let project_name = project_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        return Some(ParquetManifestInfo {
            project_name,
            revision: manifest_revision(&manifest),
            updated_at: manifest_updated_at(&manifest),
        });
    }

    None
}

fn manifest_revision(path: &FsPath) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(system_time_revision)
        .unwrap_or(0)
}

fn system_time_revision(time: SystemTime) -> Option<u64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn manifest_updated_at(path: &FsPath) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|value| {
            value
                .get("generated_at")
                .and_then(|item| item.as_str())
                .map(ToOwned::to_owned)
        })
}

async fn query_realtime_instance_entries(
    refnos: &[aios_core::RefnoEnum],
    enable_holes: bool,
) -> anyhow::Result<HashMap<String, Vec<serde_json::Value>>> {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    let geom_insts =
        aios_core::rs_surreal::inst::query_insts_with_batch(refnos, enable_holes, Some(50)).await?;
    let export_data = crate::fast_model::export_model::collect_export_data(
        geom_insts,
        refnos,
        &mesh_dir,
        false,
        Some(refnos),
        false,
    )
    .await?;

    let mut out: HashMap<String, Vec<serde_json::Value>> = HashMap::new();

    for comp in export_data.components {
        let refno_key = normalize_refno_key(&comp.refno.to_string());
        let refno_transform = matrix_to_json(comp.world_transform);
        let aabb = aabb_to_json(comp.aabb.as_ref());

        for (idx, geo) in comp.geometries.iter().enumerate() {
            let matrix = comp.world_transform * geo.geo_transform;
            out.entry(refno_key.clone()).or_default().push(json!({
                "geo_hash": geo.geo_hash,
                "matrix": matrix_to_json(matrix),
                "geo_index": idx,
                "color_index": 0,
                "name_index": 0,
                "site_name_index": 0,
                "lod_mask": 1,
                "uniforms": {
                    "refno": refno_key,
                    "noun": comp.noun.clone(),
                    "name": comp.name.clone(),
                    "owner_refno": comp.owner_refno.map(|r| normalize_refno_key(&r.to_string())),
                    "owner_noun": comp.owner_noun.clone(),
                    "spec_value": comp.spec_value.unwrap_or(0),
                    "has_neg": comp.has_neg
                },
                "refno_transform": refno_transform.clone(),
                "aabb": aabb.clone()
            }));
        }
    }

    for tubi in export_data.tubings {
        let tubi_key = normalize_refno_key(&tubi.refno.to_string());
        let owner_key = normalize_refno_key(&tubi.owner_refno.to_string());
        let entry = json!({
            "geo_hash": tubi.geo_hash,
            "matrix": matrix_to_json(tubi.transform),
            "geo_index": tubi.index,
            "color_index": 0,
            "name_index": 0,
            "site_name_index": 0,
            "lod_mask": 1,
            "uniforms": {
                "refno": tubi_key,
                "noun": "TUBI",
                "name": tubi.name,
                "owner_refno": owner_key.clone(),
                "owner_noun": "BRAN",
                "spec_value": tubi.spec_value.unwrap_or(0),
                "has_neg": false
            },
            "refno_transform": matrix_to_json(tubi.transform),
            "aabb": aabb_to_json(tubi.aabb.as_ref())
        });

        out.entry(tubi_key.clone()).or_default().push(entry.clone());
        if owner_key != tubi_key {
            out.entry(owner_key).or_default().push(entry);
        }
    }

    Ok(out)
}

fn normalize_refno_key(raw: &str) -> String {
    raw.trim().replace('/', "_")
}

fn matrix_to_json(matrix: glam::DMat4) -> Vec<f64> {
    matrix.to_cols_array().to_vec()
}

fn aabb_to_json(aabb: Option<&aios_core::types::PlantAabb>) -> serde_json::Value {
    match aabb {
        Some(value) => json!({
            "min": [value.mins().x, value.mins().y, value.mins().z],
            "max": [value.maxs().x, value.maxs().y, value.maxs().z]
        }),
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::ModelUnitVersionQuery;

    #[test]
    fn model_unit_version_query_allows_omitted_dbnum() {
        let query: ModelUnitVersionQuery = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(query.dbnum, None);
    }
}
