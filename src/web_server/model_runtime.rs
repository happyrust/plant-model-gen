use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

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
    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "dbno": dbno,
            "version": 0,
            "source": "placeholder"
        })),
    )
}

async fn query_realtime_instance_entries(
    refnos: &[aios_core::RefnoEnum],
    enable_holes: bool,
) -> anyhow::Result<HashMap<String, Vec<serde_json::Value>>> {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    let geom_insts = aios_core::rs_surreal::inst::query_insts_with_batch(
        refnos,
        enable_holes,
        Some(50),
    )
    .await?;
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
