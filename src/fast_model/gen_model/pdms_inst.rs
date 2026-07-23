use std::collections::{BTreeSet, HashMap, hash_map::Entry};

use aios_core::Transform;
use aios_core::geometry::{EleGeosInfo, EleInstGeo, EleInstGeosData, ShapeInstancesData};
use aios_core::parsed_data::TubiInfoData;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::*;
use aios_core::shape::pdms_shape::RsVec3;
use aios_core::types::*;
use aios_core::{
    SurrealQueryExt, gen_aabb_hash, gen_plant_transform_hash, gen_string_hash, get_db_option,
    project_primary_db,
};
use dashmap::DashMap;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use itertools::Itertools;
use rkyv::vec;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

use parry3d::bounding_volume::{Aabb, BoundingVolume};
use parry3d::math::Point;

use super::mesh_generate::MeshResult;
use super::model_record_id::{
    geo_relate_id, geo_relate_id_for_inst, model_ref0_range, model_refno_id, model_refno_range,
    neg_relate_id, ngmr_relate_id, refno_id_parts, tubi_relate_id,
};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::fast_model::debug_model_debug;
use crate::fast_model::shared::aabb_apply_transform;
// use crate::fast_model::EXIST_MESH_GEOS;

const MAX_FAILED_SQL_DUMPS_PER_RUN: usize = 20;
static FAILED_SQL_DUMP_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 本次运行的 failed_sql 转储计数（spec 004 任务指标采集用）。
pub fn failed_sql_dump_count() -> usize {
    FAILED_SQL_DUMP_COUNT.load(Ordering::Relaxed)
}

fn infer_failed_sql_stage(query: &str) -> &'static str {
    if query.contains("INSERT IGNORE INTO inst_relate_aabb") {
        "inst_relate_aabb"
    } else if query.contains("INTO inst_relate [") {
        "inst_relate"
    } else if query.contains("INTO geo_relate [") {
        "geo_relate"
    } else if query.contains("INSERT IGNORE INTO inst_geo") {
        "inst_geo"
    } else if query.contains("INSERT IGNORE INTO inst_info") {
        "inst_info"
    } else if query.contains("INSERT IGNORE INTO aabb") {
        "aabb"
    } else if query.contains("INSERT IGNORE INTO trans") {
        "trans"
    } else if query.contains("INSERT IGNORE INTO vec3") {
        "vec3"
    } else if query.contains("INTO tubi_info") {
        "tubi_info"
    } else if query.contains("INTO neg_relate [") {
        "neg_relate"
    } else {
        "transaction_batch"
    }
}

fn failed_sql_dump_dir() -> PathBuf {
    let db_option = crate::options::get_db_option_ext();
    let base = db_option.get_project_output_dir();
    base.join("diagnostics").join("failed_sql")
}

fn dump_failed_sql_batch(
    query: &str,
    err_msg: &str,
    attempt: usize,
    max_retries: usize,
) -> std::io::Result<Option<PathBuf>> {
    let dump_idx = FAILED_SQL_DUMP_COUNT.fetch_add(1, Ordering::Relaxed);
    if dump_idx >= MAX_FAILED_SQL_DUMPS_PER_RUN {
        return Ok(None);
    }

    let stage = infer_failed_sql_stage(query);
    let now = chrono::Local::now();
    let epoch_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dump_dir = failed_sql_dump_dir();
    std::fs::create_dir_all(&dump_dir)?;

    let file_path = dump_dir.join(format!(
        "{}_{}_pid{}_{}.surql",
        now.format("%Y%m%d_%H%M%S"),
        stage,
        std::process::id(),
        epoch_nanos
    ));

    let payload = format!(
        "-- generated_at: {}\n-- stage: {}\n-- retries: {}/{}\n-- error: {}\n-- dump_index: {}/{}\n\n{}\n",
        now.to_rfc3339(),
        stage,
        attempt,
        max_retries,
        err_msg.replace('\n', " | "),
        dump_idx + 1,
        MAX_FAILED_SQL_DUMPS_PER_RUN,
        query
    );
    std::fs::write(&file_path, payload)?;
    Ok(Some(file_path))
}

#[derive(Debug, Default, Clone)]
pub struct SaveInstanceDataReport {
    pub missing_neg_carriers: Vec<RefnoEnum>,
}

async fn delete_inst_relate_by_in_with_dbnum(
    refnos: &[RefnoEnum],
    chunk_size: usize,
    dbnum: u32,
) -> anyhow::Result<()> {
    for sql in build_delete_inst_relate_by_in_sql(refnos, chunk_size, Some(dbnum)) {
        project_primary_db().query_response(&sql).await?.check()?;
    }
    Ok(())
}

/// replace_exist=true 时，删除指定 inst_info 的 geo_relate（关系表）记录，避免旧几何残留导致同一实例出现多份 Pos。
async fn delete_geo_relate_by_inst_info_ids(
    inst_info_ids: &[String],
    chunk_size: usize,
) -> anyhow::Result<()> {
    for sql in build_delete_geo_relate_by_inst_info_ids_sql(inst_info_ids, chunk_size) {
        project_primary_db().query_response(&sql).await?.check()?;
    }
    Ok(())
}

/// replace_exist=true 时，按载体(pe) 删除 neg_relate/ngmr_relate。
///
/// 为什么用 pe 而不用 out：
/// - out 是正实体（如 WALL），多个 batch 共享同一 target
/// - 按 out 删除会跨 batch 覆盖（无论并发还是顺序执行）
/// - pe 是负载体（如 FIXING），每个 batch 独有，按 pe 删除并发安全
async fn delete_boolean_relations_by_carriers(
    carrier_refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<()> {
    for sql in build_delete_boolean_relations_by_carriers_sql(carrier_refnos, chunk_size) {
        project_primary_db().query_response(&sql).await?.check()?;
    }
    Ok(())
}

/// replace_exist=true 时，清理实例/元件库布尔结果表，避免导出链路误读“历史 booled mesh”。
///
/// 典型症状：
/// - 当前轮生成/关系扫描显示 neg/ngmr=0（不会触发布尔 worker），
/// - 但旧 `inst_relate_bool` 仍残留 status=Success，导致导出优先使用旧的 booled mesh，
///   表现为模型出现莫名缺口/截面不对。
async fn delete_inst_relate_bool_records(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    for sql in build_delete_inst_relate_bool_records_sql(refnos, chunk_size) {
        project_primary_db().query_response(&sql).await?.check()?;
    }
    Ok(())
}

/// replace_exist=true 时，删除目标 BRAN/HANG 的所有 tubi_relate 直段记录。
///
/// 典型症状：
/// - BRAN/HANG 重新生成后，新世界坐标直段已写入；
/// - 但旧的局部坐标 tubi_relate 仍残留在同一 branch range 下；
/// - 导出阶段按 BRAN/sesno 前缀全量读取时，会把新旧两套直段一起带出。
pub(crate) async fn delete_tubi_relate_by_branch_refnos(
    branch_refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<()> {
    if branch_refnos.is_empty() {
        return Ok(());
    }

    for sql in build_delete_tubi_relate_by_branch_refnos_sql(branch_refnos, chunk_size) {
        project_primary_db().query_response(&sql).await?.check()?;
    }
    Ok(())
}

/// 从本次运行合并后的 `ShapeInstancesData` 一次性持久化 TUBI 关系。
/// 生成阶段只产出 `inst_tubi_map`，不得提前写 Surreal 再由后续阶段回读。
pub async fn persist_tubi_relations_from_artifacts(
    artifacts: &ShapeInstancesData,
) -> anyhow::Result<usize> {
    if artifacts.inst_tubi_map.is_empty() {
        return Ok(0);
    }

    let mut rows = artifacts.inst_tubi_map.values().collect::<Vec<_>>();
    rows.sort_by_key(|info| {
        (
            info.owner_refno,
            info.tubi
                .as_ref()
                .and_then(|tubi| tubi.index)
                .unwrap_or_default(),
            info.refno,
        )
    });
    let branch_refnos = rows
        .iter()
        .map(|info| info.owner_refno)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    delete_tubi_relate_by_branch_refnos(&branch_refnos, 100).await?;

    let mut transforms = HashMap::new();
    let aabbs = DashMap::new();
    let points = DashMap::new();
    let mut statements = Vec::with_capacity(rows.len());
    for info in rows {
        let tubi = info
            .tubi
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 tubi payload: {}", info.refno))?;
        let arrive_refno = tubi
            .arrive_refno
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 arrive_refno: {}", info.refno))?;
        let index = tubi
            .index
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 index: {}", info.refno))?;
        let start = tubi
            .start_pt
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 start_pt: {}", info.refno))?;
        let end = tubi
            .end_pt
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 end_pt: {}", info.refno))?;
        let arrive_axis = tubi
            .arrive_axis_pt
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 arrive_axis: {}", info.refno))?;
        let leave_axis = tubi
            .leave_axis_pt
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 leave_axis: {}", info.refno))?;
        let aabb = info
            .aabb
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 aabb: {}", info.refno))?;
        let geo_hash = info
            .cata_hash
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("TUBI artifact 缺少 geo hash: {}", info.refno))?;

        let trans_hash = gen_plant_transform_hash(&info.world_transform);
        transforms
            .entry(trans_hash)
            .or_insert(serde_json::to_string(&info.world_transform)?);
        let aabb_hash = gen_aabb_hash(&aabb);
        aabbs.entry(aabb_hash.to_string()).or_insert(aabb);

        let start = RsVec3(start);
        let end = RsVec3(end);
        let arrive_axis = RsVec3(arrive_axis.into());
        let leave_axis = RsVec3(leave_axis.into());
        let start_hash = start.gen_hash();
        let end_hash = end.gen_hash();
        let arrive_hash = arrive_axis.gen_hash();
        let leave_hash = leave_axis.gen_hash();
        points
            .entry(start_hash)
            .or_insert(serde_json::to_string(&start)?);
        points
            .entry(end_hash)
            .or_insert(serde_json::to_string(&end)?);
        points
            .entry(arrive_hash)
            .or_insert(serde_json::to_string(&arrive_axis)?);
        points
            .entry(leave_hash)
            .or_insert(serde_json::to_string(&leave_axis)?);

        let relation_id = tubi_relate_id(info.owner_refno, index as usize);
        let bore_size = serde_json::to_string(tubi.bore_size.as_deref().unwrap_or_default())?;
        statements.push(format!(
            "RELATE {}->{}->{} SET geo=inst_geo:⟨{geo_hash}⟩, \
             aabb=aabb:⟨{aabb_hash}⟩, world_trans=trans:⟨{trans_hash}⟩, \
             start_pt=vec3:⟨{start_hash}⟩, end_pt=vec3:⟨{end_hash}⟩, \
             arrive_axis=vec3:⟨{arrive_hash}⟩, leave_axis=vec3:⟨{leave_hash}⟩, \
             bore_size={bore_size}, bad=false, system={}, dt=fn::ses_date({});",
            info.refno.to_pe_key(),
            relation_id,
            arrive_refno.to_pe_key(),
            info.owner_refno.to_pe_key(),
            info.refno.to_pe_key(),
        ));
    }

    crate::fast_model::utils::save_transforms_to_surreal(&transforms).await?;
    crate::fast_model::utils::save_aabb_to_surreal(&aabbs).await;
    crate::fast_model::utils::save_pts_to_surreal(&points).await;
    let mut batcher = TransactionBatcher::new(4, 2);
    for statement in &statements {
        batcher.push(statement.clone()).await?;
    }
    batcher.finish().await?;
    Ok(statements.len())
}

fn build_delete_inst_relate_bool_records_sql(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> Vec<String> {
    if refnos.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for chunk in refnos.chunks(chunk_size.max(1)) {
        let bool_ids = chunk
            .iter()
            .map(|r| model_refno_id("inst_relate_bool", *r))
            .collect::<Vec<_>>()
            .join(",");

        // 使用 “DELETE [ids]” 点删，避免全表扫描。
        out.push(format!("DELETE [{bool_ids}];"));
    }
    out
}

fn build_delete_tubi_relate_by_branch_refnos_sql(
    branch_refnos: &[RefnoEnum],
    chunk_size: usize,
) -> Vec<String> {
    if branch_refnos.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for chunk in branch_refnos.chunks(chunk_size.max(1)) {
        let mut statements = Vec::with_capacity(chunk.len());
        for branch_refno in chunk {
            statements.push(format!(
                "LET $ids = SELECT VALUE id FROM {}; DELETE $ids;",
                model_refno_range("tubi_relate", *branch_refno)
            ));
        }
        out.push(statements.join("\n"));
    }
    out
}

/// O2：单精确 id 表的删除按整块批成「每表一条列表点删」
/// `DELETE [t:[a,b], t:[c,d], …];`——与 `build_delete_inst_relate_bool_records_sql`
/// 同一已验证形式（数组 record-id 列表点删），把每块约 5×N 条单删收敛到 5 条，
/// 大幅降低 SurrealQL 语句解析/规划开销。空输入返回空串。
fn build_delete_exact_model_records_sql(refnos: &[RefnoEnum]) -> String {
    if refnos.is_empty() {
        return String::new();
    }
    let mut sql = String::new();
    for table in [
        "inst_relate",
        "inst_relate_aabb",
        "inst_relate_bool",
        "inst_relate_cata_bool",
        "refno_relations",
    ] {
        let ids = refnos
            .iter()
            .map(|refno| model_refno_id(table, *refno))
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!("DELETE [{ids}];\n"));
    }
    sql
}

/// 区间表（neg_relate/ngmr_relate/geo_relate）按 refno 区间删除。保留已验证的
/// `LET $ids = SELECT VALUE id FROM <range>; DELETE $ids;` 形式；直接
/// `DELETE <range>` 需先用区间删除探针确认锁定版本 SurrealDB 支持后再切换（O1）。
fn build_delete_range_model_records_by_refno_sql(refno: RefnoEnum) -> String {
    let mut sql = String::new();
    for table in ["neg_relate", "ngmr_relate", "geo_relate"] {
        let range = model_refno_range(table, refno);
        sql.push_str(&format!(
            "LET $ids = SELECT VALUE id FROM {range}; DELETE $ids;\n"
        ));
    }
    sql
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn build_delete_inst_relate_bool_records_sql_should_not_delete_cata_bool() {
        let refnos = vec![RefnoEnum::from_str("24381/1").unwrap()];
        let sqls = build_delete_inst_relate_bool_records_sql(&refnos, 100);
        assert!(!sqls.is_empty());
        assert!(sqls.iter().all(|s| !s.contains("inst_relate_cata_bool")));
    }

    #[test]
    fn build_delete_tubi_relate_by_branch_refnos_sql_should_use_id_range() {
        let refnos = vec![
            RefnoEnum::from_str("24381/145569").unwrap(),
            RefnoEnum::from_str("24381/145570").unwrap(),
        ];
        let sqls = build_delete_tubi_relate_by_branch_refnos_sql(&refnos, 100);
        assert_eq!(sqls.len(), 1);
        for refno in refnos {
            assert!(sqls[0].contains(&model_refno_range("tubi_relate", refno)));
        }
    }

    #[test]
    fn dedupe_inst_relate_aabb_rows_keeps_last_row_for_duplicate_id() {
        let rows = vec![
            "{id: inst_relate_aabb:⟨1⟩, aabb_id: aabb:⟨old⟩}".to_string(),
            "{id: inst_relate_aabb:⟨2⟩, aabb_id: aabb:⟨other⟩}".to_string(),
            "{id: inst_relate_aabb:⟨1⟩, aabb_id: aabb:⟨new⟩}".to_string(),
        ];
        let ids = vec![
            "inst_relate_aabb:⟨1⟩".to_string(),
            "inst_relate_aabb:⟨2⟩".to_string(),
            "inst_relate_aabb:⟨1⟩".to_string(),
        ];

        let (deduped_rows, deduped_ids) = dedupe_inst_relate_aabb_rows(&rows, &ids);

        assert_eq!(
            deduped_ids,
            vec!["inst_relate_aabb:⟨1⟩", "inst_relate_aabb:⟨2⟩"]
        );
        assert_eq!(deduped_rows.len(), 2);
        assert!(deduped_rows[0].contains("aabb:⟨new⟩"));
        assert!(deduped_rows[1].contains("aabb:⟨other⟩"));
    }
}

/// replace_exist=true 时，删除本次将要重建的 inst_geo 记录（按 geo_hash 点删）。
///
/// 说明：inst_geo 写入目前使用 `INSERT IGNORE`，若不先删除，则旧记录（含 unit_flag/param）会被保留，
/// 导致“代码已修、--regen-model 已跑、但数据库仍是旧值”的假象。
async fn delete_inst_geo_by_hashes(geo_hashes: &[u64], chunk_size: usize) -> anyhow::Result<()> {
    for sql in build_delete_inst_geo_by_hashes_sql(geo_hashes, chunk_size) {
        project_primary_db().query_response(&sql).await?.check()?;
    }
    Ok(())
}

fn parse_inst_geo_hash(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    let normalized = trimmed
        .strip_prefix("inst_geo:`")
        .map(|s| s.trim_end_matches('`'))
        .or_else(|| {
            trimmed
                .strip_prefix("inst_geo:⟨")
                .map(|s| s.trim_end_matches('⟩'))
        })
        .or_else(|| {
            trimmed
                .strip_prefix("inst_geo:")
                .map(|s| s.trim_matches('`').trim_matches('⟨').trim_matches('⟩'))
        })
        .unwrap_or(trimmed);
    normalized.parse::<u64>().ok()
}

fn build_delete_inst_relate_by_in_sql(
    refnos: &[RefnoEnum],
    chunk_size: usize,
    _dbnum: Option<u32>,
) -> Vec<String> {
    if refnos.is_empty() {
        return Vec::new();
    }
    let mut sqls = Vec::new();
    for chunk in refnos.chunks(chunk_size.max(1)) {
        let relation_ids = chunk
            .iter()
            .map(|r| model_refno_id("inst_relate", *r))
            .collect::<Vec<_>>();
        let delete_by_id_sql = relation_ids
            .iter()
            .map(|id| format!("DELETE {id};"))
            .collect::<Vec<_>>()
            .join("\n");
        sqls.push(delete_by_id_sql);
    }
    sqls
}

async fn query_refno_dbnum_map(refnos: &[RefnoEnum], chunk_size: usize) -> HashMap<RefnoEnum, u32> {
    if refnos.is_empty() {
        return HashMap::new();
    }

    let mut dbnum_map: HashMap<RefnoEnum, u32> = HashMap::with_capacity(refnos.len());
    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();
    let mut missing_refnos = Vec::new();
    for &refno in refnos {
        let dbnum = db_meta
            .get_dbnum_by_refno(refno)
            .or_else(|| crate::fast_model::db_meta_cache::get_dbnum_for_refno(refno))
            .unwrap_or(0);
        if dbnum == 0 {
            missing_refnos.push(refno);
        }
        dbnum_map.insert(refno, dbnum);
    }

    if missing_refnos.is_empty() {
        return dbnum_map;
    }

    let mut refno_by_rid: HashMap<String, RefnoEnum> = HashMap::with_capacity(missing_refnos.len());
    for &refno in &missing_refnos {
        refno_by_rid.insert(format!("{}", refno.refno()), refno);
    }

    for chunk in missing_refnos.chunks(chunk_size.max(1)) {
        let ids = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT record::id(id) AS rid, dbnum FROM [{}];", ids);

        match project_primary_db().query_response(&sql).await {
            Ok(mut resp) => {
                let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
                for row in rows {
                    let Some(rid) = row.get("rid").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Some(refno) = refno_by_rid.get(rid).copied() else {
                        continue;
                    };
                    let dbnum = row.get("dbnum").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    dbnum_map.insert(refno, dbnum);
                }
            }
            Err(e) => {
                eprintln!(
                    "[query_refno_dbnum_map] 批量查询 pe.dbnum 失败 (chunk={}): {}",
                    chunk.len(),
                    e
                );
            }
        }
    }

    dbnum_map
}

fn build_delete_geo_relate_by_inst_info_ids_sql(
    inst_info_ids: &[String],
    chunk_size: usize,
) -> Vec<String> {
    if inst_info_ids.is_empty() {
        return Vec::new();
    }
    let mut sqls = Vec::new();
    for chunk in inst_info_ids.chunks(chunk_size.max(1)) {
        let in_keys = chunk
            .iter()
            .map(|id| format!("inst_info:⟨{}⟩", id))
            .collect::<Vec<_>>()
            .join(",");
        sqls.push(format!(
            "LET $ids = SELECT VALUE id FROM [{in_keys}]->geo_relate;\nDELETE $ids;"
        ));
    }
    sqls
}

fn build_delete_boolean_relations_by_carriers_sql(
    carrier_refnos: &[RefnoEnum],
    chunk_size: usize,
) -> Vec<String> {
    if carrier_refnos.is_empty() {
        return Vec::new();
    }
    let mut sqls = Vec::new();
    for chunk in carrier_refnos.chunks(chunk_size.max(1)) {
        let pe_conditions = chunk
            .iter()
            .map(|r| format!("pe = {}", r.to_pe_key()))
            .collect::<Vec<_>>()
            .join(" OR ");
        sqls.push(format!(
            "LET $ids = SELECT VALUE id FROM neg_relate WHERE {pe_conditions};\nDELETE $ids;"
        ));
        sqls.push(format!(
            "LET $ids = SELECT VALUE id FROM ngmr_relate WHERE {pe_conditions};\nDELETE $ids;"
        ));
    }
    sqls
}

fn build_delete_inst_geo_by_hashes_sql(geo_hashes: &[u64], chunk_size: usize) -> Vec<String> {
    if geo_hashes.is_empty() {
        return Vec::new();
    }
    let mut sqls = Vec::new();
    for chunk in geo_hashes.chunks(chunk_size.max(1)) {
        // 避免删掉内置 unit mesh（0..10），这些由程序内置加载并复用
        let ids = chunk
            .iter()
            .copied()
            .filter(|h| *h >= 10)
            .map(|h| format!("inst_geo:{h}"))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            continue;
        }
        sqls.push(format!("DELETE [{}];", ids.join(",")));
    }
    sqls
}

fn dedupe_cleanup_refnos(refnos: impl IntoIterator<Item = RefnoEnum>) -> Vec<RefnoEnum> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for refno in refnos {
        if refno.is_valid() && seen.insert(refno) {
            out.push(refno);
        }
    }
    out
}

async fn query_cleanup_refnos_or_seed(seed_refnos: &[RefnoEnum]) -> Vec<RefnoEnum> {
    match crate::fast_model::query_provider::query_multi_descendants_with_self(
        seed_refnos,
        &[],
        true,
    )
    .await
    {
        Ok(refnos) => dedupe_cleanup_refnos(refnos.into_iter().chain(seed_refnos.iter().copied())),
        Err(e) => {
            eprintln!(
                "[pre_cleanup_for_regen] TreeIndex 展开全部后代失败，降级清理 seed roots 自身: {}",
                e
            );
            dedupe_cleanup_refnos(seed_refnos.iter().copied())
        }
    }
}

async fn filter_seed_bran_hang_by_attr(seed_refnos: &[RefnoEnum]) -> Vec<RefnoEnum> {
    let mut out = Vec::new();
    for &refno in seed_refnos {
        let Ok(att) = aios_core::get_named_attmap(refno).await else {
            continue;
        };
        let noun = att.get_type_str();
        if noun == "BRAN" || noun == "HANG" {
            out.push(refno);
        }
    }
    dedupe_cleanup_refnos(out)
}

async fn query_cleanup_bran_hang_or_seed(seed_refnos: &[RefnoEnum]) -> Vec<RefnoEnum> {
    match crate::fast_model::query_provider::query_multi_descendants_with_self(
        seed_refnos,
        &["BRAN", "HANG"],
        true,
    )
    .await
    {
        Ok(refnos) => dedupe_cleanup_refnos(refnos),
        Err(e) => {
            eprintln!(
                "[pre_cleanup_for_regen] TreeIndex 展开 BRAN/HANG 后代失败，降级通过属性识别 seed roots: {}",
                e
            );
            filter_seed_bran_hang_by_attr(seed_refnos).await
        }
    }
}

/// 模型重新生成前的预处理清理
///
/// 在 `--regen-model` 等 replace_exist=true 场景下，于生成流程启动前一次性删除
/// 目标 refnos（及其后代）的所有关联模型记录，包括：
/// - inst_geo（几何参数，跳过内置 hash < 10）
/// - geo_relate（几何关系）
/// - inst_relate（实例关系）
/// - inst_relate_bool（布尔运算结果）
/// - neg_relate / ngmr_relate（负实体 / 交叉负实体关系）
///
/// 将清理逻辑集中到前处理阶段，避免与并行的 mesh worker 产生竞态条件
/// （此前 DELETE + INSERT IGNORE 在 save_instance_data_optimize 中执行，
///   会覆盖 mesh worker 已写入的 meshed=true）。
pub async fn pre_cleanup_for_regen(
    seed_refnos: &[RefnoEnum],
    whole_dbnum_scope: bool,
) -> anyhow::Result<()> {
    pre_cleanup_for_regen_inner(seed_refnos, None, whole_dbnum_scope).await
}

pub async fn pre_cleanup_for_regen_versioned(
    seed_refnos: &[RefnoEnum],
    hierarchy: &crate::generation_read::HierarchySnapshot,
) -> anyhow::Result<()> {
    // 版本化增量路径始终是部分范围（子树），不得走整库 ref0 快路径。
    pre_cleanup_for_regen_inner(seed_refnos, Some(hierarchy), false).await
}

/// O6 整库快路径：仅当调用方判定“范围覆盖整个 dbnum”（--regen-model 无子 refno 过滤）时使用。
/// 按 ref0（= dbnum 的 db 文件号）区间批量清理各模型表，用与逐 refno 相同的已验证
/// `LET $ids = SELECT VALUE id FROM <range>; DELETE $ids;` 形式（只是把区间放大到整 ref0），
/// 把 O(元素数) 条语句压到每 dbnum ~10 条。正确性由调用范围门保证：整库范围下该 ref0 的
/// 全部元素都在重生成，ref0 区间删除恰好命中且不会误删同库其它 ZONE。
async fn pre_cleanup_ref0_range_for_refnos(all_refnos: &[RefnoEnum]) -> anyhow::Result<()> {
    let ref0s: BTreeSet<u32> = all_refnos
        .iter()
        .map(|refno| refno_id_parts(*refno).ref0)
        .collect();
    if ref0s.is_empty() {
        return Ok(());
    }
    let t = Instant::now();
    println!(
        "[pre_cleanup_for_regen] 整库快路径: 按 {} 个 dbnum(ref0) 区间清理",
        ref0s.len()
    );
    for ref0 in ref0s {
        // inst_geo 按 geo_relate.out 收集的 hash 点删（与逐 refno 路径一致；跳过内置 <10）。
        let geo_range = model_ref0_range("geo_relate", ref0);
        let mut resp = project_primary_db()
            .query_response(&format!("SELECT VALUE record::id(out) FROM {geo_range};"))
            .await?;
        let geo_rows: Vec<String> = resp.take(0)?;
        let hashes = geo_rows
            .iter()
            .filter_map(|s| parse_inst_geo_hash(s))
            .collect::<Vec<_>>();
        if !hashes.is_empty() {
            delete_inst_geo_by_hashes(&hashes, 200).await?;
        }
        // 各模型表按 ref0 区间删除（精确 id 表 id 首元素即 ref0，区间同样命中）。
        let mut cleanup_sql = String::new();
        for table in [
            "inst_relate",
            "inst_relate_aabb",
            "inst_relate_bool",
            "inst_relate_cata_bool",
            "refno_relations",
            "neg_relate",
            "ngmr_relate",
            "geo_relate",
            "tubi_relate",
        ] {
            let range = model_ref0_range(table, ref0);
            cleanup_sql.push_str(&format!(
                "LET $ids = SELECT VALUE id FROM {range}; DELETE $ids;\n"
            ));
        }
        project_primary_db()
            .query_response(&cleanup_sql)
            .await?
            .check()?;
    }
    println!(
        "[pre_cleanup_for_regen] 整库快路径完成，耗时 {} ms",
        t.elapsed().as_millis()
    );
    Ok(())
}

async fn pre_cleanup_for_regen_inner(
    seed_refnos: &[RefnoEnum],
    hierarchy: Option<&crate::generation_read::HierarchySnapshot>,
    whole_dbnum_fast_path: bool,
) -> anyhow::Result<()> {
    if seed_refnos.is_empty() {
        return Ok(());
    }

    const CHUNK_SIZE: usize = 200;

    // 版本化正式路径严格使用会话 hierarchy；legacy 调用保留旧查询入口。
    let (all_refnos, bran_refnos) = if let Some(hierarchy) = hierarchy {
        // delete-only / 先增后删：删除的（或从未进入已发布模型切面的）refno 不在该
        // hierarchy 里。cleanup 只清理"曾存在于此切面"的产物——先过滤掉缺席根再展开，
        // 缺席根跳过而非硬失败（plan §4.3/§7；descendants() 对缺席根会返回
        // MissingRequiredData，正是 delete-only cleanup 的崩溃点）。
        let present_roots = seed_refnos
            .iter()
            .copied()
            .filter(|refno| hierarchy.node(*refno).is_some())
            .collect::<Vec<_>>();
        let skipped = seed_refnos.len().saturating_sub(present_roots.len());
        if skipped > 0 {
            println!(
                "[pre_cleanup_for_regen] 跳过 {skipped} 个不在 cleanup 切面的根（删除/先增后删，无已发布产物需清理）"
            );
        }
        if present_roots.is_empty() {
            return Ok(());
        }
        let all_refnos = hierarchy.descendants(
            &present_roots,
            &crate::generation_read::HierarchyQuery {
                include_self: true,
                nouns: BTreeSet::new(),
                max_depth: None,
                prune_on_match: false,
            },
        )?;
        let bran_refnos = all_refnos
            .iter()
            .copied()
            .filter(|refno| {
                hierarchy
                    .node(*refno)
                    .is_some_and(|node| matches!(node.noun.as_str(), "BRAN" | "HANG"))
            })
            .collect();
        (all_refnos, bran_refnos)
    } else {
        (
            query_cleanup_refnos_or_seed(seed_refnos).await,
            query_cleanup_bran_hang_or_seed(seed_refnos).await,
        )
    };

    println!(
        "[pre_cleanup_for_regen] seed_refnos={}, 展开后 all_refnos={}, bran_or_hang={}",
        seed_refnos.len(),
        all_refnos.len(),
        bran_refnos.len()
    );

    if all_refnos.is_empty() {
        return Ok(());
    }

    // O6：整库范围（--regen-model 无子 refno 过滤）→ 按 ref0（dbnum）区间批量清理，
    // 替代 O(元素数) 的逐 refno 删除；仅整库范围启用，保证不误删同库其它 ZONE。
    if whole_dbnum_fast_path {
        return pre_cleanup_ref0_range_for_refnos(&all_refnos).await;
    }

    let t = Instant::now();

    // 使用 SurrealDB 3.1 array record id range 作为模型产物主清理路径。
    // inst_geo 不是 range-id 模型产物表，因此先从待删 geo_relate.out 收集 hash，再跳过内置 hash < 10 删除。
    use futures::stream::{self, StreamExt};
    let limit_concurrency =
        if get_db_option().effective_surrealdb().mode == aios_core::options::DbConnMode::Ws {
            4
        } else {
            16
        };

    let chunks = all_refnos
        .chunks(CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let total_chunks = chunks.len();
    let mut completed_chunks = 0usize;
    let mut last_progress = Instant::now();
    crate::perf_metrics::record_generate_progress(
        "pre_cleanup_for_regen_started",
        Some(&format!(
            "refnos={} chunks={} bran_or_hang={} concurrency={}",
            all_refnos.len(),
            total_chunks,
            bran_refnos.len(),
            limit_concurrency
        )),
        t.elapsed().as_millis() as u64,
    );
    let mut chunk_stream = stream::iter(chunks)
        .map(|chunk_vec| {
            tokio::spawn(async move {
                let mut cleanup_sql = String::new();
                let mut geo_query_sql = String::new();
                for refno in &chunk_vec {
                    let geo_range = model_refno_range("geo_relate", *refno);
                    geo_query_sql
                        .push_str(&format!("SELECT VALUE record::id(out) FROM {geo_range};\n"));
                    cleanup_sql.push_str(&build_delete_range_model_records_by_refno_sql(*refno));
                }
                // O2：5 张精确 id 表整块批量点删（每表一条列表删），替代每 refno 5 条单删。
                cleanup_sql.push_str(&build_delete_exact_model_records_sql(&chunk_vec));

                let mut geo_hashes = Vec::new();
                if !geo_query_sql.trim().is_empty() {
                    let mut resp = project_primary_db().query_response(&geo_query_sql).await?;
                    for stmt_idx in 0..chunk_vec.len() {
                        let rows: Vec<String> = resp.take(stmt_idx)?;
                        geo_hashes.extend(rows);
                    }
                }

                let hashes = geo_hashes
                    .iter()
                    .filter_map(|s| parse_inst_geo_hash(s))
                    .collect::<Vec<_>>();
                if !hashes.is_empty() {
                    delete_inst_geo_by_hashes(&hashes, CHUNK_SIZE).await?;
                }

                if !cleanup_sql.trim().is_empty() {
                    project_primary_db()
                        .query_response(&cleanup_sql)
                        .await?
                        .check()?;
                }

                Ok::<(), anyhow::Error>(())
            })
        })
        .buffer_unordered(limit_concurrency);

    let mut cleanup_errors = Vec::new();
    while let Some(res) = chunk_stream.next().await {
        completed_chunks += 1;
        match res {
            Ok(Err(e)) => cleanup_errors.push(format!(
                "range chunk {completed_chunks}/{total_chunks} 处理失败: {e}"
            )),
            Err(e) => cleanup_errors.push(format!(
                "range chunk {completed_chunks}/{total_chunks} tokio 任务崩溃: {e}"
            )),
            _ => {}
        }
        if completed_chunks == 1
            || completed_chunks == total_chunks
            || last_progress.elapsed() >= Duration::from_secs(10)
        {
            crate::perf_metrics::record_generate_progress(
                "pre_cleanup_for_regen_progress",
                Some(&format!(
                    "chunks={}/{} approx_refnos_done={} bran_or_hang={}",
                    completed_chunks,
                    total_chunks,
                    completed_chunks
                        .saturating_mul(CHUNK_SIZE)
                        .min(all_refnos.len()),
                    bran_refnos.len()
                )),
                t.elapsed().as_millis() as u64,
            );
            last_progress = Instant::now();
        }
    }
    if !cleanup_errors.is_empty() {
        anyhow::bail!(
            "pre_cleanup_for_regen 未完整完成，禁止继续模型生成:\n{}",
            cleanup_errors.join("\n")
        );
    }

    if !bran_refnos.is_empty() {
        crate::perf_metrics::record_generate_progress(
            "pre_cleanup_for_regen_tubi_cleanup",
            Some(&format!("bran_or_hang={}", bran_refnos.len())),
            t.elapsed().as_millis() as u64,
        );
        delete_tubi_relate_by_branch_refnos(&bran_refnos, CHUNK_SIZE).await?;

        // 兜底清空已退役的 `tubi_info` 表：它按内容哈希键、不带分支归属，无法按分支范围删，
        // 且不再被任何写入路径写入、也从无读取方（tubi 版本存储唯一真相源是 `tubi_relate`）。
        // 有分支参与 regen 时顺带幂等清空遗留行，避免历史遗留在 versioned 库里继续膨胀；
        // 清空后不再重新写入，后续 regen 命中空表即为廉价 no-op。
        project_primary_db()
            .query_response("DELETE tubi_info;")
            .await?
            .check()?;
    }

    crate::perf_metrics::record_generate_progress(
        "pre_cleanup_for_regen_done",
        Some(&format!(
            "refnos={} chunks={} bran_or_hang={}",
            all_refnos.len(),
            total_chunks,
            bran_refnos.len()
        )),
        t.elapsed().as_millis() as u64,
    );
    println!(
        "[pre_cleanup_for_regen] 清理完成 (array record-id range 模式)，耗时 {} ms",
        t.elapsed().as_millis()
    );

    Ok(())
}

/// 保存 instance 数据到数据库（事务化批处理版本）
#[cfg_attr(
    feature = "profile",
    tracing::instrument(skip_all, name = "save_instance_data_optimize")
)]
pub async fn save_instance_data_optimize(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
) -> anyhow::Result<()> {
    save_instance_data_with_report(inst_mgr, replace_exist, mesh_results, mesh_aabb_map, true)
        .await?;
    Ok(())
}

pub async fn save_instance_data_with_options(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
    write_inst_relate_aabb: bool,
) -> anyhow::Result<()> {
    save_instance_data_with_report(
        inst_mgr,
        replace_exist,
        mesh_results,
        mesh_aabb_map,
        write_inst_relate_aabb,
    )
    .await?;
    Ok(())
}

fn build_inst_key_carrier_map(
    inst_mgr: &ShapeInstancesData,
) -> HashMap<String, Vec<(RefnoEnum, String)>> {
    let mut carriers_by_inst_key: HashMap<String, Vec<(RefnoEnum, String)>> = HashMap::new();
    for (refno, info) in &inst_mgr.inst_info_map {
        carriers_by_inst_key
            .entry(info.get_inst_key())
            .or_default()
            .push((*refno, info.id_str()));
    }

    for carriers in carriers_by_inst_key.values_mut() {
        carriers.sort_unstable_by_key(|(refno, _)| *refno);
        carriers.dedup_by_key(|(refno, _)| *refno);
    }

    carriers_by_inst_key
}

pub async fn save_instance_data_with_report(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
    write_inst_relate_aabb: bool,
) -> anyhow::Result<SaveInstanceDataReport> {
    save_instance_data_with_report_inner(
        inst_mgr,
        replace_exist,
        mesh_results,
        mesh_aabb_map,
        write_inst_relate_aabb,
        None,
    )
    .await
}

pub async fn save_instance_data_with_report_versioned(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
    write_inst_relate_aabb: bool,
    precomputed: &InstRelatePrecomputed,
) -> anyhow::Result<SaveInstanceDataReport> {
    save_instance_data_with_report_inner(
        inst_mgr,
        replace_exist,
        mesh_results,
        mesh_aabb_map,
        write_inst_relate_aabb,
        Some(precomputed),
    )
    .await
}

async fn save_instance_data_with_report_inner(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
    write_inst_relate_aabb: bool,
    versioned_precomputed: Option<&InstRelatePrecomputed>,
) -> anyhow::Result<SaveInstanceDataReport> {
    debug_model_debug!(
        "save_instance_data_optimize start: inst_info={}, inst_geo_keys={}, tubi_keys={}, replace_exist={}, write_inst_relate_aabb={}",
        inst_mgr.inst_info_map.len(),
        inst_mgr.inst_geos_map.len(),
        inst_mgr.inst_tubi_map.len(),
        replace_exist,
        write_inst_relate_aabb
    );

    // 单条 INSERT 里拼接的记录数，过大容易触发 SurrealDB 事务取消/超时；取小一点更稳。
    const CHUNK_SIZE: usize = 100;
    // SurrealDB 在高并发/大事务时容易出现 session 丢失、匿名访问等错误；这里优先保证稳定性。
    const MAX_TX_STATEMENTS: usize = 4;
    // 本地 SurrealDB 在并发事务较高时更容易出现 “Transaction conflict: Resource busy”，
    // 这里降低并发以提升整体成功率（结合 TransactionBatcher 内部重试）。
    const MAX_CONCURRENT_TX: usize = 2;
    let mut report = SaveInstanceDataReport::default();
    let debug_filters: HashSet<String> = std::env::var("AIOS_DEBUG_NEG_RECONCILE")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|item| {
                    item.trim()
                        .trim_matches('`')
                        .trim_matches('⟨')
                        .trim_matches('⟩')
                })
                .filter_map(|item| {
                    let normalized = item
                        .strip_prefix("pe:")
                        .or_else(|| item.strip_prefix("pe:⟨"))
                        .unwrap_or(item)
                        .trim_matches('`')
                        .trim_matches('⟨')
                        .trim_matches('⟩')
                        .trim()
                        .to_string();
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let should_debug_neg_write = |carrier: &RefnoEnum, target: &RefnoEnum| -> bool {
        !debug_filters.is_empty()
            && (debug_filters.contains(&carrier.to_string())
                || debug_filters.contains(&target.to_string()))
    };
    let mut debug_neg_pairs: Vec<(RefnoEnum, RefnoEnum)> = Vec::new();
    let mut aabb_map: HashMap<u64, String> = HashMap::new();
    let mut transform_map: HashMap<u64, String> = HashMap::new();
    let inst_refnos: Vec<RefnoEnum> = inst_mgr.inst_info_map.keys().copied().collect();
    // 写入前不再逐批扫描删除 inst_relate：首次部署为空库，重生成场景的旧
    // 关系清理统一由入口 pre_cleanup_for_regen 完成（写入用 INSERT RELATION
    // IGNORE 幂等），避免空库部署时每批 refnos 白跑一次图遍历 DELETE。
    let legacy_precomputed = if versioned_precomputed.is_none() {
        Some(InstRelatePrecomputed::build(&inst_refnos).await)
    } else {
        None
    };
    let inst_relate_precomputed = versioned_precomputed
        .or(legacy_precomputed.as_ref())
        .expect("versioned or legacy precomputed metadata must exist");
    let inst_dbnum_map = if versioned_precomputed.is_some() {
        inst_refnos
            .iter()
            .map(|refno| (*refno, inst_relate_precomputed.dbnum(refno)))
            .collect()
    } else {
        query_refno_dbnum_map(&inst_refnos, CHUNK_SIZE).await
    };
    if let Entry::Vacant(entry) = transform_map.entry(0) {
        entry.insert(serde_json::to_string(&Transform::IDENTITY)?);
    }
    let mut vec3_map: HashMap<u64, String> = HashMap::new();

    // 收集 Neg 和 CataCrossNeg 类型的 geo_relate 映射
    // neg_geo_by_carrier: key=carrier_refno -> value=Vec<(geo_index, geo_relate_id)>
    //   用于 neg_relate: 通过负实体 refno 找到其所有 Neg 类型的 geo_relate
    // cata_cross_neg_geo_map: key=(carrier_refno, geom_refno) -> value=Vec<(geo_index, geo_relate_id)>
    //   用于 ngmr_relate: 通过 (负载体, ngmr_geom_refno) 找到对应的 CataCrossNeg geo_relate
    let mut neg_geo_by_carrier: HashMap<RefnoEnum, Vec<(usize, String)>> = HashMap::new();
    let mut cata_cross_neg_geo_map: HashMap<(RefnoEnum, RefnoEnum), Vec<(usize, String)>> =
        HashMap::new();
    let inst_key_carriers = build_inst_key_carrier_map(inst_mgr);

    // inst_geo & geo_relate
    let mut geo_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
    let mut inst_geo_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut geo_relate_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

    for inst_geo_data in inst_mgr.inst_geos_map.values() {
        for (geo_index, inst) in inst_geo_data.insts.iter().enumerate() {
            if inst.geo_transform.translation.is_nan()
                || inst.geo_transform.rotation.is_nan()
                || inst.geo_transform.scale.is_nan()
            {
                debug_model_debug!(
                    "[WARN] skip inst geo due to NaN transform: refno={:?}, geo_hash={}",
                    inst.refno,
                    inst.geo_hash
                );
                continue;
            }

            let transform_hash = gen_plant_transform_hash(&inst.geo_transform);
            if let Entry::Vacant(entry) = transform_map.entry(transform_hash) {
                entry.insert(serde_json::to_string(&inst.geo_transform)?);
            }

            let key_pts = inst.geo_param.key_points();
            let mut pt_hashes = Vec::with_capacity(key_pts.len());
            for key_pt in key_pts {
                let pts_hash = key_pt.gen_hash();
                pt_hashes.push(format!("vec3:⟨{}⟩", pts_hash));
                if let Entry::Vacant(entry) = vec3_map.entry(pts_hash) {
                    entry.insert(serde_json::to_string(&key_pt)?);
                }
            }

            let cat_negs_str = if !inst.cata_neg_refnos.is_empty() {
                format!(
                    ", cata_neg: [{}]",
                    inst.cata_neg_refnos.iter().map(|x| x.to_pe_key()).join(",")
                )
            } else {
                String::new()
            };

            use aios_core::geometry::GeoBasicType;
            let geom_refno = inst.refno;
            let carriers = inst_key_carriers
                .get(&inst_geo_data.id())
                .cloned()
                .unwrap_or_else(|| vec![(inst_geo_data.refno, inst_geo_data.id())]);
            for (carrier_refno, inst_info_id) in carriers {
                let relate_id = geo_relate_id_for_inst(carrier_refno, geo_index, &inst_info_id);
                let relate_json = format!(
                    r#"in: inst_info:⟨{0}⟩, out: inst_geo:⟨{1}⟩, trans: trans:⟨{2}⟩, geom_refno: pe:{3}, pts: [{4}], geo_type: '{5}', visible: {6} {7}"#,
                    inst_info_id,
                    inst.geo_hash,
                    transform_hash,
                    inst.refno,
                    pt_hashes.join(","),
                    inst.geo_type.to_string(),
                    inst.visible,
                    cat_negs_str
                );
                geo_relate_buffer.push(format!("{{ {relate_json}, id: {relate_id} }}"));
                match inst.geo_type {
                    GeoBasicType::Neg => {
                        // neg_relate: 按 carrier_refno 收集所有 Neg geo_relate
                        neg_geo_by_carrier
                            .entry(carrier_refno)
                            .or_insert_with(Vec::new)
                            .push((geo_index, relate_id));
                    }
                    GeoBasicType::CataCrossNeg => {
                        // ngmr_relate: 按 (carrier_refno, geom_refno) 收集 CataCrossNeg geo_relate
                        cata_cross_neg_geo_map
                            .entry((carrier_refno, geom_refno))
                            .or_insert_with(Vec::new)
                            .push((geo_index, relate_id));
                    }
                    _ => {}
                }

                if geo_relate_buffer.len() >= CHUNK_SIZE {
                    let statement = format!(
                        "INSERT RELATION IGNORE INTO geo_relate [{}];",
                        geo_relate_buffer.join(",")
                    );
                    geo_batcher.push(statement).await?;
                    geo_relate_buffer.clear();
                }
            }

            inst_geo_buffer.push(inst.gen_unit_geo_sur_json());

            if inst_geo_buffer.len() >= CHUNK_SIZE {
                let statement = format!(
                    "INSERT IGNORE INTO {} [{}];",
                    stringify!(inst_geo),
                    inst_geo_buffer.join(",")
                );
                geo_batcher.push(statement).await?;
                inst_geo_buffer.clear();
            }
        }
    }

    if !inst_geo_buffer.is_empty() {
        let statement = format!(
            "INSERT IGNORE INTO {} [{}];",
            stringify!(inst_geo),
            inst_geo_buffer.join(",")
        );
        geo_batcher.push(statement).await?;
        debug_model_debug!(
            "save_instance_data_optimize flushing remaining inst_geo records: {}",
            inst_geo_buffer.len()
        );
    }

    if !geo_relate_buffer.is_empty() {
        let statement = format!(
            "INSERT RELATION IGNORE INTO geo_relate [{}];",
            geo_relate_buffer.join(",")
        );
        geo_batcher.push(statement).await?;
        debug_model_debug!(
            "save_instance_data_optimize flushing remaining geo_relate records: {}",
            geo_relate_buffer.len()
        );
    }

    geo_batcher.finish().await?;

    // tubi -> aabb map
    for tubi in inst_mgr.inst_tubi_map.values() {
        if let Some(aabb) = tubi.aabb {
            let aabb_hash = gen_aabb_hash(&aabb);
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }
        }
    }

    // neg_relate - 新结构
    // 关系方向：切割几何 -[neg_relate]-> 正实体
    // - in: geo_relate ID (切割几何)
    // - out: 正实体 refno (被减实体)
    // - pe: 负实体 refno (负载体，原来的 in)
    if !inst_mgr.neg_relate_map.is_empty() {
        debug_model_debug!("开始创建 neg_relate 关系 (新结构: in=geo_relate):");
        for (target, refnos) in &inst_mgr.neg_relate_map {
            debug_model_debug!("  目标: {}, 负实体数量: {}", target, refnos.len());
        }

        // 跨 batch 缺口仅上报给 GenerationArtifacts；禁止从刚写入的模型表回查。
        // 所有 batch 汇总完成后，writer 会从完整内存事实重建确定性的 relation ID。
        let mut missing_carriers: HashSet<RefnoEnum> = HashSet::new();
        for neg_refnos in inst_mgr.neg_relate_map.values() {
            for neg_refno in neg_refnos.iter() {
                if !neg_geo_by_carrier.contains_key(neg_refno) {
                    missing_carriers.insert(*neg_refno);
                }
            }
        }
        report
            .missing_neg_carriers
            .extend(missing_carriers.iter().copied());

        let mut neg_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut neg_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (target, neg_refnos) in &inst_mgr.neg_relate_map {
            for neg_refno in neg_refnos.iter() {
                // 首先尝试从当前 batch 的 neg_geo_by_carrier 查找
                if let Some(geo_relate_ids) = neg_geo_by_carrier.get(neg_refno) {
                    for (geo_index, geo_relate_id) in geo_relate_ids {
                        let neg_id = neg_relate_id(*target, *neg_refno, *geo_index, 0);
                        neg_buffer.push(format!(
                            "{{ in: {0}, id: {3}, out: {2}, pe: {1} }}",
                            geo_relate_id,         // 切割几何
                            neg_refno.to_pe_key(), // 负载体
                            target.to_pe_key(),    // 正实体（被减实体）
                            neg_id,
                        ));
                        if should_debug_neg_write(neg_refno, target) {
                            println!(
                                "[neg-write-debug] enqueue target={} carrier={} geo_relate_id={}",
                                target, neg_refno, geo_relate_id
                            );
                            debug_neg_pairs.push((*target, *neg_refno));
                        }
                        if neg_buffer.len() >= CHUNK_SIZE {
                            let statement = if replace_exist {
                                format!(
                                    "INSERT RELATION IGNORE INTO neg_relate [{}];",
                                    neg_buffer.join(",")
                                )
                            } else {
                                format!(
                                    "INSERT RELATION IGNORE INTO neg_relate [{}];",
                                    neg_buffer.join(",")
                                )
                            };
                            neg_batcher.push(statement).await?;
                            neg_buffer.clear();
                        }
                    }
                }
            }
        }

        if !neg_buffer.is_empty() {
            let statement = if replace_exist {
                format!(
                    "INSERT RELATION IGNORE INTO neg_relate [{}];",
                    neg_buffer.join(",")
                )
            } else {
                format!(
                    "INSERT RELATION IGNORE INTO neg_relate [{}];",
                    neg_buffer.join(",")
                )
            };
            neg_batcher.push(statement).await?;
        }

        neg_batcher.finish().await?;
        if !debug_neg_pairs.is_empty() {
            debug_neg_pairs.sort_unstable();
            debug_neg_pairs.dedup();
            println!(
                "[neg-write-debug] queued relation pairs={}（禁止写后读，未执行 DB 验证查询）",
                debug_neg_pairs.len()
            );
        }
    }

    // ngmr_relate - 新结构
    // 关系方向：切割几何 -[ngmr_relate]-> 正实体
    // - in: geo_relate ID (CataCrossNeg 切割几何)
    // - out: 目标k (正实体)
    // - pe: ele_refno (负载体，原来的 in)
    // - ngmr: ngmr_geom_refno (NGMR 几何引用，保留用于调试)
    if !inst_mgr.ngmr_neg_relate_map.is_empty() {
        debug_model_debug!("开始创建 ngmr_relate 关系 (新结构: in=geo_relate):");
        for (k, refnos) in &inst_mgr.ngmr_neg_relate_map {
            debug_model_debug!("  目标: {}, NGMR 数量: {}", k, refnos.len());
        }

        let mut ngmr_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut ngmr_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (target_k, refnos) in &inst_mgr.ngmr_neg_relate_map {
            for (ele_refno, ngmr_geom_refno) in refnos {
                // 查找该 (负载体, ngmr_geom_refno) 的 CataCrossNeg geo_relate
                let key = (*ele_refno, *ngmr_geom_refno);
                if let Some(geo_relate_ids) = cata_cross_neg_geo_map.get(&key) {
                    for (geo_index, geo_relate_id) in geo_relate_ids {
                        let ele_pe = ele_refno.to_pe_key();
                        let target_pe = target_k.to_pe_key();
                        let ngmr_pe = ngmr_geom_refno.to_pe_key();
                        let ngmr_id = ngmr_relate_id(*target_k, *ele_refno, *geo_index, 0);
                        ngmr_buffer.push(format!(
                            "{{ in: {0}, id: {4}, out: {2}, pe: {1}, ngmr: {3} }}",
                            geo_relate_id, // 切割几何
                            ele_pe,        // 负载体
                            target_pe,     // 正实体（目标）
                            ngmr_pe,       // NGMR 几何引用
                            ngmr_id
                        ));
                        if ngmr_buffer.len() >= CHUNK_SIZE {
                            let statement = if replace_exist {
                                format!(
                                    "INSERT RELATION IGNORE INTO ngmr_relate [{}];",
                                    ngmr_buffer.join(",")
                                )
                            } else {
                                format!(
                                    "INSERT RELATION IGNORE INTO ngmr_relate [{}];",
                                    ngmr_buffer.join(",")
                                )
                            };
                            ngmr_batcher.push(statement).await?;
                            ngmr_buffer.clear();
                        }
                    }
                }
            }
        }

        if !ngmr_buffer.is_empty() {
            let statement = if replace_exist {
                format!(
                    "INSERT RELATION IGNORE INTO ngmr_relate [{}];",
                    ngmr_buffer.join(",")
                )
            } else {
                format!(
                    "INSERT RELATION IGNORE INTO ngmr_relate [{}];",
                    ngmr_buffer.join(",")
                )
            };
            ngmr_batcher.push(statement).await?;
        }

        ngmr_batcher.finish().await?;
    }

    // inst_info & inst_relate
    let mut inst_keys: Vec<RefnoEnum> = Vec::with_capacity(inst_mgr.inst_info_map.len());
    debug_model_debug!(
        "🔍 [DEBUG] inst_info_map keys: {:?}",
        inst_mgr.inst_info_map.keys().collect::<Vec<&RefnoEnum>>()
    );
    let mut inst_info_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
    let mut inst_info_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
    let mut inst_relate_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_ids: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_ids: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_chunks: Vec<(Vec<String>, Vec<String>)> = Vec::new();

    for (key, info) in &inst_mgr.inst_info_map {
        inst_keys.push(*key);

        if info.world_transform.translation.is_nan()
            || info.world_transform.rotation.is_nan()
            || info.world_transform.scale.is_nan()
        {
            continue;
        }

        // 使用完整格式存储 ptset（不压缩，方便调试和人工可读）
        inst_info_buffer.push(info.gen_sur_json_full());
        if inst_info_buffer.len() >= CHUNK_SIZE {
            let statement = format!(
                "INSERT IGNORE INTO {} [{}];",
                stringify!(inst_info),
                inst_info_buffer.join(",")
            );
            inst_info_batcher.push(statement).await?;
            inst_info_buffer.clear();
        }

        let resolved_aabb: Option<(u64, Aabb)> = resolve_element_world_aabb_for_index(
            info,
            inst_mgr.inst_geos_map.get(&info.get_inst_key()),
            mesh_results,
            mesh_aabb_map,
        )
        .map(|aabb| (gen_aabb_hash(&aabb), aabb));

        if let Some((aabb_hash, aabb)) = resolved_aabb {
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }

            let aabb_row_sql = format!(
                "{{id: {0}, refno: {1}, aabb: aabb:⟨{2}⟩, aabb_id: aabb:⟨{2}⟩}}",
                model_refno_id("inst_relate_aabb", *key),
                key.to_pe_key(),
                aabb_hash
            );
            inst_relate_aabb_buffer.push(aabb_row_sql);
            inst_relate_aabb_ids.push(model_refno_id("inst_relate_aabb", *key));
        }

        // inst_relate 不再保存 world_trans；世界变换统一从 pe_transform 获取。
        let dbnum = inst_dbnum_map
            .get(key)
            .copied()
            .filter(|dbnum| *dbnum != 0)
            .unwrap_or_else(|| inst_relate_precomputed.dbnum(key));
        let dt = inst_relate_precomputed.dt(key);
        let inst_relate_id = model_refno_id("inst_relate", *key);
        let relate_sql = format!(
            "{{id: {0}, in: {1}, out: inst_info:⟨{2}⟩, dbnum: {3}, zone_refno: NONE, spec_value: 0, dt: {4}, has_cata_neg: {5}, solid: {6}, owner_refno: {7}, owner_type: '{8}'}}",
            inst_relate_id,
            key.to_pe_key(),
            info.id_str(),
            dbnum,
            dt,
            info.has_cata_neg,
            info.is_solid,
            info.owner_refno.to_pe_key(),
            info.owner_type
        );

        inst_relate_buffer.push(relate_sql);
        inst_relate_ids.push(inst_relate_id);
        if inst_relate_buffer.len() >= CHUNK_SIZE {
            let statements = build_replace_rows_statements(
                "inst_relate",
                true,
                &inst_relate_buffer,
                &inst_relate_ids,
            );
            inst_relate_batcher.push_group(statements).await?;
            inst_relate_buffer.clear();
            inst_relate_ids.clear();

            // 延后处理 inst_relate_aabb（必须在 aabb UPSERT 之后写，避免 aabb_id 侧空记录 d=NONE）
            if !inst_relate_aabb_buffer.is_empty() {
                inst_relate_aabb_chunks.push((
                    std::mem::take(&mut inst_relate_aabb_buffer),
                    std::mem::take(&mut inst_relate_aabb_ids),
                ));
            }
        }
    }

    if !inst_relate_buffer.is_empty() {
        let statements = build_replace_rows_statements(
            "inst_relate",
            true,
            &inst_relate_buffer,
            &inst_relate_ids,
        );
        inst_relate_batcher.push_group(statements).await?;
        debug_model_debug!(
            "save_instance_data_optimize flushing inst_relate from inst_info_map: {}",
            inst_relate_buffer.len()
        );
    }

    // 注意：inst_relate_aabb.aabb_id 指向 aabb 表的记录。
    // 若先写 inst_relate_aabb 再写 aabb 内容，SurrealDB 可能会"隐式创建"空的 aabb 记录（d = NONE）。
    // 这里把 inst_relate_aabb 的写入延后到 aabb UPSERT 之后，保证 aabb_id 侧不会出现空记录。

    // inst_tubi_map 不再创建 inst_relate（tubing 使用专门的 tubi_relate 表）
    // world_transform 已提前写入 pe_transform，这里仅收集 aabb 数据用于 tubi_relate
    if !inst_mgr.inst_tubi_map.is_empty() {
        debug_model_debug!(
            "save_instance_data_optimize processing inst_tubi_map: {} Tubing records (不创建 inst_relate)",
            inst_mgr.inst_tubi_map.len()
        );

        for (_key, info) in &inst_mgr.inst_tubi_map {
            if info.world_transform.translation.is_nan()
                || info.world_transform.rotation.is_nan()
                || info.world_transform.scale.is_nan()
            {
                continue;
            }

            // 收集 aabb 数据（用于 tubi_relate）
            if let Some(aabb) = info.aabb {
                let aabb_hash = gen_aabb_hash(&aabb);
                if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                    entry.insert(serde_json::to_string(&aabb)?);
                }
            }
        }
    }

    if !inst_info_buffer.is_empty() {
        let statement = format!(
            "INSERT IGNORE INTO {} [{}];",
            stringify!(inst_info),
            inst_info_buffer.join(",")
        );
        inst_info_batcher.push(statement).await?;
        debug_model_debug!(
            "save_instance_data_optimize flushing remaining inst_info records: {}",
            inst_info_buffer.len()
        );
    }

    // NOTE: 暂时跳过 has_inst 标记更新，后续单独处理以避免阻塞调试

    debug_model_debug!("🔍 [DEBUG] Finishing inst_relate_batcher...");
    inst_relate_batcher.finish().await?;
    debug_model_debug!("✅ [DEBUG] inst_relate_batcher finished successfully");

    debug_model_debug!("🔍 [DEBUG] Finishing inst_info_batcher...");
    inst_info_batcher.finish().await?;
    debug_model_debug!("✅ [DEBUG] inst_info_batcher finished successfully");

    // aabb
    if !aabb_map.is_empty() {
        let mut aabb_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (&hash, value) in &aabb_map {
            json_buffer.push(format!("{{'id':aabb:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                let statement = format!("INSERT IGNORE INTO aabb [{}];", json_buffer.join(","));
                aabb_batcher.push(statement).await?;
                json_buffer.clear();
            }
        }

        if !json_buffer.is_empty() {
            let statement = format!("INSERT IGNORE INTO aabb [{}];", json_buffer.join(","));
            aabb_batcher.push(statement).await?;
        }

        aabb_batcher.finish().await?;
    }

    // inst_relate_aabb（普通表：refno=pe, aabb_id=aabb），按历史约定延后到 aabb 写入之后执行
    if write_inst_relate_aabb
        && (!inst_relate_aabb_chunks.is_empty() || !inst_relate_aabb_buffer.is_empty())
    {
        let mut inst_aabb_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);

        // 统一把积累的 chunks + 剩余 buffer 一次性落库；同一 refno 可能在
        // 多个批次中被重新聚合，写入前按目标 id 去重，避免单个 INSERT 内自冲突。
        let raw_total = inst_relate_aabb_chunks
            .iter()
            .map(|(rows, _)| rows.len())
            .sum::<usize>()
            + inst_relate_aabb_buffer.len();
        let mut all_rows: Vec<String> = Vec::with_capacity(raw_total);
        let mut all_ids: Vec<String> = Vec::with_capacity(raw_total);
        for (rows, ids) in &inst_relate_aabb_chunks {
            all_rows.extend(rows.iter().cloned());
            all_ids.extend(ids.iter().cloned());
        }
        all_rows.extend(inst_relate_aabb_buffer.iter().cloned());
        all_ids.extend(inst_relate_aabb_ids.iter().cloned());
        let (deduped_rows, deduped_ids) = dedupe_inst_relate_aabb_rows(&all_rows, &all_ids);
        let total = deduped_rows.len();

        // 同一 refno 在同一轮生成中可能先后产生不同 canonical inst_info。
        // AABB 必须跟随当前 inst_relate 一起替换，避免保留先到候选的包围盒。
        for (rows, ids) in deduped_rows
            .chunks(CHUNK_SIZE)
            .zip(deduped_ids.chunks(CHUNK_SIZE))
        {
            let statements = build_replace_rows_statements("inst_relate_aabb", false, rows, ids);
            inst_aabb_batcher.push_group(statements).await?;
        }

        debug_model_debug!(
            "save_instance_data_optimize flushing inst_relate_aabb after aabb insert: {}",
            total
        );
        inst_aabb_batcher.finish().await?;
    } else if !write_inst_relate_aabb
        && (!inst_relate_aabb_chunks.is_empty() || !inst_relate_aabb_buffer.is_empty())
    {
        debug_model_debug!(
            "save_instance_data_optimize skip inst_relate_aabb write: {} buffered rows",
            inst_relate_aabb_chunks
                .iter()
                .map(|(rows, _)| rows.len())
                .sum::<usize>()
                + inst_relate_aabb_buffer.len()
        );
    }

    // transform
    if !transform_map.is_empty() {
        let mut transform_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (&hash, value) in &transform_map {
            json_buffer.push(format!("{{'id':trans:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                let statement = format!("INSERT IGNORE INTO trans [{}];", json_buffer.join(","));
                transform_batcher.push(statement).await?;
                json_buffer.clear();
            }
        }

        if !json_buffer.is_empty() {
            let statement = format!("INSERT IGNORE INTO trans [{}];", json_buffer.join(","));
            transform_batcher.push(statement).await?;
        }

        transform_batcher.finish().await?;
    }

    // vec3
    if !vec3_map.is_empty() {
        let mut vec3_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (&hash, value) in &vec3_map {
            json_buffer.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                let statement = format!("INSERT IGNORE INTO vec3 [{}];", json_buffer.join(","));
                vec3_batcher.push(statement).await?;
                json_buffer.clear();
            }
        }

        if !json_buffer.is_empty() {
            let statement = format!("INSERT IGNORE INTO vec3 [{}];", json_buffer.join(","));
            vec3_batcher.push(statement).await?;
        }

        vec3_batcher.finish().await?;
    }

    debug_model_debug!(
        "save_instance_data_optimize finish: inst_info={}, inst_geo={}, tubi={}, neg={}, ngmr={}",
        inst_mgr.inst_info_map.len(),
        inst_mgr.inst_geos_map.len(),
        inst_mgr.inst_tubi_map.len(),
        inst_mgr.neg_relate_map.len(),
        inst_mgr.ngmr_neg_relate_map.len()
    );

    // 聚合数据到 refno_relations 表（极简方案）
    if replace_exist {
        use crate::fast_model::gen_model::pdms_inst_surreal::{
            RefnoRelations, save_refno_relations_surreal,
        };
        use std::collections::HashMap;

        let mut relations_map: HashMap<RefnoEnum, RefnoRelations> = HashMap::new();

        // 聚合 inst_info
        for (refno, info) in &inst_mgr.inst_info_map {
            let dbnum = *inst_dbnum_map.get(refno).unwrap_or(&0);
            let rel = relations_map
                .entry(*refno)
                .or_insert_with(|| RefnoRelations {
                    refno: *refno,
                    dbnum,
                    ..Default::default()
                });
            rel.inst_keys.push(info.get_inst_key());
        }

        // 聚合 inst_geos
        for inst_geo_data in inst_mgr.inst_geos_map.values() {
            for inst in &inst_geo_data.insts {
                if let Some(rel) = relations_map.get_mut(&inst.refno) {
                    rel.geo_hashes.push(inst.geo_hash);
                }
            }
        }

        // 批量保存
        let relations: Vec<_> = relations_map.into_values().collect();
        if !relations.is_empty() {
            save_refno_relations_surreal(&relations).await?;
        }
    }

    report.missing_neg_carriers.sort_unstable();
    report.missing_neg_carriers.dedup();

    Ok(report)
}

fn merge_aabb_slot(slot: &mut Option<Aabb>, next: Aabb) {
    *slot = Some(match slot.take() {
        Some(existing) => existing.merged(&next),
        None => next,
    });
}

fn is_valid_aabb(aabb: &Aabb) -> bool {
    let ext = aabb.extents().magnitude();
    !ext.is_nan() && !ext.is_infinite()
}

fn extrusion_key_points_aabb(
    extrusion: &aios_core::prim_geo::Extrusion,
    world_t: Transform,
) -> Option<Aabb> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut min_z0 = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut max_z0 = f32::NEG_INFINITY;

    for poly in &extrusion.verts {
        for v in poly {
            min_x = min_x.min(v.x);
            min_y = min_y.min(v.y);
            min_z0 = min_z0.min(v.z);
            max_x = max_x.max(v.x);
            max_y = max_y.max(v.y);
            max_z0 = max_z0.max(v.z);
        }
    }

    if !min_x.is_finite() || !min_y.is_finite() || !min_z0.is_finite() {
        return None;
    }

    let z_candidates = [
        min_z0,
        max_z0,
        min_z0 + extrusion.height,
        max_z0 + extrusion.height,
    ];
    let min_z = z_candidates.iter().copied().fold(f32::INFINITY, f32::min);
    let max_z = z_candidates
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);

    let corners = [
        glam::Vec3::new(min_x, min_y, min_z),
        glam::Vec3::new(min_x, min_y, max_z),
        glam::Vec3::new(min_x, max_y, min_z),
        glam::Vec3::new(min_x, max_y, max_z),
        glam::Vec3::new(max_x, min_y, min_z),
        glam::Vec3::new(max_x, min_y, max_z),
        glam::Vec3::new(max_x, max_y, min_z),
        glam::Vec3::new(max_x, max_y, max_z),
    ];

    let mut aabb = Aabb::new_invalid();
    for corner in corners {
        let wp = world_t.transform_point(corner);
        aabb.take_point(Point::new(wp.x, wp.y, wp.z));
    }

    is_valid_aabb(&aabb).then_some(aabb)
}

fn derive_inst_world_aabb_from_param(info: &EleGeosInfo, inst: &EleInstGeo) -> Option<Aabb> {
    let world_t = info.get_geo_world_transform(inst);

    if let Some(local_aabb) = inst.aabb {
        let world_aabb = aabb_apply_transform(&local_aabb, &world_t);
        return is_valid_aabb(&world_aabb).then_some(world_aabb);
    }

    let points = inst.geo_param.key_points();
    if points.is_empty() {
        return match &inst.geo_param {
            PdmsGeoParam::PrimExtrusion(extrusion) => extrusion_key_points_aabb(extrusion, world_t),
            _ => None,
        };
    }

    let mut aabb = Aabb::new_invalid();
    for point in points {
        let wp = world_t.transform_point(point.0);
        aabb.take_point(Point::new(wp.x, wp.y, wp.z));
    }

    is_valid_aabb(&aabb).then_some(aabb)
}

fn resolve_inst_world_aabb_for_index(
    info: &EleGeosInfo,
    inst: &EleInstGeo,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
) -> Option<Aabb> {
    if let Some(mr) = mesh_results.get(&inst.geo_hash) {
        if let Some(h) = mr.aabb_hash {
            if let Some(local_aabb) = mesh_aabb_map.get(&h.to_string()) {
                let world_t = info.get_geo_world_transform(inst);
                let world_aabb = aabb_apply_transform(&local_aabb, &world_t);
                return is_valid_aabb(&world_aabb).then_some(world_aabb);
            }
        }
    }

    if let Some(local_aabb) = crate::fast_model::EXIST_MESH_GEO_HASHES
        .get(&inst.geo_hash.to_string())
        .map(|aabb| *aabb)
    {
        let world_t = info.get_geo_world_transform(inst);
        let world_aabb = aabb_apply_transform(&local_aabb, &world_t);
        return is_valid_aabb(&world_aabb).then_some(world_aabb);
    }

    derive_inst_world_aabb_from_param(info, inst)
}

fn resolve_element_world_aabb_for_index(
    info: &EleGeosInfo,
    geos_info: Option<&EleInstGeosData>,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
) -> Option<Aabb> {
    if let Some(geos_info) = geos_info {
        let mut union_aabb: Option<Aabb> = None;
        for inst in &geos_info.insts {
            if let Some(world_aabb) =
                resolve_inst_world_aabb_for_index(info, inst, mesh_results, mesh_aabb_map)
            {
                merge_aabb_slot(&mut union_aabb, world_aabb);
            }
        }
        if union_aabb.is_some() {
            return union_aabb;
        }
        if let Some(aabb) = geos_info.aabb {
            return is_valid_aabb(&aabb).then_some(aabb);
        }
    }

    info.aabb.filter(is_valid_aabb)
}

pub fn build_inst_relate_aabb_rows(
    inst_mgr: &ShapeInstancesData,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
) -> anyhow::Result<(HashMap<u64, String>, Vec<String>, Vec<String>)> {
    let mut aabb_map: HashMap<u64, String> = HashMap::new();
    let mut inst_relate_aabb_rows: Vec<String> = Vec::new();
    let mut inst_relate_aabb_ids: Vec<String> = Vec::new();

    for (key, info) in &inst_mgr.inst_info_map {
        let resolved_aabb: Option<(u64, Aabb)> = resolve_element_world_aabb_for_index(
            info,
            inst_mgr.inst_geos_map.get(&info.get_inst_key()),
            mesh_results,
            mesh_aabb_map,
        )
        .map(|aabb| (gen_aabb_hash(&aabb), aabb));

        if let Some((aabb_hash, aabb)) = resolved_aabb {
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }

            inst_relate_aabb_rows.push(format!(
                "{{id: {0}, refno: {1}, aabb: aabb:⟨{2}⟩, aabb_id: aabb:⟨{2}⟩}}",
                model_refno_id("inst_relate_aabb", *key),
                key.to_pe_key(),
                aabb_hash
            ));
            inst_relate_aabb_ids.push(model_refno_id("inst_relate_aabb", *key));
        }
    }

    let (inst_relate_aabb_rows, inst_relate_aabb_ids) =
        dedupe_inst_relate_aabb_rows(&inst_relate_aabb_rows, &inst_relate_aabb_ids);

    Ok((aabb_map, inst_relate_aabb_rows, inst_relate_aabb_ids))
}

fn dedupe_inst_relate_aabb_rows(rows: &[String], ids: &[String]) -> (Vec<String>, Vec<String>) {
    dedupe_rows_by_id(rows, ids)
}

fn dedupe_rows_by_id(rows: &[String], ids: &[String]) -> (Vec<String>, Vec<String>) {
    debug_assert_eq!(rows.len(), ids.len());

    let mut index_by_id: HashMap<&str, usize> = HashMap::with_capacity(ids.len());
    let mut deduped_rows: Vec<String> = Vec::with_capacity(rows.len());
    let mut deduped_ids: Vec<String> = Vec::with_capacity(ids.len());

    for (row, id) in rows.iter().zip(ids.iter()) {
        if let Some(&idx) = index_by_id.get(id.as_str()) {
            deduped_rows[idx] = row.clone();
            deduped_ids[idx] = id.clone();
        } else {
            index_by_id.insert(id.as_str(), deduped_rows.len());
            deduped_rows.push(row.clone());
            deduped_ids.push(id.clone());
        }
    }

    (deduped_rows, deduped_ids)
}

fn build_replace_rows_statements(
    table: &str,
    relation: bool,
    rows: &[String],
    ids: &[String],
) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    debug_assert_eq!(rows.len(), ids.len());

    let (deduped_rows, deduped_ids) = dedupe_rows_by_id(rows, ids);
    if deduped_rows.is_empty() {
        return Vec::new();
    }

    let insert_keyword = if relation {
        "INSERT RELATION"
    } else {
        "INSERT"
    };
    vec![
        format!("DELETE [{}];", deduped_ids.join(",")),
        format!(
            "{insert_keyword} INTO {table} [{}];",
            deduped_rows.join(",")
        ),
    ]
}

pub async fn save_inst_relate_aabb_rows(
    aabb_map: &HashMap<u64, String>,
    inst_relate_aabb_rows: &[String],
    inst_relate_aabb_ids: &[String],
) -> anyhow::Result<()> {
    if aabb_map.is_empty() && inst_relate_aabb_rows.is_empty() {
        return Ok(());
    }
    anyhow::ensure!(
        inst_relate_aabb_rows.len() == inst_relate_aabb_ids.len(),
        "inst_relate_aabb rows/ids 数量不一致: rows={}, ids={}",
        inst_relate_aabb_rows.len(),
        inst_relate_aabb_ids.len()
    );

    const CHUNK_SIZE: usize = 100;
    const MAX_TX_STATEMENTS: usize = 4;
    const MAX_CONCURRENT_TX: usize = 2;

    if !aabb_map.is_empty() {
        let mut aabb_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (&hash, value) in aabb_map {
            json_buffer.push(format!("{{'id':aabb:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                let statement = format!("INSERT IGNORE INTO aabb [{}];", json_buffer.join(","));
                aabb_batcher.push(statement).await?;
                json_buffer.clear();
            }
        }

        if !json_buffer.is_empty() {
            let statement = format!("INSERT IGNORE INTO aabb [{}];", json_buffer.join(","));
            aabb_batcher.push(statement).await?;
        }

        aabb_batcher.finish().await?;
    }

    if !inst_relate_aabb_rows.is_empty() {
        let mut inst_aabb_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let (deduped_rows, deduped_ids) =
            dedupe_inst_relate_aabb_rows(inst_relate_aabb_rows, inst_relate_aabb_ids);
        for (rows, ids) in deduped_rows
            .chunks(CHUNK_SIZE)
            .zip(deduped_ids.chunks(CHUNK_SIZE))
        {
            let statements = build_replace_rows_statements("inst_relate_aabb", false, rows, ids);
            inst_aabb_batcher.push_group(statements).await?;
        }
        inst_aabb_batcher.finish().await?;
    }

    Ok(())
}

struct TransactionBatcher {
    max_statements: usize,
    max_concurrent: usize,
    pending: Vec<String>,
    tasks: FuturesUnordered<JoinHandle<anyhow::Result<()>>>,
}

impl TransactionBatcher {
    fn new(max_statements: usize, max_concurrent: usize) -> Self {
        let max_statements = max_statements.max(1);
        let max_concurrent = max_concurrent.max(1);
        Self {
            max_statements,
            max_concurrent,
            pending: Vec::with_capacity(max_statements),
            tasks: FuturesUnordered::new(),
        }
    }

    async fn push(&mut self, statement: String) -> anyhow::Result<()> {
        if statement.trim().is_empty() {
            return Ok(());
        }

        self.pending.push(statement);
        if self.pending.len() >= self.max_statements {
            self.flush().await?;
        }
        Ok(())
    }

    async fn push_group(&mut self, statements: Vec<String>) -> anyhow::Result<()> {
        let statements = statements
            .into_iter()
            .filter(|statement| !statement.trim().is_empty())
            .collect::<Vec<_>>();
        if statements.is_empty() {
            return Ok(());
        }

        if self.pending.len() + statements.len() > self.max_statements {
            self.flush().await?;
        }

        self.pending.extend(statements);
        if self.pending.len() >= self.max_statements {
            self.flush().await?;
        }
        Ok(())
    }

    async fn flush(&mut self) -> anyhow::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }

        let statements = std::mem::take(&mut self.pending);
        let statements_len = statements.len();
        let query = build_transaction_block(&statements);
        let debug_query = query.clone();

        self.tasks.push(tokio::spawn(async move {
            macro_rules! take_all_results_or_err {
                ($resp:ident) => {{
                    // surrealdb::Response 可能在某些语句失败时仍然返回 Ok(resp)，错误会延迟到 take() 时才暴露；
                    // 这里对每个 statement 做一次 take 以确保事务块里的错误不会被吞掉。
                    let mut errors: Vec<(usize, String)> = Vec::new();
                    for idx in 0..(statements_len + 2) {
                        match $resp.take::<surrealdb::types::Value>(idx) {
                            Ok(_) => {}
                            Err(e) => errors.push((idx, e.to_string())),
                        }
                    }
                    if errors.is_empty() {
                        Ok(())
                    } else {
                        let mut msg = String::new();
                        for (idx, e) in &errors {
                            msg.push_str(&format!("[{}] {}\n", idx, e));
                        }
                        Err(anyhow::anyhow!("transaction block statement errors:\n{msg}"))
                    }
                }};
            }

            fn is_tx_conflict(msg: &str) -> bool {
                msg.contains("Transaction conflict")
                    || msg.contains("Resource busy")
                    || msg.contains("This transaction can be retried")
            }

            // 注意：不要对 project_primary_db() 做 clone 再 query。
            // 在当前 surrealdb client 实现中，clone 后可能丢失已选定的 namespace/database，
            // 从而随机触发 “Specify a namespace to use” 并导致整块事务回滚。
            //
            // 同时：SurrealDB 在高并发事务下可能返回 “Transaction conflict: Resource busy”，
            // 官方提示该事务可重试。这里对整块事务做有限次重试 + 退避，尽量避免“部分批次直接丢数据”。
            let mut repaired_inst_relate_aabb_index = false;
            let mut repaired_neg_relate_index = false;
            let mut attempt: usize = 0;
            let max_retries: usize = 8;

            loop {
                attempt += 1;

                let run_once = async {
                    match project_primary_db().query_response(&query).await {
                        Ok(mut resp) => take_all_results_or_err!(resp),
                        Err(err) => Err(err),
                    }
                }
                .await;

                match run_once {
                    Ok(()) => {
                        return Ok(());
                    }
                    Err(e) => {
                        let es = e.to_string();

                        // 某些情况下 inst_relate_aabb 的唯一索引可能“脏”了（表里查不到记录但索引仍占用值），
                        // 这会导致所有 INSERT 失败并连带回滚同一事务块（inst_relate 也写不进去）。
                        let is_inst_relate_aabb_unique_conflict = es.contains("idx_inst_relate_aabb_refno")
                            && es.contains("already contains");
                        let is_neg_relate_unique_conflict =
                            es.contains("unique_neg_relate") && es.contains("already contains");

                        if is_inst_relate_aabb_unique_conflict && !repaired_inst_relate_aabb_index {
                            repaired_inst_relate_aabb_index = true;
                            debug_model_debug!(
                                "⚠️ [DEBUG] 检测到 inst_relate_aabb 唯一索引冲突，尝试重建索引并重试..."
                            );
                            let repair_sql = "REMOVE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb; \
DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb FIELDS refno;";
                            let _ = project_primary_db().query_response(repair_sql).await;
                            continue;
                        }

                        if is_neg_relate_unique_conflict && !repaired_neg_relate_index {
                            repaired_neg_relate_index = true;
                            debug_model_debug!(
                                "⚠️ [DEBUG] 检测到 neg_relate 唯一索引冲突，尝试重建索引并重试..."
                            );
                            let repair_sql = "REMOVE INDEX unique_neg_relate ON TABLE neg_relate; \
DEFINE INDEX unique_neg_relate ON TABLE neg_relate COLUMNS in, out UNIQUE;";
                            let _ = project_primary_db().query_response(repair_sql).await;
                            continue;
                        }

                        let conflict = is_tx_conflict(&es);
                        if conflict && attempt < max_retries {
                            // 50ms,100ms,200ms,... up to 2s
                            let backoff_ms = (50u64.saturating_mul(1u64 << (attempt - 1))).min(2000);
                            debug_model_debug!(
                                "⚠️ [DEBUG] Transaction conflict, retry {}/{} after {}ms",
                                attempt,
                                max_retries,
                                backoff_ms
                            );
                            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                            continue;
                        }

                        debug_model_debug!(
                            "❌ [DEBUG] TransactionBatcher failed: {}\n--- transaction block ---\n{}",
                            e,
                            debug_query
                        );
                        match dump_failed_sql_batch(&debug_query, &es, attempt, max_retries) {
                            Ok(Some(file_path)) => {
                                eprintln!(
                                    "❌ 写入失败超出重试限制，导致失败的 SQL 块已转储至 {}",
                                    file_path.display()
                                );
                            }
                            Ok(None) => {
                                eprintln!(
                                    "❌ 写入失败超出重试限制，但 failed_sql 转储已达到单次运行上限({})，后续仅保留错误输出",
                                    MAX_FAILED_SQL_DUMPS_PER_RUN
                                );
                            }
                            Err(write_err) => {
                                eprintln!("写入失败 SQL 诊断文件时出错: {}", write_err);
                            }
                        }

                        return Err(e);
                    }
                }
            }
        }));

        self.await_if_needed().await
    }

    async fn await_if_needed(&mut self) -> anyhow::Result<()> {
        while self.tasks.len() >= self.max_concurrent {
            if let Some(result) = self.tasks.next().await {
                match result {
                    Ok(inner) => inner?,
                    Err(join_err) => return Err(join_err.into()),
                }
            }
        }
        Ok(())
    }

    async fn finish(mut self) -> anyhow::Result<()> {
        if !self.pending.is_empty() {
            self.flush().await?;
        }

        while let Some(result) = self.tasks.next().await {
            match result {
                Ok(inner) => inner?,
                Err(join_err) => return Err(join_err.into()),
            }
        }

        Ok(())
    }
}

fn build_transaction_block(statements: &[String]) -> String {
    let estimated_len = statements.iter().map(|s| s.len() + 2).sum::<usize>() + 32;
    let mut block = String::with_capacity(estimated_len);
    block.push_str("BEGIN TRANSACTION;\n");
    for stmt in statements {
        let trimmed = stmt.trim_end();
        block.push_str(trimmed);
        if !trimmed.ends_with(';') {
            block.push(';');
        }
        block.push('\n');
    }
    block.push_str("COMMIT TRANSACTION;");
    block
}

/// 批量保存 tubi_info 数据到数据库（INSERT IGNORE 幂等）。
///
/// spec 005（grill-me Q3）：不再预查已存在 id——IGNORE 本身就是幂等闸，
/// 预查在部署场景是纯白跑的批量查询。返回值为提交条数（非精确新增数，
/// 仅 debug 日志消费）。
///
/// # 参数
/// - `tubi_info_map`: 组合键 ID -> TubiInfoData 的映射
///
/// # 返回
/// - `Ok(usize)`: 提交的记录数量
pub async fn save_tubi_info_batch(
    tubi_info_map: &DashMap<String, TubiInfoData>,
) -> anyhow::Result<usize> {
    if tubi_info_map.is_empty() {
        return Ok(0);
    }

    const CHUNK_SIZE: usize = 200;

    let entries: Vec<_> = tubi_info_map.iter().collect();
    debug_model_debug!("save_tubi_info_batch: total={}", entries.len());

    let mut submitted = 0;
    for chunk in entries.chunks(CHUNK_SIZE) {
        let values: Vec<String> = chunk.iter().map(|e| e.value().to_surreal_json()).collect();

        let sql = format!("INSERT IGNORE INTO tubi_info [{}];", values.join(","));
        project_primary_db().query_response(&sql).await?.check()?;
        submitted += chunk.len();

        debug_model_debug!(
            "save_tubi_info_batch: submitted chunk of {} records",
            chunk.len()
        );
    }

    Ok(submitted)
}

/// 仅使用本次 run 汇总的内存事实补写跨 batch 负关系。
///
/// `geo_relate` ID 与逐批 writer 使用相同的内容寻址规则，因此这里无需查询
/// SurrealDB；`INSERT RELATION IGNORE` 只负责幂等持久化。
pub async fn persist_negative_relations_from_artifacts(
    artifacts: &ShapeInstancesData,
) -> anyhow::Result<usize> {
    use aios_core::geometry::GeoBasicType;

    let inst_key_carriers = build_inst_key_carrier_map(artifacts);
    let mut neg_geo_by_carrier: HashMap<RefnoEnum, Vec<(usize, String)>> = HashMap::new();
    let mut cata_cross_neg_geo_map: HashMap<(RefnoEnum, RefnoEnum), Vec<(usize, String)>> =
        HashMap::new();

    for inst_geo_data in artifacts.inst_geos_map.values() {
        let carriers = inst_key_carriers
            .get(&inst_geo_data.id())
            .cloned()
            .unwrap_or_else(|| vec![(inst_geo_data.refno, inst_geo_data.id())]);
        for (geo_index, inst) in inst_geo_data.insts.iter().enumerate() {
            for (carrier_refno, inst_info_id) in &carriers {
                let relation_id =
                    geo_relate_id_for_inst(*carrier_refno, geo_index, inst_info_id.as_str());
                match inst.geo_type {
                    GeoBasicType::Neg => neg_geo_by_carrier
                        .entry(*carrier_refno)
                        .or_default()
                        .push((geo_index, relation_id)),
                    GeoBasicType::CataCrossNeg => cata_cross_neg_geo_map
                        .entry((*carrier_refno, inst.refno))
                        .or_default()
                        .push((geo_index, relation_id)),
                    _ => {}
                }
            }
        }
    }
    for rows in neg_geo_by_carrier.values_mut() {
        rows.sort_unstable();
        rows.dedup();
    }
    for rows in cata_cross_neg_geo_map.values_mut() {
        rows.sort_unstable();
        rows.dedup();
    }

    let mut relation_records = Vec::new();
    let mut neg_targets = artifacts.neg_relate_map.iter().collect::<Vec<_>>();
    neg_targets.sort_unstable_by_key(|(target, _)| **target);
    for (target, carriers) in neg_targets {
        let mut carriers = carriers.clone();
        carriers.sort_unstable();
        carriers.dedup();
        for carrier in carriers {
            if let Some(geometries) = neg_geo_by_carrier.get(&carrier) {
                for (geo_index, geo_relate_id) in geometries {
                    relation_records.push((
                        "neg_relate",
                        format!(
                            "{{ in: {geo_relate_id}, id: {}, out: {}, pe: {} }}",
                            neg_relate_id(*target, carrier, *geo_index, 0),
                            target.to_pe_key(),
                            carrier.to_pe_key(),
                        ),
                    ));
                }
            }
        }
    }

    let mut ngmr_targets = artifacts.ngmr_neg_relate_map.iter().collect::<Vec<_>>();
    ngmr_targets.sort_unstable_by_key(|(target, _)| **target);
    for (target, pairs) in ngmr_targets {
        let mut pairs = pairs.clone();
        pairs.sort_unstable();
        pairs.dedup();
        for (carrier, ngmr_geom_refno) in pairs {
            if let Some(geometries) = cata_cross_neg_geo_map.get(&(carrier, ngmr_geom_refno)) {
                for (geo_index, geo_relate_id) in geometries {
                    relation_records.push((
                        "ngmr_relate",
                        format!(
                            "{{ in: {geo_relate_id}, id: {}, out: {}, pe: {}, ngmr: {} }}",
                            ngmr_relate_id(*target, carrier, *geo_index, 0),
                            target.to_pe_key(),
                            carrier.to_pe_key(),
                            ngmr_geom_refno.to_pe_key(),
                        ),
                    ));
                }
            }
        }
    }

    const INSERT_CHUNK_SIZE: usize = 200;
    let mut submitted = 0usize;
    for table in ["neg_relate", "ngmr_relate"] {
        let records = relation_records
            .iter()
            .filter_map(|(record_table, record)| (*record_table == table).then_some(record))
            .collect::<Vec<_>>();
        for chunk in records.chunks(INSERT_CHUNK_SIZE) {
            let sql = format!(
                "INSERT RELATION IGNORE INTO {table} [{}];",
                chunk.iter().map(|record| record.as_str()).join(",")
            );
            project_primary_db().query_response(&sql).await?.check()?;
            submitted += chunk.len();
        }
    }
    Ok(submitted)
}

/// 补建跨阶段缺失的 neg_relate
///
/// 当 LOOP 阶段的 LoopOwner（如 GWALL）发现负实体子孙（如 NPYR）时，会在
/// `neg_relate_map` 中记录关系。但负实体的 Neg 类型 `geo_relate` 要到 PRIM 阶段
/// 才创建，导致 `save_instance_data_optimize` 中 `neg_geo_by_carrier` 找不到
/// 对应条目，`neg_relate` 未实际写入。
///
/// 此函数在所有阶段（LOOP/CATE/PRIM）完成后、布尔运算前调用，
/// 从 DB 查询已有的 Neg/CataNeg geo_relate 并补建缺失的 neg_relate。
pub async fn reconcile_missing_neg_relate(
    all_refnos: &[RefnoEnum],
    candidate_carriers: &[RefnoEnum],
) -> anyhow::Result<usize> {
    if all_refnos.is_empty() {
        return Ok(0);
    }
    if candidate_carriers.is_empty() {
        println!(
            "[reconcile] skip missing neg reconcile: all_refnos={} candidate_carriers=0",
            all_refnos.len()
        );
        return Ok(0);
    }

    let reconcile_started = std::time::Instant::now();
    let refno_set: HashSet<RefnoEnum> = all_refnos.iter().copied().collect();
    let debug_filters: HashSet<String> = std::env::var("AIOS_DEBUG_NEG_RECONCILE")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|item| {
                    item.trim()
                        .trim_matches('`')
                        .trim_matches('⟨')
                        .trim_matches('⟩')
                })
                .filter_map(|item| {
                    let normalized = item
                        .strip_prefix("pe:")
                        .or_else(|| item.strip_prefix("pe:⟨"))
                        .unwrap_or(item)
                        .trim_matches('`')
                        .trim_matches('⟨')
                        .trim_matches('⟩')
                        .trim()
                        .to_string();
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let should_debug_reconcile = |carrier: &str, parent: Option<&str>| -> bool {
        !debug_filters.is_empty()
            && (debug_filters.contains(carrier)
                || parent.is_some_and(|pid| debug_filters.contains(pid)))
    };
    let candidate_carriers = candidate_carriers
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    // 嵌入式 SurrealDB 在超长 IN 列表 / relation 遍历上容易出现长时间静默等待。
    // 这里保守降低 chunk，并补进度日志，便于定位到底卡在 query/check/insert 哪一段。
    const QUERY_CHUNK_SIZE: usize = 200;
    const CHECK_CHUNK_SIZE: usize = 200;
    const INSERT_CHUNK_SIZE: usize = 200;

    struct NegGeoInfo {
        gr_id: String,
        neg_carrier: String,
        parent_id: String,
    }
    struct NegGeoCandidate {
        gr_id: String,
        neg_carrier: String,
    }
    let mut candidates: Vec<NegGeoCandidate> = Vec::new();

    // 1. 分块查询当前 batch 中所有 Neg/CataNeg 类型 geo_relate，避免超长 IN 列表拖垮 Surreal 解析。
    //    这里先只取 gr_id + neg_carrier，避免在“零命中 chunk”上提前求值 geom_refno.owner。
    let query_start = std::time::Instant::now();
    let total_query_chunks = candidate_carriers.len().div_ceil(QUERY_CHUNK_SIZE);
    println!(
        "[reconcile] start all_refnos={} candidate_carriers={} query_chunk_size={} check_chunk_size={} insert_chunk_size={}",
        all_refnos.len(),
        candidate_carriers.len(),
        QUERY_CHUNK_SIZE,
        CHECK_CHUNK_SIZE,
        INSERT_CHUNK_SIZE
    );
    for (chunk_idx, refno_chunk) in candidate_carriers.chunks(QUERY_CHUNK_SIZE).enumerate() {
        let pe_list = refno_chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            r#"SELECT
                id as gr_id,
                record::id(geom_refno) as neg_carrier
            FROM geo_relate
            WHERE geo_type IN ['Neg', 'CataNeg']
              AND geom_refno IN [{pe_list}]"#
        );
        let mut response = project_primary_db().query_response(&sql).await?;
        let neg_geos: Vec<serde_json::Value> = response.take(0)?;
        for val in &neg_geos {
            let gr_id = val
                .get("gr_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let neg_carrier = val
                .get("neg_carrier")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if gr_id.is_empty() || neg_carrier.is_empty() {
                continue;
            }
            if should_debug_reconcile(&neg_carrier, None) {
                println!(
                    "[reconcile-debug] candidate carrier={} gr_id={}",
                    neg_carrier, gr_id
                );
            }
            candidates.push(NegGeoCandidate { gr_id, neg_carrier });
        }
        if chunk_idx == 0 || (chunk_idx + 1) % 50 == 0 || chunk_idx + 1 == total_query_chunks {
            println!(
                "[reconcile] query chunk {}/{} candidates_so_far={} elapsed_ms={}",
                chunk_idx + 1,
                total_query_chunks,
                candidates.len(),
                query_start.elapsed().as_millis()
            );
        }
    }
    if candidates.is_empty() {
        println!(
            "[reconcile] no neg geo candidates found query_ms={} total_ms={}",
            query_start.elapsed().as_millis(),
            reconcile_started.elapsed().as_millis()
        );
        return Ok(0);
    }

    // 2. 仅对已命中的 neg carrier 点查 owner，避免在全量扫描阶段做昂贵表达式求值。
    let parent_lookup_start = std::time::Instant::now();
    let unique_carriers: Vec<String> = candidates
        .iter()
        .map(|info| format!("pe:⟨{}⟩", info.neg_carrier))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let total_parent_chunks = unique_carriers.len().div_ceil(CHECK_CHUNK_SIZE);
    let mut parent_by_carrier: HashMap<String, String> = HashMap::new();
    for (chunk_idx, carrier_chunk) in unique_carriers.chunks(CHECK_CHUNK_SIZE).enumerate() {
        let sql = format!(
            "SELECT record::id(id) as carrier_id, record::id(owner) as parent_id FROM [{}];",
            carrier_chunk.join(",")
        );
        let mut response = project_primary_db().query_response(&sql).await?;
        let rows: Vec<serde_json::Value> = response.take(0).unwrap_or_default();
        for row in rows {
            let carrier_id = row
                .get("carrier_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let parent_id = row
                .get("parent_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if carrier_id.is_empty() || parent_id.is_empty() {
                if should_debug_reconcile(&carrier_id, Some(&parent_id)) {
                    println!(
                        "[reconcile-debug] parent-miss carrier={} parent_id='{}'",
                        carrier_id, parent_id
                    );
                }
                continue;
            }
            if should_debug_reconcile(&carrier_id, Some(&parent_id)) {
                println!(
                    "[reconcile-debug] parent-hit carrier={} parent={}",
                    carrier_id, parent_id
                );
            }
            parent_by_carrier.insert(carrier_id, parent_id);
        }
        if chunk_idx == 0 || (chunk_idx + 1) % 50 == 0 || chunk_idx + 1 == total_parent_chunks {
            println!(
                "[reconcile] parent-lookup chunk {}/{} resolved_so_far={} elapsed_ms={}",
                chunk_idx + 1,
                total_parent_chunks,
                parent_by_carrier.len(),
                parent_lookup_start.elapsed().as_millis()
            );
        }
    }

    let mut infos: Vec<NegGeoInfo> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let carrier_id = format!("pe:⟨{}⟩", candidate.neg_carrier);
        let Some(parent_id) = parent_by_carrier.get(&carrier_id).cloned() else {
            if should_debug_reconcile(&candidate.neg_carrier, None) {
                println!(
                    "[reconcile-debug] candidate-without-parent carrier={} gr_id={}",
                    candidate.neg_carrier, candidate.gr_id
                );
            }
            continue;
        };
        if should_debug_reconcile(&candidate.neg_carrier, Some(&parent_id)) {
            println!(
                "[reconcile-debug] resolved carrier={} parent={} gr_id={}",
                candidate.neg_carrier, parent_id, candidate.gr_id
            );
        }
        infos.push(NegGeoInfo {
            gr_id: candidate.gr_id,
            neg_carrier: candidate.neg_carrier,
            parent_id,
        });
    }
    if infos.is_empty() {
        println!(
            "[reconcile] no carrier parent found candidates={} query_ms={} parent_lookup_ms={} total_ms={}",
            parent_by_carrier.len(),
            query_start.elapsed().as_millis(),
            parent_lookup_start.elapsed().as_millis(),
            reconcile_started.elapsed().as_millis()
        );
        return Ok(0);
    }

    // 3. 分块检查已存在的 neg_relate，避免超长 geo_relate id 列表
    let existing_check_start = std::time::Instant::now();
    let mut existing: HashSet<String> = HashSet::new();
    let total_check_chunks = infos.len().div_ceil(CHECK_CHUNK_SIZE);
    for (chunk_idx, info_chunk) in infos.chunks(CHECK_CHUNK_SIZE).enumerate() {
        let gr_id_list = info_chunk
            .iter()
            .map(|r| r.gr_id.clone())
            .collect::<Vec<_>>()
            .join(",");
        let check_sql = format!("SELECT VALUE record::id(in) FROM [{gr_id_list}]->neg_relate");
        let mut check_resp = project_primary_db().query_response(&check_sql).await?;
        let existing_vec: Vec<String> = check_resp.take(0).unwrap_or_default();
        existing.extend(existing_vec);
        if chunk_idx == 0 || (chunk_idx + 1) % 50 == 0 || chunk_idx + 1 == total_check_chunks {
            println!(
                "[reconcile] existing-check chunk {}/{} existing_so_far={} elapsed_ms={}",
                chunk_idx + 1,
                total_check_chunks,
                existing.len(),
                existing_check_start.elapsed().as_millis()
            );
        }
    }

    // 4. 创建缺失的 neg_relate
    let mut neg_buffer: Vec<String> = Vec::new();
    for info in &infos {
        if existing.contains(&info.gr_id) {
            if should_debug_reconcile(&info.neg_carrier, Some(&info.parent_id)) {
                println!(
                    "[reconcile-debug] skip-existing carrier={} parent={} gr_id={}",
                    info.neg_carrier, info.parent_id, info.gr_id
                );
            }
            continue;
        }
        // parent 必须在当前 batch 中（确保只补建本次生成范围内的关系）
        let target: RefnoEnum = match info.parent_id.parse() {
            Ok(r) => r,
            Err(_) => {
                if should_debug_reconcile(&info.neg_carrier, Some(&info.parent_id)) {
                    println!(
                        "[reconcile-debug] skip-parent-parse carrier={} parent={} gr_id={}",
                        info.neg_carrier, info.parent_id, info.gr_id
                    );
                }
                continue;
            }
        };
        if !refno_set.contains(&target) {
            if should_debug_reconcile(&info.neg_carrier, Some(&info.parent_id)) {
                println!(
                    "[reconcile-debug] skip-parent-out-of-batch carrier={} parent={} gr_id={}",
                    info.neg_carrier, info.parent_id, info.gr_id
                );
            }
            continue;
        }

        let target_refno = target;
        let carrier_refno: RefnoEnum = match info.neg_carrier.parse() {
            Ok(r) => r,
            Err(_) => continue,
        };
        let relation_index = neg_buffer.len();
        let neg_id = neg_relate_id(target_refno, carrier_refno, relation_index, 0);
        neg_buffer.push(format!(
            "{{ in: {0}, id: {3}, out: pe:⟨{2}⟩, pe: pe:⟨{1}⟩ }}",
            info.gr_id, info.neg_carrier, info.parent_id, neg_id,
        ));
        if should_debug_reconcile(&info.neg_carrier, Some(&info.parent_id)) {
            println!(
                "[reconcile-debug] enqueue-insert carrier={} parent={} gr_id={}",
                info.neg_carrier, info.parent_id, info.gr_id
            );
        }
    }

    let created = neg_buffer.len();
    if neg_buffer.is_empty() {
        println!(
            "[reconcile] no missing neg_relate to insert infos={} existing={} query_ms={} parent_lookup_ms={} existing_check_ms={} total_ms={}",
            infos.len(),
            existing.len(),
            query_start.elapsed().as_millis(),
            parent_lookup_start.elapsed().as_millis(),
            existing_check_start.elapsed().as_millis(),
            reconcile_started.elapsed().as_millis()
        );
    } else {
        let insert_start = std::time::Instant::now();
        let total_insert_chunks = neg_buffer.len().div_ceil(INSERT_CHUNK_SIZE);
        for (chunk_idx, relation_chunk) in neg_buffer.chunks(INSERT_CHUNK_SIZE).enumerate() {
            let sql = format!(
                "INSERT RELATION IGNORE INTO neg_relate [{}];",
                relation_chunk.join(",")
            );
            project_primary_db().query_response(&sql).await?.check()?;
            if chunk_idx == 0 || (chunk_idx + 1) % 50 == 0 || chunk_idx + 1 == total_insert_chunks {
                println!(
                    "[reconcile] insert chunk {}/{} created_so_far={} elapsed_ms={}",
                    chunk_idx + 1,
                    total_insert_chunks,
                    ((chunk_idx + 1) * INSERT_CHUNK_SIZE).min(created),
                    insert_start.elapsed().as_millis()
                );
            }
        }
        println!(
            "[reconcile] 补建 {} 条 neg_relate（跨阶段负实体关系） infos={} existing={} query_ms={} parent_lookup_ms={} existing_check_ms={} insert_ms={} total_ms={}",
            created,
            infos.len(),
            existing.len(),
            query_start.elapsed().as_millis(),
            parent_lookup_start.elapsed().as_millis(),
            existing_check_start.elapsed().as_millis(),
            insert_start.elapsed().as_millis(),
            reconcile_started.elapsed().as_millis()
        );
    }

    Ok(created)
}

// ============================================================================
// 零 DB 写入模式：将 SQL 输出到 .surql 文件
// ============================================================================

use super::sql_file_writer::SqlFileWriter;
/// inst_relate 中 fn::* 的预计算结果缓存
pub struct InstRelatePrecomputed {
    /// refno → zone PE key (e.g. "pe:⟨17496_8517⟩")，None 表示未找到 ZONE 祖先
    zone_map: HashMap<RefnoEnum, Option<String>>,
    /// refno → spec_value (i64)
    spec_map: HashMap<RefnoEnum, i64>,
    /// refno → ses_date (Option<String>，SurrealDB datetime 格式)
    dt_map: HashMap<RefnoEnum, Option<String>>,
    /// refno → dbnum
    dbnum_map: HashMap<RefnoEnum, u32>,
}

impl InstRelatePrecomputed {
    pub fn from_generation_read(
        read: &super::context::GenerationReadContext,
    ) -> anyhow::Result<Self> {
        let mut zone_map = HashMap::new();
        let mut spec_map = HashMap::new();
        let mut dt_map = HashMap::new();
        let mut dbnum_map = HashMap::new();
        for refno in read.hierarchy.all_refnos() {
            let node = read
                .hierarchy
                .node(refno)
                .ok_or_else(|| anyhow::anyhow!("hierarchy missing refno={refno}"))?;
            anyhow::ensure!(
                read.session.manifest().versions.contains_key(&node.dbnum),
                "refno={refno} dbnum={} 不在输入版本清单中",
                node.dbnum
            );
            zone_map.insert(refno, None);
            spec_map.insert(refno, 0);
            dt_map.insert(refno, None);
            dbnum_map.insert(refno, node.dbnum);
        }
        Ok(Self {
            zone_map,
            spec_map,
            dt_map,
            dbnum_map,
        })
    }

    /// 从 TreeIndex 本地缓存 + 批量 DB 读取构建预计算缓存。
    ///
    /// - zone_refno: 使用默认值 NONE（已禁用 TreeIndex 查询）
    /// - spec_value: 使用默认值 0（已禁用 DB 查询）
    /// - dt: 批量读 ses 表（一次 DB 读）
    pub async fn build(refnos: &[RefnoEnum]) -> Self {
        let mut zone_map: HashMap<RefnoEnum, Option<String>> = HashMap::new();
        let mut spec_map: HashMap<RefnoEnum, i64> = HashMap::new();
        let mut dt_map: HashMap<RefnoEnum, Option<String>> = HashMap::new();
        let mut dbnum_map: HashMap<RefnoEnum, u32> = HashMap::new();

        if refnos.is_empty() {
            return Self {
                zone_map,
                spec_map,
                dt_map,
                dbnum_map,
            };
        }

        // 1. zone_refno: 使用默认值 NONE（已禁用查询）
        for &refno in refnos {
            zone_map.insert(refno, None);
        }

        // 2. spec_value: 使用默认值 0（已禁用查询）
        for &refno in refnos {
            spec_map.insert(refno, 0);
        }

        // 3. dt (ses_date): 批量读 PE 的 dbnum+sesno，再批量读 ses 表
        // 收集所有 PE 的 dbnum 和 sesno
        {
            let pe_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
            // 分批查询避免 SQL 过长
            let mut pe_dbnum_sesno: HashMap<String, (u32, u32)> = HashMap::new();
            for chunk in pe_keys.chunks(500) {
                let sql = format!(
                    "SELECT record::id(id) AS rid, dbnum, sesno FROM [{}];",
                    chunk.join(",")
                );
                match project_primary_db().query_response(&sql).await {
                    Ok(mut resp) => {
                        let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
                        for row in rows {
                            if let Some(rid) = row.get("rid").and_then(|v| v.as_str()) {
                                let dbnum =
                                    row.get("dbnum").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                let sesno =
                                    row.get("sesno").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                pe_dbnum_sesno.insert(rid.to_string(), (dbnum, sesno));
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[precompute] 批量读取 PE dbnum/sesno 失败: {}", e);
                    }
                }
            }

            // 构建唯一的 ses ID 集合并批量查询 date
            let mut ses_keys: HashSet<String> = HashSet::new();
            for (_, (dbnum, sesno)) in &pe_dbnum_sesno {
                if *sesno > 0 {
                    ses_keys.insert(format!("ses:[{},{}]", dbnum, sesno));
                }
            }

            let mut ses_date_map: HashMap<String, String> = HashMap::new();
            if !ses_keys.is_empty() {
                let keys_vec: Vec<String> = ses_keys.into_iter().collect();
                for chunk in keys_vec.chunks(500) {
                    let sql = format!(
                        "SELECT record::id(id) AS rid, date FROM [{}];",
                        chunk.join(",")
                    );
                    match project_primary_db().query_response(&sql).await {
                        Ok(mut resp) => {
                            let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
                            for row in rows {
                                if let (Some(rid), Some(date)) = (
                                    row.get("rid").and_then(|v| v.as_str()),
                                    row.get("date").and_then(|v| v.as_str()),
                                ) {
                                    ses_date_map.insert(rid.to_string(), date.to_string());
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[precompute] 批量读取 ses date 失败: {}", e);
                        }
                    }
                }
            }

            // 填充 dt_map
            for &refno in refnos {
                let refno_str = format!("{}", refno.refno());
                if let Some((dbnum, sesno)) = pe_dbnum_sesno.get(&refno_str) {
                    dbnum_map.insert(refno, *dbnum);
                    if *sesno > 0 {
                        let ses_key = format!("[{},{}]", dbnum, sesno);
                        dt_map.insert(refno, ses_date_map.get(&ses_key).cloned());
                    } else {
                        dt_map.insert(refno, None);
                    }
                } else {
                    dbnum_map.insert(refno, 0);
                    dt_map.insert(refno, None);
                }
            }
        }

        println!(
            "[precompute] InstRelatePrecomputed 构建完成: refnos={}, zones={}, specs={}, dts={}, dbnums={}",
            refnos.len(),
            zone_map.values().filter(|v| v.is_some()).count(),
            spec_map.len(),
            dt_map.values().filter(|v| v.is_some()).count(),
            dbnum_map.len(),
        );

        Self {
            zone_map,
            spec_map,
            dt_map,
            dbnum_map,
        }
    }

    /// 获取预计算的 zone PE key
    pub fn zone_key(&self, refno: &RefnoEnum) -> String {
        self.zone_map
            .get(refno)
            .and_then(|v| v.clone())
            .unwrap_or_else(|| "NONE".to_string())
    }

    /// 获取预计算的 spec_value
    pub fn spec_value(&self, refno: &RefnoEnum) -> i64 {
        self.spec_map.get(refno).copied().unwrap_or(0)
    }

    /// 获取预计算的 ses_date
    pub fn dt(&self, refno: &RefnoEnum) -> String {
        self.dt_map
            .get(refno)
            .and_then(|v| v.clone())
            .map(|d| format!("'{}'", d))
            .unwrap_or_else(|| "NONE".to_string())
    }

    /// 获取预计算的 dbnum
    pub fn dbnum(&self, refno: &RefnoEnum) -> u32 {
        self.dbnum_map.get(refno).copied().unwrap_or(0)
    }
}

/// 将 instance 数据保存到 .surql 文件（零 DB 写入模式）。
///
/// 逻辑与 `save_instance_data_optimize` 完全对应，但所有 SQL 写入文件而非 SurrealDB，
/// 且 inst_relate 中的 `fn::find_ancestor_type` / `fn::ses_date` 已替换为预计算常量值。
#[cfg_attr(
    feature = "profile",
    tracing::instrument(skip_all, name = "save_instance_data_to_sql_file")
)]
pub async fn save_instance_data_to_sql_file(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    writer: &SqlFileWriter,
    precomputed: &InstRelatePrecomputed,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
) -> anyhow::Result<()> {
    const CHUNK_SIZE: usize = 200;
    writer.write_comment(&format!(
        "batch: inst_info={}, inst_geo_keys={}, tubi_keys={}, replace_exist={}",
        inst_mgr.inst_info_map.len(),
        inst_mgr.inst_geos_map.len(),
        inst_mgr.inst_tubi_map.len(),
        replace_exist
    ))?;

    let mut aabb_map: HashMap<u64, String> = HashMap::new();
    let mut transform_map: HashMap<u64, String> = HashMap::new();
    if let Entry::Vacant(entry) = transform_map.entry(0) {
        entry.insert(serde_json::to_string(&Transform::IDENTITY)?);
    }
    let mut vec3_map: HashMap<u64, String> = HashMap::new();
    let mut neg_geo_by_carrier: HashMap<RefnoEnum, Vec<(usize, String)>> = HashMap::new();
    let mut cata_cross_neg_geo_map: HashMap<(RefnoEnum, RefnoEnum), Vec<(usize, String)>> =
        HashMap::new();
    let inst_key_carriers = build_inst_key_carrier_map(inst_mgr);

    // DELETE（replace_exist=true 时）
    // 统一写入 .surql 文件，不直接执行到 DB（pre_cleanup_for_regen 已在前置阶段完成清理）
    if replace_exist {
        let refnos: Vec<RefnoEnum> = inst_mgr.inst_info_map.keys().copied().collect();
        let geo_hashes: Vec<u64> = inst_mgr
            .inst_geos_map
            .values()
            .flat_map(|d| d.insts.iter().map(|g| g.geo_hash))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let inst_info_ids: Vec<String> = inst_mgr
            .inst_geos_map
            .values()
            .map(|x| x.id())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // Legacy 模式：也写入 .surql 文件而非直接执行，避免阻塞 ~120 秒。
        writer.write_statements(&build_delete_inst_relate_by_in_sql(
            &refnos, CHUNK_SIZE, None,
        ))?;
        writer.write_statements(&build_delete_inst_relate_bool_records_sql(
            &refnos, CHUNK_SIZE,
        ))?;
        writer.write_statements(&build_delete_inst_geo_by_hashes_sql(
            &geo_hashes,
            CHUNK_SIZE,
        ))?;
        writer.write_statements(&build_delete_geo_relate_by_inst_info_ids_sql(
            &inst_info_ids,
            CHUNK_SIZE,
        ))?;
        writer.write_statements(&build_delete_boolean_relations_by_carriers_sql(
            &refnos, CHUNK_SIZE,
        ))?;
    }

    // inst_geo & geo_relate
    let mut inst_geo_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut geo_relate_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

    for inst_geo_data in inst_mgr.inst_geos_map.values() {
        for (geo_index, inst) in inst_geo_data.insts.iter().enumerate() {
            if inst.geo_transform.translation.is_nan()
                || inst.geo_transform.rotation.is_nan()
                || inst.geo_transform.scale.is_nan()
            {
                continue;
            }

            let transform_hash = gen_plant_transform_hash(&inst.geo_transform);
            if let Entry::Vacant(entry) = transform_map.entry(transform_hash) {
                entry.insert(serde_json::to_string(&inst.geo_transform)?);
            }

            let key_pts = inst.geo_param.key_points();
            let mut pt_hashes = Vec::with_capacity(key_pts.len());
            for key_pt in key_pts {
                let pts_hash = key_pt.gen_hash();
                pt_hashes.push(format!("vec3:⟨{}⟩", pts_hash));
                if let Entry::Vacant(entry) = vec3_map.entry(pts_hash) {
                    entry.insert(serde_json::to_string(&key_pt)?);
                }
            }

            let cat_negs_str = if !inst.cata_neg_refnos.is_empty() {
                format!(
                    ", cata_neg: [{}]",
                    inst.cata_neg_refnos.iter().map(|x| x.to_pe_key()).join(",")
                )
            } else {
                String::new()
            };

            use aios_core::geometry::GeoBasicType;
            let geom_refno = inst.refno;
            let carriers = inst_key_carriers
                .get(&inst_geo_data.id())
                .cloned()
                .unwrap_or_else(|| vec![(inst_geo_data.refno, inst_geo_data.id())]);
            for (carrier_refno, inst_info_id) in carriers {
                let relate_id = geo_relate_id_for_inst(carrier_refno, geo_index, &inst_info_id);
                let relate_json = format!(
                    r#"in: inst_info:⟨{0}⟩, out: inst_geo:⟨{1}⟩, trans: trans:⟨{2}⟩, geom_refno: pe:{3}, pts: [{4}], geo_type: '{5}', visible: {6} {7}"#,
                    inst_info_id,
                    inst.geo_hash,
                    transform_hash,
                    inst.refno,
                    pt_hashes.join(","),
                    inst.geo_type.to_string(),
                    inst.visible,
                    cat_negs_str
                );
                geo_relate_buffer.push(format!("{{ {relate_json}, id: {relate_id} }}"));
                match inst.geo_type {
                    GeoBasicType::Neg => {
                        neg_geo_by_carrier
                            .entry(carrier_refno)
                            .or_insert_with(Vec::new)
                            .push((geo_index, relate_id));
                    }
                    GeoBasicType::CataCrossNeg => {
                        cata_cross_neg_geo_map
                            .entry((carrier_refno, geom_refno))
                            .or_insert_with(Vec::new)
                            .push((geo_index, relate_id));
                    }
                    _ => {}
                }

                if geo_relate_buffer.len() >= CHUNK_SIZE {
                    writer.write_statement(&format!(
                        "INSERT RELATION INTO geo_relate [{}]",
                        geo_relate_buffer.join(",")
                    ))?;
                    geo_relate_buffer.clear();
                }
            }

            let mut geo_json = inst.gen_unit_geo_sur_json();
            if let Some(mr) = mesh_results.get(&inst.geo_hash) {
                if let Some(pos) = geo_json.rfind('}') {
                    geo_json.truncate(pos);
                    geo_json.push_str(&mr.to_insert_fields());
                    geo_json.push_str(" }");
                }
            }
            inst_geo_buffer.push(geo_json);

            if inst_geo_buffer.len() >= CHUNK_SIZE {
                writer.write_statement(&format!(
                    "INSERT IGNORE INTO inst_geo [{}]",
                    inst_geo_buffer.join(",")
                ))?;
                inst_geo_buffer.clear();
            }
        }
    }

    if !inst_geo_buffer.is_empty() {
        writer.write_statement(&format!(
            "INSERT IGNORE INTO inst_geo [{}]",
            inst_geo_buffer.join(",")
        ))?;
    }
    if !geo_relate_buffer.is_empty() {
        writer.write_statement(&format!(
            "INSERT RELATION INTO geo_relate [{}]",
            geo_relate_buffer.join(",")
        ))?;
    }

    // tubi -> aabb map
    for tubi in inst_mgr.inst_tubi_map.values() {
        if let Some(aabb) = tubi.aabb {
            let aabb_hash = gen_aabb_hash(&aabb);
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }
        }
    }

    // neg_relate
    if !inst_mgr.neg_relate_map.is_empty() {
        let mut neg_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
        for (target, neg_refnos) in &inst_mgr.neg_relate_map {
            for neg_refno in neg_refnos.iter() {
                if let Some(geo_relate_ids) = neg_geo_by_carrier.get(neg_refno) {
                    for (relation_index, (geo_index, geo_relate_id)) in
                        geo_relate_ids.iter().enumerate()
                    {
                        let neg_id = neg_relate_id(*target, *neg_refno, *geo_index, relation_index);
                        neg_buffer.push(format!(
                            "{{ in: {0}, id: {3}, out: {2}, pe: {1} }}",
                            geo_relate_id,
                            neg_refno.to_pe_key(),
                            target.to_pe_key(),
                            neg_id,
                        ));
                        if neg_buffer.len() >= CHUNK_SIZE {
                            writer.write_statement(&format!(
                                "INSERT RELATION IGNORE INTO neg_relate [{}]",
                                neg_buffer.join(",")
                            ))?;
                            neg_buffer.clear();
                        }
                    }
                }
            }
        }
        if !neg_buffer.is_empty() {
            writer.write_statement(&format!(
                "INSERT RELATION IGNORE INTO neg_relate [{}]",
                neg_buffer.join(",")
            ))?;
        }
    }

    // ngmr_relate
    if !inst_mgr.ngmr_neg_relate_map.is_empty() {
        let mut ngmr_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
        for (target_k, refnos) in &inst_mgr.ngmr_neg_relate_map {
            let target_pe = target_k.to_pe_key();
            for (ele_refno, ngmr_geom_refno) in refnos {
                let key = (*ele_refno, *ngmr_geom_refno);
                if let Some(geo_relate_ids) = cata_cross_neg_geo_map.get(&key) {
                    for (relation_index, (geo_index, geo_relate_id)) in
                        geo_relate_ids.iter().enumerate()
                    {
                        let ele_pe = ele_refno.to_pe_key();
                        let ngmr_pe = ngmr_geom_refno.to_pe_key();
                        let ngmr_id =
                            ngmr_relate_id(*target_k, *ele_refno, *geo_index, relation_index);
                        ngmr_buffer.push(format!(
                            "{{ in: {0}, id: {4}, out: {2}, pe: {1}, ngmr: {3} }}",
                            geo_relate_id, ele_pe, target_pe, ngmr_pe, ngmr_id
                        ));
                        if ngmr_buffer.len() >= CHUNK_SIZE {
                            writer.write_statement(&format!(
                                "INSERT RELATION IGNORE INTO ngmr_relate [{}]",
                                ngmr_buffer.join(",")
                            ))?;
                            ngmr_buffer.clear();
                        }
                    }
                }
            }
        }
        if !ngmr_buffer.is_empty() {
            writer.write_statement(&format!(
                "INSERT RELATION IGNORE INTO ngmr_relate [{}]",
                ngmr_buffer.join(",")
            ))?;
        }
    }

    // inst_info & inst_relate（使用预计算值替代 fn::*）
    let mut inst_info_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_ids: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_ids: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

    for (key, info) in &inst_mgr.inst_info_map {
        if info.world_transform.translation.is_nan()
            || info.world_transform.rotation.is_nan()
            || info.world_transform.scale.is_nan()
        {
            continue;
        }

        inst_info_buffer.push(info.gen_sur_json_full());
        if inst_info_buffer.len() >= CHUNK_SIZE {
            writer.write_statement(&format!(
                "INSERT IGNORE INTO inst_info [{}]",
                inst_info_buffer.join(",")
            ))?;
            inst_info_buffer.clear();
        }

        let resolved_aabb: Option<(u64, Aabb)> = resolve_element_world_aabb_for_index(
            info,
            inst_mgr.inst_geos_map.get(&info.get_inst_key()),
            mesh_results,
            mesh_aabb_map,
        )
        .map(|aabb| (gen_aabb_hash(&aabb), aabb));

        if let Some((aabb_hash, aabb)) = resolved_aabb {
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }
            inst_relate_aabb_buffer.push(format!(
                "{{id: {0}, refno: {1}, aabb_id: aabb:⟨{2}⟩}}",
                model_refno_id("inst_relate_aabb", *key),
                key.to_pe_key(),
                aabb_hash
            ));
            inst_relate_aabb_ids.push(model_refno_id("inst_relate_aabb", *key));
        }

        // inst_relate: 使用预计算值替代 fn::find_ancestor_type / fn::ses_date
        let zone_key = precomputed.zone_key(key);
        let spec_value = precomputed.spec_value(key);
        let dt = precomputed.dt(key);
        let dbnum = precomputed.dbnum(key);
        let inst_relate_id = model_refno_id("inst_relate", *key);

        let relate_sql = format!(
            "{{id: {0}, in: {1}, out: inst_info:⟨{2}⟩, dbnum: {3}, zone_refno: {4}, spec_value: {5}, dt: {6}, has_cata_neg: {7}, solid: {8}, owner_refno: {9}, owner_type: '{10}'}}",
            inst_relate_id,
            key.to_pe_key(),
            info.id_str(),
            dbnum,
            zone_key,
            spec_value,
            dt,
            info.has_cata_neg,
            info.is_solid,
            info.owner_refno.to_pe_key(),
            info.owner_type
        );
        inst_relate_buffer.push(relate_sql);
        inst_relate_ids.push(inst_relate_id);
        if inst_relate_buffer.len() >= CHUNK_SIZE {
            for statement in build_replace_rows_statements(
                "inst_relate",
                true,
                &inst_relate_buffer,
                &inst_relate_ids,
            ) {
                writer.write_statement(&statement)?;
            }
            inst_relate_buffer.clear();
            inst_relate_ids.clear();
        }
    }

    // flush remaining inst_info
    if !inst_info_buffer.is_empty() {
        writer.write_statement(&format!(
            "INSERT IGNORE INTO inst_info [{}]",
            inst_info_buffer.join(",")
        ))?;
    }

    // flush remaining inst_relate
    if !inst_relate_buffer.is_empty() {
        for statement in build_replace_rows_statements(
            "inst_relate",
            true,
            &inst_relate_buffer,
            &inst_relate_ids,
        ) {
            writer.write_statement(&statement)?;
        }
    }

    // aabb
    if !aabb_map.is_empty() {
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
        for (&hash, value) in &aabb_map {
            json_buffer.push(format!("{{'id':aabb:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                writer.write_statement(&format!(
                    "INSERT IGNORE INTO aabb [{}]",
                    json_buffer.join(",")
                ))?;
                json_buffer.clear();
            }
        }
        if !json_buffer.is_empty() {
            writer.write_statement(&format!(
                "INSERT IGNORE INTO aabb [{}]",
                json_buffer.join(",")
            ))?;
        }
    }

    // inst_relate_aabb：与 DB 直写路径同口径，按当前 refno 状态替换。
    if !inst_relate_aabb_buffer.is_empty() {
        let (deduped_rows, deduped_ids) =
            dedupe_inst_relate_aabb_rows(&inst_relate_aabb_buffer, &inst_relate_aabb_ids);
        for (rows, ids) in deduped_rows
            .chunks(CHUNK_SIZE)
            .zip(deduped_ids.chunks(CHUNK_SIZE))
        {
            for statement in build_replace_rows_statements("inst_relate_aabb", false, rows, ids) {
                writer.write_statement(&statement)?;
            }
        }
    }

    // transform
    if !transform_map.is_empty() {
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
        for (&hash, value) in &transform_map {
            json_buffer.push(format!("{{'id':trans:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                writer.write_statement(&format!(
                    "INSERT IGNORE INTO trans [{}]",
                    json_buffer.join(",")
                ))?;
                json_buffer.clear();
            }
        }
        if !json_buffer.is_empty() {
            writer.write_statement(&format!(
                "INSERT IGNORE INTO trans [{}]",
                json_buffer.join(",")
            ))?;
        }
    }

    // vec3
    if !vec3_map.is_empty() {
        let mut json_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
        for (&hash, value) in &vec3_map {
            json_buffer.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", hash, value));
            if json_buffer.len() >= CHUNK_SIZE {
                writer.write_statement(&format!(
                    "INSERT IGNORE INTO vec3 [{}]",
                    json_buffer.join(",")
                ))?;
                json_buffer.clear();
            }
        }
        if !json_buffer.is_empty() {
            writer.write_statement(&format!(
                "INSERT IGNORE INTO vec3 [{}]",
                json_buffer.join(",")
            ))?;
        }
    }

    Ok(())
}
