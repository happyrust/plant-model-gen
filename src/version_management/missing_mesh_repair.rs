use crate::fast_model::gen_model::mesh_generate::gen_inst_meshes_by_geo_ids_with_state;
use crate::fast_model::gen_model::mesh_state::mesh_file_exists_in_dir;
use crate::options::DbOptionExt;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::utils::RecordIdExt;
use aios_core::{RecordId, SurrealQueryExt, project_primary_db};
use anyhow::Context;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use surrealdb::types::SurrealValue;

const DEGRADED_FRADIUS_FALLBACK_ENV: &str = "AIOS_CSG_ALLOW_DEGRADED_PROFILE_FALLBACK";
const DEGRADED_FRADIUS_FALLBACK_LOG_ENV: &str = "AIOS_CSG_DEGRADED_PROFILE_FALLBACK_LOG";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMissingMeshRepairRequest {
    pub project_name: String,
    pub dbnum: u32,
    pub report_file: PathBuf,
    pub mesh_root: PathBuf,
    pub limit: Option<usize>,
    pub dry_run: bool,
    pub retry_bad: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMissingMeshRepairRow {
    pub geo_hash: String,
    pub before_exists: bool,
    pub after_exists: bool,
    pub inst_geo_found: bool,
    pub has_param: bool,
    pub was_bad: bool,
    pub attempted: bool,
    pub generated_now: bool,
    pub still_missing: bool,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMissingMeshRepairResponse {
    pub project_name: String,
    pub dbnum: u32,
    pub report_file: PathBuf,
    pub mesh_root: PathBuf,
    pub dry_run: bool,
    pub retry_bad: bool,
    pub degraded_fradius_fallback_enabled: bool,
    pub degraded_fradius_fallback_log: Option<PathBuf>,
    pub degraded_fradius_fallback_rows: usize,
    pub requested_hashes: usize,
    pub limited: bool,
    pub invalid_hashes: usize,
    pub skipped_existing: usize,
    pub missing_inst_geo: usize,
    pub param_missing: usize,
    pub non_renderable_inputs: usize,
    pub self_intersecting_inputs: usize,
    pub bad_skipped: usize,
    pub attempted_hashes: usize,
    pub generated_hashes: usize,
    pub still_missing_hashes: usize,
    pub rows: Vec<ModelMissingMeshRepairRow>,
    pub recommended_action: String,
}

#[derive(Debug, Deserialize)]
struct MissingMeshReportFile {
    missing_geo_hash_list: Vec<MissingMeshReportEntry>,
}

#[derive(Debug, Deserialize)]
struct MissingMeshReportEntry {
    geo_hash: String,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct InstGeoRepairCandidate {
    id: RecordId,
    has_param: bool,
    param: Option<PdmsGeoParam>,
    bad: bool,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct InstGeoRepairStatus {
    id: RecordId,
    meshed: bool,
    bad: bool,
}

struct ParamRepairBlocker {
    status: &'static str,
    message: String,
}

pub async fn repair_missing_meshes(
    db_option_ext: &DbOptionExt,
    request: ModelMissingMeshRepairRequest,
) -> anyhow::Result<ModelMissingMeshRepairResponse> {
    let degraded_fradius_fallback_enabled = env_bool(DEGRADED_FRADIUS_FALLBACK_ENV);
    let degraded_fradius_fallback_log = env_path(DEGRADED_FRADIUS_FALLBACK_LOG_ENV);

    if !request.report_file.is_file() {
        anyhow::bail!(
            "missing mesh report does not exist or is not a file: {}",
            request.report_file.display()
        );
    }
    crate::fast_model::utils::ensure_surreal_init().await?;

    let raw = fs::read_to_string(&request.report_file).with_context(|| {
        format!(
            "read missing mesh report failed: {}",
            request.report_file.display()
        )
    })?;
    let report: MissingMeshReportFile =
        serde_json::from_str(&raw).context("parse missing mesh report JSON")?;

    let requested_hashes = report.missing_geo_hash_list.len();
    let mut invalid_hashes = 0usize;
    let mut seen = HashSet::new();
    let mut hashes = Vec::new();

    for entry in report.missing_geo_hash_list {
        let trimmed = entry.geo_hash.trim();
        if is_builtin_geo_hash(trimmed) {
            continue;
        }
        match trimmed.parse::<u64>() {
            Ok(hash) => {
                if seen.insert(hash) {
                    hashes.push(hash);
                }
            }
            Err(_) => invalid_hashes += 1,
        }
    }

    let limited = request
        .limit
        .map(|limit| {
            if hashes.len() > limit {
                hashes.truncate(limit);
                true
            } else {
                false
            }
        })
        .unwrap_or(false);

    let before_exists = hashes
        .iter()
        .map(|hash| (*hash, mesh_file_exists_in_dir(&request.mesh_root, *hash)))
        .collect::<HashMap<_, _>>();

    let missing_before = hashes
        .iter()
        .copied()
        .filter(|hash| !before_exists.get(hash).copied().unwrap_or(false))
        .collect::<Vec<_>>();

    let candidates = query_inst_geo_candidates(&missing_before).await?;
    let candidates_by_hash = candidates
        .into_iter()
        .map(|candidate| {
            let hash = candidate.id.to_mesh_id().parse::<u64>().unwrap_or(0);
            (hash, candidate)
        })
        .collect::<HashMap<_, _>>();

    let mut rows = Vec::with_capacity(hashes.len());
    let mut eligible_ids = Vec::new();

    for hash in &hashes {
        let before = before_exists.get(hash).copied().unwrap_or(false);
        if before {
            rows.push(ModelMissingMeshRepairRow {
                geo_hash: hash.to_string(),
                before_exists: true,
                after_exists: true,
                inst_geo_found: true,
                has_param: true,
                was_bad: false,
                attempted: false,
                generated_now: false,
                still_missing: false,
                status: "already_present".to_string(),
                message: "mesh file already exists; rerun export to refresh manifest".to_string(),
            });
            continue;
        }

        let Some(candidate) = candidates_by_hash.get(hash) else {
            rows.push(ModelMissingMeshRepairRow {
                geo_hash: hash.to_string(),
                before_exists: false,
                after_exists: false,
                inst_geo_found: false,
                has_param: false,
                was_bad: false,
                attempted: false,
                generated_now: false,
                still_missing: true,
                status: "missing_inst_geo".to_string(),
                message: "inst_geo row was not found in the current SurrealDB namespace"
                    .to_string(),
            });
            continue;
        };

        if !candidate.has_param {
            rows.push(ModelMissingMeshRepairRow {
                geo_hash: hash.to_string(),
                before_exists: false,
                after_exists: false,
                inst_geo_found: true,
                has_param: false,
                was_bad: candidate.bad,
                attempted: false,
                generated_now: false,
                still_missing: true,
                status: "param_missing".to_string(),
                message: "inst_geo row has no param and cannot be regenerated".to_string(),
            });
            continue;
        }

        if let Some(blocker) =
            classify_param_before_retry(candidate.param.as_ref(), degraded_fradius_fallback_enabled)
        {
            rows.push(ModelMissingMeshRepairRow {
                geo_hash: hash.to_string(),
                before_exists: false,
                after_exists: false,
                inst_geo_found: true,
                has_param: true,
                was_bad: candidate.bad,
                attempted: false,
                generated_now: false,
                still_missing: true,
                status: blocker.status.to_string(),
                message: blocker.message,
            });
            continue;
        }

        if candidate.bad && !request.retry_bad {
            rows.push(ModelMissingMeshRepairRow {
                geo_hash: hash.to_string(),
                before_exists: false,
                after_exists: false,
                inst_geo_found: true,
                has_param: true,
                was_bad: true,
                attempted: false,
                generated_now: false,
                still_missing: true,
                status: "bad_skipped".to_string(),
                message: "inst_geo is marked bad; pass --retry-bad to regenerate anyway"
                    .to_string(),
            });
            continue;
        }

        eligible_ids.push(candidate.id.clone());
        rows.push(ModelMissingMeshRepairRow {
            geo_hash: hash.to_string(),
            before_exists: false,
            after_exists: false,
            inst_geo_found: true,
            has_param: true,
            was_bad: candidate.bad,
            attempted: !request.dry_run,
            generated_now: false,
            still_missing: true,
            status: if request.dry_run {
                "dry_run_eligible".to_string()
            } else {
                "attempted".to_string()
            },
            message: if request.dry_run {
                "eligible for mesh repair; no generation executed".to_string()
            } else {
                "mesh repair attempted".to_string()
            },
        });
    }

    if !request.dry_run && !eligible_ids.is_empty() {
        let precision = db_option_ext.inner.mesh_precision().clone();
        let mut mesh_formats = db_option_ext.mesh_formats.clone();
        if !mesh_formats.contains(&crate::options::MeshFormat::Glb) {
            mesh_formats.push(crate::options::MeshFormat::Glb);
        }

        gen_inst_meshes_by_geo_ids_with_state(
            &request.mesh_root,
            &precision,
            &eligible_ids,
            &mesh_formats,
            true,
        )
        .await?;
    }

    let after_status = query_inst_geo_statuses(&eligible_ids).await?;
    let after_status_by_hash = after_status
        .into_iter()
        .map(|status| {
            let hash = status.id.to_mesh_id().parse::<u64>().unwrap_or(0);
            (hash, status)
        })
        .collect::<HashMap<_, _>>();

    for row in &mut rows {
        let Ok(hash) = row.geo_hash.parse::<u64>() else {
            continue;
        };
        let after_exists = mesh_file_exists_in_dir(&request.mesh_root, hash);
        row.after_exists = after_exists;
        row.generated_now = !row.before_exists && after_exists;
        row.still_missing = !after_exists && !matches!(row.status.as_str(), "dry_run_eligible");

        if request.dry_run {
            continue;
        }

        if row.attempted {
            if after_exists {
                row.status = "generated".to_string();
                row.message =
                    "mesh file generated; rerun Parquet export and validation".to_string();
            } else if let Some(status) = after_status_by_hash.get(&hash) {
                if let Some(message) = candidates_by_hash.get(&hash).and_then(|candidate| {
                    extrusion_self_intersection_message(candidate.param.as_ref())
                }) {
                    row.status = "self_intersecting_input".to_string();
                    row.message = format!(
                        "generation did not produce a GLB; {message}. Repair source geometry or keep the release degraded/quarantined."
                    );
                } else if status.bad {
                    row.status = "generation_failed_bad".to_string();
                    row.message =
                        "generation did not produce a GLB and inst_geo is marked bad".to_string();
                } else if status.meshed {
                    row.status = "meshed_without_file".to_string();
                    row.message =
                        "inst_geo is meshed but no expected GLB candidate exists".to_string();
                } else {
                    row.status = "still_missing".to_string();
                    row.message = "generation did not produce a GLB".to_string();
                }
            } else {
                row.status = "still_missing".to_string();
                row.message = "generation did not produce a GLB".to_string();
            }
        }
    }

    let skipped_existing = rows
        .iter()
        .filter(|row| row.status == "already_present")
        .count();
    let missing_inst_geo = rows
        .iter()
        .filter(|row| row.status == "missing_inst_geo")
        .count();
    let param_missing = rows
        .iter()
        .filter(|row| row.status == "param_missing")
        .count();
    let non_renderable_inputs = rows
        .iter()
        .filter(|row| row.status == "non_renderable_input")
        .count();
    let self_intersecting_inputs = rows
        .iter()
        .filter(|row| row.status == "self_intersecting_input")
        .count();
    let bad_skipped = rows
        .iter()
        .filter(|row| row.status == "bad_skipped")
        .count();
    let attempted_hashes = rows.iter().filter(|row| row.attempted).count();
    let generated_hashes = rows.iter().filter(|row| row.generated_now).count();
    let still_missing_hashes = rows
        .iter()
        .filter(|row| row.still_missing && row.status != "dry_run_eligible")
        .count();
    let degraded_fradius_fallback_rows =
        count_log_lines(degraded_fradius_fallback_log.as_ref()).unwrap_or(0);

    let recommended_action = if request.dry_run {
        "Dry run only. Re-run without --dry-run to generate eligible missing meshes.".to_string()
    } else if degraded_fradius_fallback_rows > 0 {
        "Generated one or more meshes with degraded FRADIUS fallback. Review the approximation and keep the repair report as operational evidence."
            .to_string()
    } else if self_intersecting_inputs > 0 {
        "Review self_intersecting_input rows as source-data/profile defects and repair the source profile."
            .to_string()
    } else if still_missing_hashes == 0 {
        "Rerun the required viewer export and verify that all referenced mesh files exist."
            .to_string()
    } else if non_renderable_inputs > 0 {
        "Review non_renderable_input rows as source-data defects and repair upstream geometry."
            .to_string()
    } else {
        "Review rows with still_missing=true; classify non-visual geometry or fix generation."
            .to_string()
    };

    Ok(ModelMissingMeshRepairResponse {
        project_name: request.project_name,
        dbnum: request.dbnum,
        report_file: request.report_file,
        mesh_root: request.mesh_root,
        dry_run: request.dry_run,
        retry_bad: request.retry_bad,
        degraded_fradius_fallback_enabled,
        degraded_fradius_fallback_log,
        degraded_fradius_fallback_rows,
        requested_hashes,
        limited,
        invalid_hashes,
        skipped_existing,
        missing_inst_geo,
        param_missing,
        non_renderable_inputs,
        self_intersecting_inputs,
        bad_skipped,
        attempted_hashes,
        generated_hashes,
        still_missing_hashes,
        rows,
        recommended_action,
    })
}

async fn query_inst_geo_candidates(hashes: &[u64]) -> anyhow::Result<Vec<InstGeoRepairCandidate>> {
    if hashes.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows = Vec::new();
    for chunk in hashes.chunks(200) {
        let ids = chunk.iter().map(|hash| inst_geo_id(*hash)).join(",");
        let sql = format!(
            "SELECT id, param != NONE AS has_param, param, bad ?? false AS bad FROM [{}]",
            ids
        );
        let mut chunk_rows: Vec<InstGeoRepairCandidate> =
            project_primary_db().query_take(&sql, 0).await?;
        rows.append(&mut chunk_rows);
    }
    Ok(rows)
}

async fn query_inst_geo_statuses(ids: &[RecordId]) -> anyhow::Result<Vec<InstGeoRepairStatus>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows = Vec::new();
    for chunk in ids.chunks(200) {
        let ids = chunk.iter().map(|id| id.to_raw()).join(",");
        let sql = format!(
            "SELECT id, meshed ?? false AS meshed, bad ?? false AS bad FROM [{}]",
            ids
        );
        let mut chunk_rows: Vec<InstGeoRepairStatus> =
            project_primary_db().query_take(&sql, 0).await?;
        rows.append(&mut chunk_rows);
    }
    Ok(rows)
}

fn inst_geo_id(hash: u64) -> String {
    format!("inst_geo:⟨{}⟩", hash)
}

fn is_builtin_geo_hash(geo_hash: &str) -> bool {
    matches!(geo_hash.trim(), "0" | "1" | "2" | "3")
}

fn classify_param_before_retry(
    param: Option<&PdmsGeoParam>,
    allow_degraded_fallback: bool,
) -> Option<ParamRepairBlocker> {
    let Some(param) = param else {
        return Some(ParamRepairBlocker {
            status: "non_renderable_input",
            message: "inst_geo param is missing and cannot be rendered".to_string(),
        });
    };
    match param {
        PdmsGeoParam::Unknown | PdmsGeoParam::CompoundShape => Some(ParamRepairBlocker {
            status: "non_renderable_input",
            message: format!(
                "non-renderable source geometry: {} has no concrete mesh generator",
                param.type_name()
            ),
        }),
        PdmsGeoParam::PrimExtrusion(extrusion) => {
            if extrusion.height.abs() <= f32::EPSILON {
                return Some(ParamRepairBlocker {
                    status: "non_renderable_input",
                    message: "non-renderable source geometry: PrimExtrusion height is zero"
                        .to_string(),
                });
            }
            if extrusion.verts.is_empty() {
                return Some(ParamRepairBlocker {
                    status: "non_renderable_input",
                    message: "non-renderable source geometry: PrimExtrusion has no wires"
                        .to_string(),
                });
            }
            let valid_wires = extrusion
                .verts
                .iter()
                .filter(|wire| count_distinct_points(wire) >= 3)
                .count();
            if valid_wires == 0 {
                return Some(ParamRepairBlocker {
                    status: "non_renderable_input",
                    message:
                        "non-renderable source geometry: PrimExtrusion has no wire with at least 3 distinct points"
                            .to_string(),
                });
            }
            let self_intersections = extrusion_self_intersection_count(extrusion);
            if self_intersections > 0 {
                let has_fradius = extrusion_has_nonzero_fradius(extrusion);
                if !has_fradius || !allow_degraded_fallback {
                    let fallback_hint = if has_fradius {
                        "; enable AIOS_CSG_ALLOW_DEGRADED_PROFILE_FALLBACK=1 only for degraded diagnostic repair"
                    } else {
                        ""
                    };
                    return Some(ParamRepairBlocker {
                        status: "self_intersecting_input",
                        message: format!(
                            "source geometry: PrimExtrusion wire self-intersects ({} crossing segment pairs){}",
                            self_intersections, fallback_hint
                        ),
                    });
                }
            }
            None
        }
        PdmsGeoParam::PrimPolyhedron(polyhedron) => {
            let has_renderable_loop = polyhedron.polygons.iter().any(|polygon| {
                polygon
                    .loops
                    .iter()
                    .any(|loop_points| count_distinct_points(loop_points) >= 3)
            });
            if !has_renderable_loop {
                return Some(ParamRepairBlocker {
                    status: "non_renderable_input",
                    message:
                        "non-renderable source geometry: PrimPolyhedron has no polygon loop with at least 3 distinct points"
                            .to_string(),
                });
            }
            None
        }
        _ => None,
    }
}

fn extrusion_self_intersection_message(param: Option<&PdmsGeoParam>) -> Option<String> {
    let Some(PdmsGeoParam::PrimExtrusion(extrusion)) = param else {
        return None;
    };
    let count = extrusion_self_intersection_count(extrusion);
    if count == 0 {
        return None;
    }
    Some(format!(
        "source geometry: PrimExtrusion wire self-intersects ({} crossing segment pairs)",
        count
    ))
}

fn extrusion_has_nonzero_fradius(extrusion: &aios_core::prim_geo::extrusion::Extrusion) -> bool {
    extrusion
        .verts
        .iter()
        .flat_map(|wire| wire.iter())
        .any(|point| point.z.abs() > 1e-6)
}

fn extrusion_self_intersection_count(
    extrusion: &aios_core::prim_geo::extrusion::Extrusion,
) -> usize {
    extrusion
        .verts
        .iter()
        .map(|wire| wire_self_intersection_count(wire))
        .sum()
}

fn wire_self_intersection_count(points: &[glam::Vec3]) -> usize {
    let points = normalized_wire_points(points, 0.01);
    if points.len() < 4 {
        return 0;
    }

    let mut count = 0usize;
    let n = points.len();
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        for j in (i + 1)..n {
            if i.abs_diff(j) <= 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let c = points[j];
            let d = points[(j + 1) % n];
            if segments_intersect_2d(a, b, c, d, 1e-4) {
                count += 1;
            }
        }
    }
    count
}

fn normalized_wire_points(points: &[glam::Vec3], tol: f32) -> Vec<glam::Vec2> {
    let mut cleaned: Vec<glam::Vec2> = Vec::with_capacity(points.len());
    for point in points {
        let point_2d = glam::Vec2::new(point.x, point.y);
        if cleaned
            .last()
            .map(|last| last.distance(point_2d) < tol)
            .unwrap_or(false)
        {
            continue;
        }
        cleaned.push(point_2d);
    }
    if cleaned.len() > 1
        && cleaned
            .first()
            .zip(cleaned.last())
            .map(|(first, last)| first.distance(*last) < tol)
            .unwrap_or(false)
    {
        cleaned.pop();
    }
    cleaned
}

fn segments_intersect_2d(
    a: glam::Vec2,
    b: glam::Vec2,
    c: glam::Vec2,
    d: glam::Vec2,
    eps: f32,
) -> bool {
    let o1 = orient_2d(a, b, c, eps);
    let o2 = orient_2d(a, b, d, eps);
    let o3 = orient_2d(c, d, a, eps);
    let o4 = orient_2d(c, d, b, eps);

    if o1 == 0 && point_on_segment_2d(c, a, b, eps) {
        return true;
    }
    if o2 == 0 && point_on_segment_2d(d, a, b, eps) {
        return true;
    }
    if o3 == 0 && point_on_segment_2d(a, c, d, eps) {
        return true;
    }
    if o4 == 0 && point_on_segment_2d(b, c, d, eps) {
        return true;
    }
    o1 != o2 && o3 != o4
}

fn orient_2d(a: glam::Vec2, b: glam::Vec2, c: glam::Vec2, eps: f32) -> i8 {
    let cross = (b - a).perp_dot(c - a);
    if cross.abs() <= eps {
        0
    } else if cross > 0.0 {
        1
    } else {
        -1
    }
}

fn point_on_segment_2d(p: glam::Vec2, a: glam::Vec2, b: glam::Vec2, eps: f32) -> bool {
    let ab = b - a;
    let ap = p - a;
    if ab.length_squared() <= eps * eps {
        return ap.length() <= eps;
    }
    ab.perp_dot(ap).abs() <= eps * ab.length()
        && ap.dot(ab) >= -eps
        && ap.dot(ab) <= ab.length_squared() + eps
}

fn count_distinct_points(points: &[glam::Vec3]) -> usize {
    let mut distinct: Vec<glam::Vec3> = Vec::new();
    for point in points {
        if !distinct
            .iter()
            .any(|existing| existing.distance(*point) < 0.01)
        {
            distinct.push(*point);
        }
    }
    distinct.len()
}

fn env_bool(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).and_then(|value| {
        let value = value.to_string_lossy();
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn count_log_lines(path: Option<&PathBuf>) -> anyhow::Result<usize> {
    let Some(path) = path else {
        return Ok(0);
    };
    if !path.is_file() {
        return Ok(0);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read degraded FRADIUS fallback log: {}", path.display()))?;
    Ok(raw.lines().filter(|line| !line.trim().is_empty()).count())
}
