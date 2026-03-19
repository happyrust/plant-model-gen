use std::collections::{HashMap, hash_map::Entry};

use aios_core::Transform;
use aios_core::geometry::ShapeInstancesData;
use aios_core::parsed_data::TubiInfoData;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::*;
use aios_core::types::*;
use aios_core::{
    SurrealQueryExt, gen_aabb_hash, gen_plant_transform_hash, gen_string_hash, get_db_option,
    model_primary_db, model_query_response,
};
use dashmap::DashMap;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use itertools::Itertools;
use rkyv::vec;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use parry3d::bounding_volume::{Aabb, BoundingVolume};

use super::mesh_generate::MeshResult;
use super::refno_assoc_index::RefnoAssocIndexBatch;
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::fast_model::debug_model_debug;
use crate::fast_model::shared::aabb_apply_transform;
// use crate::fast_model::EXIST_MESH_GEOS;

#[inline]
fn is_refno_assoc_index_enabled() -> bool {
    true
}

#[derive(Debug, Default, Clone)]
pub struct SaveInstanceDataReport {
    pub missing_neg_carriers: Vec<RefnoEnum>,
}

/// 将 tubi_info 数据写入数据库（可选覆盖）。
///
pub async fn save_tubi_info_batch_with_replace(
    tubi_info_map: &dashmap::DashMap<String, TubiInfoData>,
    _replace_exist: bool,
) -> anyhow::Result<usize> {
    use anyhow::Context;

    if tubi_info_map.is_empty() {
        return Ok(0);
    }

    const CHUNK_SIZE: usize = 200;
    let ids = tubi_info_map
        .iter()
        .map(|e| e.key().clone())
        .collect::<Vec<_>>();
    let mut written = 0usize;

    for chunk in ids.chunks(CHUNK_SIZE) {
        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
        for id in chunk {
            let Some(v) = tubi_info_map.get(id) else {
                continue;
            };
            rows.push(v.value().to_surreal_json());
            written += 1;
        }
        if !rows.is_empty() {
            let sql = format!("INSERT IGNORE INTO tubi_info [{}];", rows.join(","));
            model_query_response(&sql)
                .await
                .with_context(|| format!("写入 tubi_info 失败 (insert ignore): {}", written))?;
        }
    }

    Ok(written)
}

/// replace_exist=true 时，仅删除 inst_relate（按 in=pe），避免级联误删 inst_info/inst_geo，
/// 以支持“inst_relate 重建 + inst_info/ptset 复用”的工作流。
async fn delete_inst_relate_by_in(refnos: &[RefnoEnum], chunk_size: usize) -> anyhow::Result<()> {
    for sql in build_delete_inst_relate_by_in_sql(refnos, chunk_size, None) {
        model_query_response(&sql).await?;
    }
    Ok(())
}

async fn delete_inst_relate_by_in_with_dbnum(
    refnos: &[RefnoEnum],
    chunk_size: usize,
    dbnum: u32,
) -> anyhow::Result<()> {
    for sql in build_delete_inst_relate_by_in_sql(refnos, chunk_size, Some(dbnum)) {
        model_query_response(&sql).await?;
    }
    Ok(())
}

/// replace_exist=true 时，删除指定 inst_info 的 geo_relate（关系表）记录，避免旧几何残留导致同一实例出现多份 Pos。
async fn delete_geo_relate_by_inst_info_ids(
    inst_info_ids: &[String],
    chunk_size: usize,
) -> anyhow::Result<()> {
    for sql in build_delete_geo_relate_by_inst_info_ids_sql(inst_info_ids, chunk_size) {
        model_query_response(&sql).await?;
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
        model_query_response(&sql).await?;
    }
    Ok(())
}

/// replace_exist=true 时，清理实例/元件库布尔结果表，避免导出链路误读“历史 booled mesh”。
///
/// 典型症状：
/// - 当前轮生成/关系扫描显示 neg/ngmr=0（不会触发布尔 worker），
/// - 但 `inst_relate_bool:⟨refno⟩` 仍残留 status=Success，导致导出优先使用旧的 booled mesh，
///   表现为模型出现莫名缺口/截面不对。
async fn delete_inst_relate_bool_records(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    for sql in build_delete_inst_relate_bool_records_sql(refnos, chunk_size) {
        model_query_response(&sql).await?;
    }
    Ok(())
}

/// replace_exist=true 时，删除目标 BRAN/HANG 的所有 tubi_relate 直段记录。
///
/// 典型症状：
/// - BRAN/HANG 重新生成后，新世界坐标直段已写入；
/// - 但旧的局部坐标 tubi_relate 仍残留在同一 branch range 下；
/// - 导出阶段按 `tubi_relate:[bran,0]..[bran,..]` 全量读取时，会把新旧两套直段一起带出。
async fn delete_tubi_relate_by_branch_refnos(
    branch_refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<()> {
    if branch_refnos.is_empty() {
        return Ok(());
    }

    for sql in build_delete_tubi_relate_by_branch_refnos_sql(branch_refnos, chunk_size) {
        model_query_response(&sql).await?;
    }
    Ok(())
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
            .map(|r| format!("inst_relate_bool:⟨{}⟩", r))
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
            let pe_key = branch_refno.to_pe_key();
            statements.push(format!(
                "LET $ids = SELECT VALUE id FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..]; DELETE $ids;"
            ));
        }
        out.push(statements.join("\n"));
    }
    out
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
        assert!(sqls[0].contains("tubi_relate:[pe:⟨24381_145569⟩, 0]..[pe:⟨24381_145569⟩, ..]"));
        assert!(sqls[0].contains("tubi_relate:[pe:⟨24381_145570⟩, 0]..[pe:⟨24381_145570⟩, ..]"));
    }
}

/// replace_exist=true 时，删除本次将要重建的 inst_geo 记录（按 geo_hash 点删）。
///
/// 说明：inst_geo 写入目前使用 `INSERT IGNORE`，若不先删除，则旧记录（含 unit_flag/param）会被保留，
/// 导致“代码已修、--regen-model 已跑、但数据库仍是旧值”的假象。
async fn delete_inst_geo_by_hashes(geo_hashes: &[u64], chunk_size: usize) -> anyhow::Result<()> {
    for sql in build_delete_inst_geo_by_hashes_sql(geo_hashes, chunk_size) {
        model_query_response(&sql).await?;
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
    dbnum: Option<u32>,
) -> Vec<String> {
    if refnos.is_empty() {
        return Vec::new();
    }
    let mut sqls = Vec::new();
    for chunk in refnos.chunks(chunk_size.max(1)) {
        let in_keys = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");
        if let Some(dbnum) = dbnum {
            sqls.push(format!(
                "LET $ids = SELECT VALUE id FROM inst_relate WHERE dbnum = {dbnum} AND in IN [{in_keys}];\nDELETE $ids;"
            ));
        } else {
            sqls.push(format!(
                "LET $ids = SELECT VALUE id FROM [{in_keys}]->inst_relate;\nDELETE $ids;"
            ));
        }
    }
    sqls
}

async fn query_refno_dbnum_map(refnos: &[RefnoEnum], chunk_size: usize) -> HashMap<RefnoEnum, u32> {
    if refnos.is_empty() {
        return HashMap::new();
    }

    let mut refno_by_rid: HashMap<String, RefnoEnum> = HashMap::with_capacity(refnos.len());
    let mut dbnum_map: HashMap<RefnoEnum, u32> = HashMap::with_capacity(refnos.len());
    for &refno in refnos {
        refno_by_rid.insert(format!("{}", refno.refno()), refno);
        dbnum_map.insert(refno, 0);
    }

    for chunk in refnos.chunks(chunk_size.max(1)) {
        let ids = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT record::id(id) AS rid, dbnum FROM [{}];", ids);

        match model_primary_db().query_response(&sql).await {
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
pub async fn pre_cleanup_for_regen(seed_refnos: &[RefnoEnum]) -> anyhow::Result<()> {
    // if seed_refnos.is_empty() {
    return Ok(());
    // }

    const CHUNK_SIZE: usize = 200;

    // 展开 seed_refnos 到所有后代（包含自身），不过滤 noun 类型
    let all_refnos =
        aios_core::collect_descendant_filter_ids_with_self(seed_refnos, &[], None, true).await?;
    let bran_refnos = aios_core::collect_descendant_filter_ids_with_self(
        seed_refnos,
        &["BRAN", "HANG"],
        None,
        true,
    )
    .await?;

    println!(
        "[pre_cleanup_for_regen] seed_refnos={}, 展开后 all_refnos={}, bran_or_hang={}",
        seed_refnos.len(),
        all_refnos.len(),
        bran_refnos.len()
    );

    if all_refnos.is_empty() {
        return Ok(());
    }

    let t = std::time::Instant::now();

    if is_refno_assoc_index_enabled() {
        let index_started = std::time::Instant::now();
        let summary = super::refno_assoc_index::delete_by_refnos(&all_refnos, CHUNK_SIZE).await?;
        println!(
            "[pre_cleanup_for_regen] refno_assoc_index requested_refnos={} indexed_refnos={} cache_miss_refnos={} used_index={} deleted_statements={} inst_relate_ids={} inst_info_ids={} geo_relate_ids={} geo_hashes={} neg_relate_ids={} ngmr_relate_ids={} inst_relate_bool_ids={} inst_relate_cata_bool_ids={} inst_relate_aabb_ids={} tubi_branch_keys={} prefetched_ref0_groups={} overfetched_rows={} elapsed_ms={}",
            summary.requested_refnos,
            summary.indexed_refnos,
            summary.cache_miss_refnos,
            summary.used_index,
            summary.deleted_statement_count,
            summary.inst_relate_ids,
            summary.inst_info_ids,
            summary.geo_relate_ids,
            summary.geo_hashes,
            summary.neg_relate_ids,
            summary.ngmr_relate_ids,
            summary.inst_relate_bool_ids,
            summary.inst_relate_cata_bool_ids,
            summary.inst_relate_aabb_ids,
            summary.tubi_branch_keys,
            summary.prefetched_ref0_groups,
            summary.overfetched_rows,
            index_started.elapsed().as_millis()
        );
        if summary.used_index {
            println!(
                "[pre_cleanup_for_regen] 清理完成 (refno_assoc_index 模式)，耗时 {} ms",
                t.elapsed().as_millis()
            );
            return Ok(());
        }
    }

    let refno_dbnum_map = query_refno_dbnum_map(&all_refnos, CHUNK_SIZE).await;
    let mut refnos_by_dbnum: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &refno in &all_refnos {
        let dbnum = *refno_dbnum_map.get(&refno).unwrap_or(&0);
        refnos_by_dbnum.entry(dbnum).or_default().push(refno);
    }
    println!(
        "[pre_cleanup_for_regen] dbnum 分组完成: groups={}",
        refnos_by_dbnum.len()
    );

    // 2. 降级使用分批高并发扫描删除 (Legacy 模式)

    // 限制最大并发数，以防止对单一 SurrealDB 底层施加过大连接压力
    use futures::stream::{self, StreamExt};
    let limit_concurrency = 16;

    let mut chunks: Vec<(u32, Vec<RefnoEnum>)> = Vec::new();
    for (dbnum, refs) in refnos_by_dbnum {
        for chunk in refs.chunks(CHUNK_SIZE) {
            chunks.push((dbnum, chunk.to_vec()));
        }
    }
    let mut chunk_stream = stream::iter(chunks)
        .map(|(dbnum, chunk_vec)| {
            tokio::spawn(async move {
                let pe_keys = chunk_vec.iter().map(|r| r.to_pe_key()).collect::<Vec<_>>().join(",");

                // 步骤 a: 获取关联的 geo_relate -> inst_geo (如果需要删除的话)
                let sql = format!(
                    "LET $inst_ids = SELECT VALUE out FROM inst_relate WHERE dbnum = {dbnum} AND in IN [{pe_keys}];\
                     SELECT VALUE record::id(out) FROM geo_relate WHERE in IN $inst_ids;"
                );

                let geo_hashes: Vec<String> = model_primary_db()
                    .query_take(&sql, 1)
                    .await
                    .unwrap_or_default();

                let hashes: Vec<u64> = geo_hashes
                    .iter()
                    .filter_map(|s| parse_inst_geo_hash(s))
                    .collect();

                if !hashes.is_empty() {
                    let _ = delete_inst_geo_by_hashes(&hashes, CHUNK_SIZE).await;
                }

                // 步骤 b: 删除 geo_relate
                let sql_relate = format!(
                    "LET $inst_ids = SELECT VALUE out FROM inst_relate WHERE dbnum = {dbnum} AND in IN [{pe_keys}];\
                     DELETE FROM geo_relate WHERE in IN $inst_ids;"
                );
                let _ = model_query_response(&sql_relate).await;

                // 步骤 c: 删除 inst_relate
                let _ = delete_inst_relate_by_in_with_dbnum(&chunk_vec, CHUNK_SIZE, dbnum).await;

                Ok::<(), anyhow::Error>(())
            })
        })
        .buffer_unordered(limit_concurrency);

    while let Some(res) = chunk_stream.next().await {
        match res {
            Ok(Err(e)) => eprintln!("[pre_cleanup_for_regen] chunk 处理失败返回: {}", e),
            Err(e) => eprintln!("[pre_cleanup_for_regen] chunk tokio 任务崩溃: {}", e),
            _ => {}
        }
    }

    // 处理独立的记录（bool 记录、负实体关系等）
    let bool_sqls = build_delete_inst_relate_bool_records_sql(&all_refnos, CHUNK_SIZE);
    let neg_sqls = build_delete_boolean_relations_by_carriers_sql(&all_refnos, CHUNK_SIZE);

    let mut misc_stream = stream::iter(bool_sqls.into_iter().chain(neg_sqls.into_iter()))
        .map(|sql| {
            tokio::spawn(async move {
                let _ = model_query_response(&sql).await;
                Ok::<(), anyhow::Error>(())
            })
        })
        .buffer_unordered(limit_concurrency);

    while let Some(res) = misc_stream.next().await {
        match res {
            Ok(Err(e)) => eprintln!("[pre_cleanup_for_regen] misc 独立处理失败: {}", e),
            Err(e) => eprintln!("[pre_cleanup_for_regen] misc tokio 任务崩溃: {}", e),
            _ => {}
        }
    }

    if !bran_refnos.is_empty() {
        delete_tubi_relate_by_branch_refnos(&bran_refnos, CHUNK_SIZE).await?;
    }

    println!(
        "[pre_cleanup_for_regen] 清理完成 (Legacy 并发模式)，耗时 {} ms",
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

pub async fn save_instance_data_with_report(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
    write_inst_relate_aabb: bool,
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
    const MAX_TX_STATEMENTS: usize = 5;
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
    let mut refno_assoc_batch = if is_refno_assoc_index_enabled() {
        Some(RefnoAssocIndexBatch::default())
    } else {
        None
    };

    let mut aabb_map: HashMap<u64, String> = HashMap::new();
    let mut transform_map: HashMap<u64, String> = HashMap::new();
    let inst_refnos: Vec<RefnoEnum> = inst_mgr.inst_info_map.keys().copied().collect();
    let inst_dbnum_map = query_refno_dbnum_map(&inst_refnos, CHUNK_SIZE).await;
    if let Entry::Vacant(entry) = transform_map.entry(0) {
        entry.insert(serde_json::to_string(&Transform::IDENTITY)?);
    }
    let mut vec3_map: HashMap<u64, String> = HashMap::new();

    // 收集 Neg 和 CataCrossNeg 类型的 geo_relate 映射
    // neg_geo_by_carrier: key=carrier_refno -> value=Vec<geo_relate_id>
    //   用于 neg_relate: 通过负实体 refno 找到其所有 Neg 类型的 geo_relate
    // cata_cross_neg_geo_map: key=(carrier_refno, geom_refno) -> value=Vec<geo_relate_id>
    //   用于 ngmr_relate: 通过 (负载体, ngmr_geom_refno) 找到对应的 CataCrossNeg geo_relate
    let mut neg_geo_by_carrier: HashMap<RefnoEnum, Vec<u64>> = HashMap::new();
    let mut cata_cross_neg_geo_map: HashMap<(RefnoEnum, RefnoEnum), Vec<u64>> = HashMap::new();

    // inst_geo & geo_relate
    let mut geo_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
    let mut inst_geo_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut geo_relate_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

    for inst_geo_data in inst_mgr.inst_geos_map.values() {
        for inst in &inst_geo_data.insts {
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

            let relate_json = format!(
                r#"in: inst_info:⟨{0}⟩, out: inst_geo:⟨{1}⟩, trans: trans:⟨{2}⟩, geom_refno: pe:{3}, pts: [{4}], geo_type: '{5}', visible: {6} {7}"#,
                inst_geo_data.id(),
                inst.geo_hash,
                transform_hash,
                inst.refno,
                pt_hashes.join(","),
                inst.geo_type.to_string(),
                inst.visible,
                cat_negs_str
            );
            let relate_id = gen_string_hash(&relate_json);
            geo_relate_buffer.push(format!("{{ {relate_json}, id: '{relate_id}' }}"));
            if let Some(batch) = refno_assoc_batch.as_mut() {
                batch.add_inst_info_id(
                    inst_geo_data.refno,
                    format!("inst_info:⟨{}⟩", inst_geo_data.id()),
                );
                batch.add_geo_relate_id(inst_geo_data.refno, format!("geo_relate:⟨{}⟩", relate_id));
                batch.add_geo_hash(inst_geo_data.refno, inst.geo_hash.to_string());
            }

            // 收集 Neg 和 CataCrossNeg 类型的 geo_relate 映射
            // carrier_refno: 拥有这个 geo_relate 的实体
            // geom_refno: inst.refno (geo_relate 中的 geom_refno 字段)
            use aios_core::geometry::GeoBasicType;
            let carrier_refno = inst_geo_data.refno;
            let geom_refno = inst.refno;
            match inst.geo_type {
                GeoBasicType::Neg => {
                    // neg_relate: 按 carrier_refno 收集所有 Neg geo_relate
                    neg_geo_by_carrier
                        .entry(carrier_refno)
                        .or_insert_with(Vec::new)
                        .push(relate_id);
                }
                GeoBasicType::CataCrossNeg => {
                    // ngmr_relate: 按 (carrier_refno, geom_refno) 收集 CataCrossNeg geo_relate
                    cata_cross_neg_geo_map
                        .entry((carrier_refno, geom_refno))
                        .or_insert_with(Vec::new)
                        .push(relate_id);
                }
                _ => {}
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

            if geo_relate_buffer.len() >= CHUNK_SIZE {
                let statement = format!(
                    "INSERT RELATION INTO geo_relate [{}];",
                    geo_relate_buffer.join(",")
                );
                geo_batcher.push(statement).await?;
                geo_relate_buffer.clear();
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
            "INSERT RELATION INTO geo_relate [{}];",
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

        // 跨 batch 兜底：若负载体的 Neg/CataNeg 几何不在当前 inst_mgr 中，
        // 则从 DB 回查 geo_relate，避免“负实体与被减实体分批写入”导致 neg_relate 丢失。
        let mut missing_carriers: HashSet<RefnoEnum> = HashSet::new();
        for neg_refnos in inst_mgr.neg_relate_map.values() {
            for neg_refno in neg_refnos.iter() {
                if !neg_geo_by_carrier.contains_key(neg_refno) {
                    missing_carriers.insert(*neg_refno);
                }
            }
        }
        if !missing_carriers.is_empty() {
            let carrier_list = missing_carriers
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                r#"SELECT
                    record::id(geom_refno) AS carrier,
                    record::id(id) AS gr_id
                FROM geo_relate
                WHERE geo_type IN ['Neg', 'CataNeg']
                  AND geom_refno IN [{carrier_list}]"#
            );
            let mut resp = model_primary_db().query_response(&sql).await?;
            let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
            let mut loaded = 0usize;
            for row in rows {
                let carrier = row
                    .get("carrier")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let gr_id = row
                    .get("gr_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if carrier.is_empty() || gr_id.is_empty() {
                    continue;
                }
                let Ok(carrier_refno) = carrier.parse::<RefnoEnum>() else {
                    continue;
                };
                let Ok(gr_id_u64) = gr_id.parse::<u64>() else {
                    continue;
                };
                neg_geo_by_carrier
                    .entry(carrier_refno)
                    .or_insert_with(Vec::new)
                    .push(gr_id_u64);
                loaded += 1;
            }
            debug_model_debug!(
                "neg_relate DB回查: missing_carriers={}, loaded_geo_relate_ids={}",
                missing_carriers.len(),
                loaded
            );
        }
        report.missing_neg_carriers.extend(
            missing_carriers
                .iter()
                .filter(|neg_refno| !neg_geo_by_carrier.contains_key(neg_refno))
                .copied(),
        );

        let mut neg_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut neg_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

        for (target, neg_refnos) in &inst_mgr.neg_relate_map {
            let target_inst = format!("inst_relate:⟨{}⟩", target);
            for neg_refno in neg_refnos.iter() {
                // 首先尝试从当前 batch 的 neg_geo_by_carrier 查找
                if let Some(geo_relate_ids) = neg_geo_by_carrier.get(neg_refno) {
                    for geo_relate_id in geo_relate_ids.iter() {
                        // ID 简化：[geo_relate_id, target_pe] 唯一确定一条关系
                        neg_buffer.push(format!(
                            "{{ in: geo_relate:⟨{0}⟩, id: ['{0}', {2}], out: {2}, pe: {1} }}",
                            geo_relate_id,         // 切割几何
                            neg_refno.to_pe_key(), // 负载体
                            target.to_pe_key(),    // 正实体（被减实体）
                        ));
                        if should_debug_neg_write(neg_refno, target) {
                            println!(
                                "[neg-write-debug] enqueue target={} carrier={} geo_relate_id={}",
                                target, neg_refno, geo_relate_id
                            );
                            debug_neg_pairs.push((*target, *neg_refno));
                        }
                        if let Some(batch) = refno_assoc_batch.as_mut() {
                            batch.add_neg_relate_id(
                                *neg_refno,
                                format!("neg_relate:['{}',{}]", geo_relate_id, target.to_pe_key()),
                            );
                        }

                        if neg_buffer.len() >= CHUNK_SIZE {
                            let statement = if replace_exist {
                                format!(
                                    "INSERT RELATION INTO neg_relate [{}];",
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
                    "INSERT RELATION INTO neg_relate [{}];",
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
            for (target, carrier) in debug_neg_pairs {
                let sql = format!(
                    "SELECT id, record::id(in) AS in_id, record::id(out) AS out_id, record::id(pe) AS pe_id \
FROM neg_relate WHERE out = {} AND pe = {}",
                    target.to_pe_key(),
                    carrier.to_pe_key()
                );
                let rows: Vec<serde_json::Value> = model_primary_db()
                    .query_take(&sql, 0)
                    .await
                    .unwrap_or_default();
                println!(
                    "[neg-write-debug] post-finish target={} carrier={} rows={}",
                    target,
                    carrier,
                    rows.len()
                );
                for row in rows {
                    println!("[neg-write-debug] row={}", row);
                }
            }
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
            let target_pe = target_k.to_pe_key();
            let target_inst = format!("inst_relate:⟨{}⟩", target_k);
            for (ele_refno, ngmr_geom_refno) in refnos {
                // 查找该 (负载体, ngmr_geom_refno) 的 CataCrossNeg geo_relate
                let key = (*ele_refno, *ngmr_geom_refno);
                if let Some(geo_relate_ids) = cata_cross_neg_geo_map.get(&key) {
                    for geo_relate_id in geo_relate_ids.iter() {
                        let ele_pe = ele_refno.to_pe_key();
                        let ngmr_pe = ngmr_geom_refno.to_pe_key();
                        // ID 简化：[geo_relate_id, target_pe] 唯一确定一条关系
                        ngmr_buffer.push(format!(
                            "{{ in: geo_relate:⟨{0}⟩, id: ['{0}', {2}], out: {2}, pe: {1}, ngmr: {3} }}",
                            geo_relate_id,  // 切割几何
                            ele_pe,         // 负载体
                            target_pe,      // 正实体（目标）
                            ngmr_pe         // NGMR 几何引用
                        ));
                        if let Some(batch) = refno_assoc_batch.as_mut() {
                            batch.add_ngmr_relate_id(
                                *ele_refno,
                                format!("ngmr_relate:['{}',{}]", geo_relate_id, target_pe),
                            );
                        }

                        if ngmr_buffer.len() >= CHUNK_SIZE {
                            let statement = if replace_exist {
                                format!(
                                    "INSERT RELATION INTO ngmr_relate [{}];",
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
                    "INSERT RELATION INTO ngmr_relate [{}];",
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
    let mut inst_relate_aabb_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_ins: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut inst_relate_aabb_chunks: Vec<(Vec<String>, Vec<String>)> = Vec::new();

    for (key, info) in &inst_mgr.inst_info_map {
        inst_keys.push(*key);
        if let Some(batch) = refno_assoc_batch.as_mut() {
            batch.add_inst_relate_id(*key, key.to_inst_relate_key());
            batch.add_inst_relate_bool_id(*key, format!("inst_relate_bool:⟨{}⟩", key));
            batch.add_inst_info_id(*key, format!("inst_info:⟨{}⟩", info.id_str()));
        }

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

        // 优先从实际网格几何计算世界空间AABB，避免使用过时的单位盒子
        let resolved_aabb: Option<(u64, Aabb)> =
            if let Some(geos_info) = inst_mgr.inst_geos_map.get(&info.get_inst_key()) {
                let mut union_aabb: Option<Aabb> = None;
                for inst in &geos_info.insts {
                    if let Some(mr) = mesh_results.get(&inst.geo_hash) {
                        if let Some(h) = mr.aabb_hash {
                            if let Some(aabb_ref) = mesh_aabb_map.get(&h.to_string()) {
                                let world_t = info.world_transform * inst.geo_transform;
                                let world_aabb = aabb_apply_transform(&aabb_ref, &world_t);
                                union_aabb = Some(match union_aabb {
                                    Some(existing) => existing.merged(&world_aabb),
                                    None => world_aabb,
                                });
                            }
                        }
                    }
                }
                union_aabb.map(|aabb| (gen_aabb_hash(&aabb), aabb))
            } else if let Some(aabb) = info.aabb {
                // 仅当无可用网格时才回退到 info.aabb（可能是占位符）
                Some((gen_aabb_hash(&aabb), aabb))
            } else {
                None
            };

        if let Some((aabb_hash, aabb)) = resolved_aabb {
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }

            let aabb_row_sql = format!(
                "{{id: {0}, refno: {1}, aabb_id: aabb:⟨{2}⟩}}",
                key.to_table_key("inst_relate_aabb"),
                key.to_pe_key(),
                aabb_hash
            );
            if let Some(batch) = refno_assoc_batch.as_mut() {
                batch.add_inst_relate_aabb_id(*key, key.to_table_key("inst_relate_aabb"));
            }
            inst_relate_aabb_buffer.push(aabb_row_sql);
            inst_relate_aabb_ins.push(key.to_pe_key());
        }

        // inst_relate 不再保存 world_trans；世界变换统一从 pe_transform 获取。
        let dbnum = inst_dbnum_map.get(key).copied().unwrap_or(0);
        let relate_sql = format!(
            "{{id: {0}, in: {1}, out: inst_info:⟨{2}⟩, dbnum: {3}, zone_refno: NONE, spec_value: 0, dt: fn::ses_date({1}), has_cata_neg: {4}, solid: {5}, owner_refno: {6}, owner_type: '{7}'}}",
            key.to_inst_relate_key(),
            key.to_pe_key(),
            info.id_str(),
            dbnum,
            info.has_cata_neg,
            info.is_solid,
            info.owner_refno.to_pe_key(),
            info.owner_type
        );

        inst_relate_buffer.push(relate_sql);
        if inst_relate_buffer.len() >= CHUNK_SIZE {
            let statement = format!(
                "INSERT RELATION INTO inst_relate [{}];",
                inst_relate_buffer.join(",")
            );
            inst_relate_batcher.push(statement).await?;
            inst_relate_buffer.clear();

            // 延后处理 inst_relate_aabb（必须在 aabb UPSERT 之后写，避免 aabb_id 侧空记录 d=NONE）
            if !inst_relate_aabb_buffer.is_empty() {
                inst_relate_aabb_chunks.push((
                    std::mem::take(&mut inst_relate_aabb_buffer),
                    std::mem::take(&mut inst_relate_aabb_ins),
                ));
            }
        }
    }

    if !inst_relate_buffer.is_empty() {
        let statement = format!(
            "INSERT RELATION INTO inst_relate [{}];",
            inst_relate_buffer.join(",")
        );
        inst_relate_batcher.push(statement).await?;
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

            if let Some(batch) = refno_assoc_batch.as_mut() {
                batch.add_tubi_branch_key(info.refno, info.refno.to_pe_key());
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

        // 统一把积累的 chunks + 剩余 buffer 一次性落库
        let mut total = 0usize;
        macro_rules! flush_pairs {
            ($rows:expr, $ins:expr) => {{
                let n = ($ins).len().min(($rows).len());
                if n > 0 {
                    for idx in (0..n).step_by(CHUNK_SIZE) {
                        let end = (idx + CHUNK_SIZE).min(n);
                        let insert_stmt = format!(
                            "INSERT IGNORE INTO inst_relate_aabb [{}];",
                            ($rows)[idx..end].join(",")
                        );
                        inst_aabb_batcher.push(insert_stmt).await?;
                    }
                    total += n;
                }
                anyhow::Result::<()>::Ok(())
            }};
        }

        for (rows, ins) in &inst_relate_aabb_chunks {
            flush_pairs!(rows, ins)?;
        }
        flush_pairs!(&inst_relate_aabb_buffer, &inst_relate_aabb_ins)?;

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

    if let Some(batch) = refno_assoc_batch.as_ref() {
        if !batch.is_empty() {
            batch.upsert_to_db().await?;
            debug_model_debug!(
                "save_instance_data_optimize upsert refno_assoc_index: refnos={}",
                inst_mgr.inst_info_map.len()
            );
        }
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

pub fn build_inst_relate_aabb_rows(
    inst_mgr: &ShapeInstancesData,
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, Aabb>,
) -> anyhow::Result<(HashMap<u64, String>, Vec<String>)> {
    let mut aabb_map: HashMap<u64, String> = HashMap::new();
    let mut inst_relate_aabb_rows: Vec<String> = Vec::new();

    for (key, info) in &inst_mgr.inst_info_map {
        let resolved_aabb: Option<(u64, Aabb)> = if let Some(aabb) = info.aabb {
            Some((gen_aabb_hash(&aabb), aabb))
        } else if let Some(geos_info) = inst_mgr.inst_geos_map.get(&info.get_inst_key()) {
            let mut union_aabb: Option<Aabb> = None;
            for inst in &geos_info.insts {
                if let Some(mr) = mesh_results.get(&inst.geo_hash) {
                    if let Some(h) = mr.aabb_hash {
                        if let Some(aabb_ref) = mesh_aabb_map.get(&h.to_string()) {
                            union_aabb = Some(match union_aabb {
                                Some(existing) => existing.merged(&*aabb_ref),
                                None => *aabb_ref,
                            });
                        }
                    }
                }
            }
            union_aabb.map(|aabb| (gen_aabb_hash(&aabb), aabb))
        } else {
            None
        };

        if let Some((aabb_hash, aabb)) = resolved_aabb {
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }

            inst_relate_aabb_rows.push(format!(
                "{{id: {0}, refno: {1}, aabb_id: aabb:⟨{2}⟩}}",
                key.to_table_key("inst_relate_aabb"),
                key.to_pe_key(),
                aabb_hash
            ));
        }
    }

    Ok((aabb_map, inst_relate_aabb_rows))
}

pub async fn save_inst_relate_aabb_rows(
    aabb_map: &HashMap<u64, String>,
    inst_relate_aabb_rows: &[String],
) -> anyhow::Result<()> {
    if aabb_map.is_empty() && inst_relate_aabb_rows.is_empty() {
        return Ok(());
    }

    const CHUNK_SIZE: usize = 100;
    const MAX_TX_STATEMENTS: usize = 5;
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
        for chunk in inst_relate_aabb_rows.chunks(CHUNK_SIZE) {
            let statement = format!("INSERT IGNORE INTO inst_relate_aabb [{}];", chunk.join(","));
            inst_aabb_batcher.push(statement).await?;
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

            // 注意：不要对 model_primary_db() 做 clone 再 query。
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
                    match model_query_response(&query).await {
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
DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb FIELDS refno UNIQUE;";
                            let _ = model_query_response(repair_sql).await;
                            continue;
                        }

                        if is_neg_relate_unique_conflict && !repaired_neg_relate_index {
                            repaired_neg_relate_index = true;
                            debug_model_debug!(
                                "⚠️ [DEBUG] 检测到 neg_relate 唯一索引冲突，尝试重建索引并重试..."
                            );
                            let repair_sql = "REMOVE INDEX unique_neg_relate ON TABLE neg_relate; \
DEFINE INDEX unique_neg_relate ON TABLE neg_relate COLUMNS in, out UNIQUE;";
                            let _ = model_query_response(repair_sql).await;
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
                        let file_name = format!("failed_sql_batch_{}.log", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
                        if let Err(write_err) = std::fs::write(&file_name, &debug_query) {
                            eprintln!("写入失败 SQL 诊断日志至 {} 时出错: {}", file_name, write_err);
                        } else {
                            eprintln!("❌ 写入失败超出重试限制，导致失败的 SQL 块已转储至 {}", file_name);
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

/// 增量保存 tubi_info 数据到数据库
///
/// 仅写入尚不存在的 tubi_info 记录，返回新增记录数量。
///
/// # 参数
/// - `tubi_info_map`: 组合键 ID -> TubiInfoData 的映射
///
/// # 返回
/// - `Ok(usize)`: 新增的记录数量
pub async fn save_tubi_info_batch(
    tubi_info_map: &DashMap<String, TubiInfoData>,
) -> anyhow::Result<usize> {
    if tubi_info_map.is_empty() {
        return Ok(0);
    }

    const CHUNK_SIZE: usize = 200;

    // 1. 查询已存在的 tubi_info ID
    let ids: Vec<String> = tubi_info_map.iter().map(|e| e.key().clone()).collect();
    let existing = query_existing_tubi_info_ids(&ids).await?;

    debug_model_debug!(
        "save_tubi_info_batch: total={}, existing={}, to_insert={}",
        ids.len(),
        existing.len(),
        ids.len() - existing.len()
    );

    // 2. 过滤出需要新建的
    let new_entries: Vec<_> = tubi_info_map
        .iter()
        .filter(|e| !existing.contains(e.key()))
        .collect();

    if new_entries.is_empty() {
        return Ok(0);
    }

    // 3. 批量 INSERT
    let mut inserted = 0;
    for chunk in new_entries.chunks(CHUNK_SIZE) {
        let values: Vec<String> = chunk.iter().map(|e| e.value().to_surreal_json()).collect();

        let sql = format!("INSERT INTO tubi_info [{}];", values.join(","));
        model_query_response(&sql).await?;
        inserted += chunk.len();

        debug_model_debug!(
            "save_tubi_info_batch: inserted chunk of {} records",
            chunk.len()
        );
    }

    Ok(inserted)
}

/// 查询已存在的 tubi_info ID 列表
async fn query_existing_tubi_info_ids(ids: &[String]) -> anyhow::Result<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }

    // 分批查询以避免 SQL 过长
    const BATCH_SIZE: usize = 500;
    let mut existing = HashSet::new();

    for chunk in ids.chunks(BATCH_SIZE) {
        let id_list: String = chunk
            .iter()
            .map(|id| format!("tubi_info:⟨{}⟩", id))
            .join(",");

        let sql = format!("SELECT VALUE record::id(id) FROM [{}];", id_list);

        let result: Vec<String> = model_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        existing.extend(result);
    }

    Ok(existing)
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
                record::id(id) as gr_id,
                record::id(geom_refno) as neg_carrier
            FROM geo_relate
            WHERE geo_type IN ['Neg', 'CataNeg']
              AND geom_refno IN [{pe_list}]"#
        );
        let mut response = model_primary_db().query_response(&sql).await?;
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
        let mut response = model_primary_db().query_response(&sql).await?;
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
            .map(|r| format!("geo_relate:⟨{}⟩", r.gr_id))
            .collect::<Vec<_>>()
            .join(",");
        let check_sql = format!("SELECT VALUE record::id(in) FROM [{gr_id_list}]->neg_relate");
        let mut check_resp = model_primary_db().query_response(&check_sql).await?;
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

        neg_buffer.push(format!(
            "{{ in: geo_relate:⟨{0}⟩, id: ['{0}', pe:⟨{2}⟩], out: pe:⟨{2}⟩, pe: pe:⟨{1}⟩ }}",
            info.gr_id, info.neg_carrier, info.parent_id,
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
            model_query_response(&sql).await?;
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
use super::tree_index_manager::TreeIndexManager;

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
                match model_primary_db().query_response(&sql).await {
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
                    match model_primary_db().query_response(&sql).await {
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
    let mut refno_assoc_batch = if is_refno_assoc_index_enabled() {
        Some(RefnoAssocIndexBatch::default())
    } else {
        None
    };

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
    let mut neg_geo_by_carrier: HashMap<RefnoEnum, Vec<u64>> = HashMap::new();
    let mut cata_cross_neg_geo_map: HashMap<(RefnoEnum, RefnoEnum), Vec<u64>> = HashMap::new();

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
        for inst in &inst_geo_data.insts {
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

            let relate_json = format!(
                r#"in: inst_info:⟨{0}⟩, out: inst_geo:⟨{1}⟩, trans: trans:⟨{2}⟩, geom_refno: pe:{3}, pts: [{4}], geo_type: '{5}', visible: {6} {7}"#,
                inst_geo_data.id(),
                inst.geo_hash,
                transform_hash,
                inst.refno,
                pt_hashes.join(","),
                inst.geo_type.to_string(),
                inst.visible,
                cat_negs_str
            );
            let relate_id = gen_string_hash(&relate_json);
            geo_relate_buffer.push(format!("{{ {relate_json}, id: '{relate_id}' }}"));
            if let Some(batch) = refno_assoc_batch.as_mut() {
                batch.add_inst_info_id(
                    inst_geo_data.refno,
                    format!("inst_info:⟨{}⟩", inst_geo_data.id()),
                );
                batch.add_geo_relate_id(inst_geo_data.refno, format!("geo_relate:⟨{}⟩", relate_id));
                batch.add_geo_hash(inst_geo_data.refno, inst.geo_hash.to_string());
            }

            use aios_core::geometry::GeoBasicType;
            let carrier_refno = inst_geo_data.refno;
            let geom_refno = inst.refno;
            match inst.geo_type {
                GeoBasicType::Neg => {
                    neg_geo_by_carrier
                        .entry(carrier_refno)
                        .or_insert_with(Vec::new)
                        .push(relate_id);
                }
                GeoBasicType::CataCrossNeg => {
                    cata_cross_neg_geo_map
                        .entry((carrier_refno, geom_refno))
                        .or_insert_with(Vec::new)
                        .push(relate_id);
                }
                _ => {}
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

            if geo_relate_buffer.len() >= CHUNK_SIZE {
                writer.write_statement(&format!(
                    "INSERT RELATION INTO geo_relate [{}]",
                    geo_relate_buffer.join(",")
                ))?;
                geo_relate_buffer.clear();
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
                    for geo_relate_id in geo_relate_ids.iter() {
                        neg_buffer.push(format!(
                            "{{ in: geo_relate:⟨{0}⟩, id: ['{0}', {2}], out: {2}, pe: {1} }}",
                            geo_relate_id,
                            neg_refno.to_pe_key(),
                            target.to_pe_key(),
                        ));
                        if let Some(batch) = refno_assoc_batch.as_mut() {
                            batch.add_neg_relate_id(
                                *neg_refno,
                                format!("neg_relate:['{}',{}]", geo_relate_id, target.to_pe_key()),
                            );
                        }
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
                    for geo_relate_id in geo_relate_ids.iter() {
                        let ele_pe = ele_refno.to_pe_key();
                        let ngmr_pe = ngmr_geom_refno.to_pe_key();
                        ngmr_buffer.push(format!(
                            "{{ in: geo_relate:⟨{0}⟩, id: ['{0}', {2}], out: {2}, pe: {1}, ngmr: {3} }}",
                            geo_relate_id, ele_pe, target_pe, ngmr_pe
                        ));
                        if let Some(batch) = refno_assoc_batch.as_mut() {
                            batch.add_ngmr_relate_id(
                                *ele_refno,
                                format!("ngmr_relate:['{}',{}]", geo_relate_id, target_pe),
                            );
                        }
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
    let mut inst_relate_aabb_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

    for (key, info) in &inst_mgr.inst_info_map {
        if let Some(batch) = refno_assoc_batch.as_mut() {
            batch.add_inst_relate_id(*key, key.to_inst_relate_key());
            batch.add_inst_relate_bool_id(*key, format!("inst_relate_bool:⟨{}⟩", key));
            batch.add_inst_info_id(*key, format!("inst_info:⟨{}⟩", info.id_str()));
        }
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

        // 优先从实际网格几何计算世界空间AABB，避免使用过时的单位盒子
        let resolved_aabb: Option<(u64, Aabb)> =
            if let Some(geos_info) = inst_mgr.inst_geos_map.get(&info.get_inst_key()) {
                let mut union_aabb: Option<Aabb> = None;
                for inst in &geos_info.insts {
                    if let Some(mr) = mesh_results.get(&inst.geo_hash) {
                        if let Some(h) = mr.aabb_hash {
                            if let Some(aabb_ref) = mesh_aabb_map.get(&h.to_string()) {
                                let world_t = info.world_transform * inst.geo_transform;
                                let world_aabb = aabb_apply_transform(&aabb_ref, &world_t);
                                union_aabb = Some(match union_aabb {
                                    Some(existing) => existing.merged(&world_aabb),
                                    None => world_aabb,
                                });
                            }
                        }
                    }
                }
                union_aabb.map(|aabb| (gen_aabb_hash(&aabb), aabb))
            } else if let Some(aabb) = info.aabb {
                // 仅当无可用网格时才回退到 info.aabb（可能是占位符）
                Some((gen_aabb_hash(&aabb), aabb))
            } else {
                None
            };

        if let Some((aabb_hash, aabb)) = resolved_aabb {
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }
            inst_relate_aabb_buffer.push(format!(
                "{{id: {0}, refno: {1}, aabb_id: aabb:⟨{2}⟩}}",
                key.to_table_key("inst_relate_aabb"),
                key.to_pe_key(),
                aabb_hash
            ));
            if let Some(batch) = refno_assoc_batch.as_mut() {
                batch.add_inst_relate_aabb_id(*key, key.to_table_key("inst_relate_aabb"));
            }
        }

        // inst_relate: 使用预计算值替代 fn::find_ancestor_type / fn::ses_date
        let zone_key = precomputed.zone_key(key);
        let spec_value = precomputed.spec_value(key);
        let dt = precomputed.dt(key);
        let dbnum = precomputed.dbnum(key);

        let relate_sql = format!(
            "{{id: {0}, in: {1}, out: inst_info:⟨{2}⟩, dbnum: {3}, zone_refno: {4}, spec_value: {5}, dt: {6}, has_cata_neg: {7}, solid: {8}, owner_refno: {9}, owner_type: '{10}'}}",
            key.to_inst_relate_key(),
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
        if inst_relate_buffer.len() >= CHUNK_SIZE {
            writer.write_statement(&format!(
                "INSERT RELATION INTO inst_relate [{}]",
                inst_relate_buffer.join(",")
            ))?;
            inst_relate_buffer.clear();
        }
    }

    if !inst_mgr.inst_tubi_map.is_empty() {
        for (_key, info) in &inst_mgr.inst_tubi_map {
            if let Some(batch) = refno_assoc_batch.as_mut() {
                batch.add_tubi_branch_key(info.refno, info.refno.to_pe_key());
            }
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
        writer.write_statement(&format!(
            "INSERT RELATION INTO inst_relate [{}]",
            inst_relate_buffer.join(",")
        ))?;
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

    // inst_relate_aabb
    if !inst_relate_aabb_buffer.is_empty() {
        // 用户要求不做删除：改为仅写入 INSERT IGNORE，
        // 由唯一键/幂等语义保证重复导入可安全跳过。
        for chunk in inst_relate_aabb_buffer.chunks(CHUNK_SIZE) {
            writer.write_statement(&format!(
                "INSERT IGNORE INTO inst_relate_aabb [{}]",
                chunk.join(",")
            ))?;
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

    if let Some(batch) = refno_assoc_batch.as_ref() {
        if !batch.is_empty() {
            batch.write_to_sql_file(writer)?;
        }
    }

    Ok(())
}
