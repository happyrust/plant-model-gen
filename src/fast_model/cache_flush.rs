use std::collections::{HashMap, HashSet};
use std::path::Path;

use aios_core::geometry::{EleGeosInfo, ShapeInstancesData};
use aios_core::parsed_data::{CateAxisParam, TubiInfoData};
use aios_core::shape::pdms_shape::RsVec3;
use aios_core::types::hash::{gen_aabb_hash, gen_plant_transform_hash};
use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt};
use anyhow::{Context, Result};
use glam::Vec3;
use dashmap::DashMap;

use crate::fast_model::instance_cache::{CachedInstanceBatch, InstanceCacheManager};
use crate::fast_model::pdms_inst::save_instance_data_optimize;
use crate::fast_model::utils::{save_inst_relate_bool, save_transforms_to_surreal};
use crate::fast_model::pdms_inst::save_tubi_info_batch_with_replace;
use crate::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos;

#[derive(Default)]
struct MergedCacheData {
    inst_info_map: HashMap<RefnoEnum, EleGeosInfo>,
    inst_tubi_map: HashMap<RefnoEnum, EleGeosInfo>,
    inst_geos_map: HashMap<String, aios_core::geometry::EleInstGeosData>,
    neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>>,
    ngmr_neg_relate_map: HashMap<RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>>,
    inst_relate_bool_map: HashMap<RefnoEnum, crate::fast_model::instance_cache::CachedInstRelateBool>,
}

impl MergedCacheData {
    /// 按 refno 集合过滤：仅保留属于 filter 的条目。
    fn filter_by_refnos(mut self, filter: &HashSet<RefnoEnum>) -> Self {
        self.inst_info_map.retain(|k, _| filter.contains(k));
        self.inst_geos_map.retain(|_, v| filter.contains(&v.refno));
        self.inst_tubi_map.retain(|k, _| filter.contains(k));
        self.neg_relate_map.retain(|k, _| filter.contains(k));
        self.ngmr_neg_relate_map.retain(|k, _| filter.contains(k));
        self.inst_relate_bool_map.retain(|k, _| filter.contains(k));
        self
    }
}

/// 合并多个 cache batch 为“最终态”数据（用于 DB flush）。
///
/// 说明：该函数先实现为“旧语义”（inst_geos_map 对同 key 做 extend），以配合 TDD 的 RED；
/// 后续会按测试要求改为 last-write-wins 替换。
fn merge_cached_batches_for_db_flush(batches: Vec<CachedInstanceBatch>) -> MergedCacheData {
    let mut merged = MergedCacheData::default();
    for batch in batches {
        for (k, v) in batch.inst_info_map {
            merged.inst_info_map.insert(k, v);
        }
        for (k, v) in batch.inst_tubi_map {
            merged.inst_tubi_map.insert(k, v);
        }
        for (k, v) in batch.inst_geos_map {
            // last-write-wins：后批次覆盖前批次，避免多 batch 混入导致重复/旧数据污染。
            merged.inst_geos_map.insert(k, v);
        }
        for (k, v) in batch.neg_relate_map {
            // last-write-wins：与 inst_info_map/inst_geos_map 保持一致，
            // 避免旧 batch 的已删除负实体残留（幽灵负实体）。
            merged.neg_relate_map.insert(k, v);
        }
        for (k, v) in batch.ngmr_neg_relate_map {
            // last-write-wins：同上。
            merged.ngmr_neg_relate_map.insert(k, v);
        }
        for (k, v) in batch.inst_relate_bool_map {
            merged.inst_relate_bool_map.insert(k, v);
        }
    }
    merged
}

/// 从 `inst_info_map` 反推 tubi_info（组合键 id -> TubiInfoData）。
///
/// 先以“空实现”占位以配合 TDD 的 RED；后续按测试补齐。
fn collect_tubi_info_from_inst_info_map(_inst_info_map: &HashMap<RefnoEnum, EleGeosInfo>) -> DashMap<String, TubiInfoData> {
    let out: DashMap<String, TubiInfoData> = DashMap::new();

    fn parse_tubi_info_id(id: &str) -> Option<(String, i32, i32)> {
        let mut it = id.split('_');
        let cata_hash = it.next()?.to_string();
        let arrive = it.next()?.parse::<i32>().ok()?;
        let leave = it.next()?.parse::<i32>().ok()?;
        // 约定必须恰好 3 段；若还有多余段，视为非法。
        if it.next().is_some() {
            return None;
        }
        Some((cata_hash, arrive, leave))
    }

    fn find_axis_param(ptset_map: &std::collections::BTreeMap<i32, CateAxisParam>, number: i32) -> Option<CateAxisParam> {
        if let Some(v) = ptset_map.get(&number) {
            return Some(v.clone());
        }
        ptset_map.values().find(|x| x.number == number).cloned()
    }

    let mut skipped_parse = 0usize;
    let mut skipped_ptset = 0usize;

    for info in _inst_info_map.values() {
        let Some(id) = info.tubi.as_ref().and_then(|t| t.info_id.as_deref()) else {
            // tubi info_id 为 None 是正常情况（非管件元件），不计入跳过
            continue;
        };
        let Some((cata_hash, arrive, leave)) = parse_tubi_info_id(id) else {
            skipped_parse += 1;
            continue;
        };

        let a = find_axis_param(&info.ptset_map, arrive);
        let l = find_axis_param(&info.ptset_map, leave);
        let (Some(a), Some(l)) = (a, l) else {
            skipped_ptset += 1;
            continue;
        };

        out.entry(id.to_string())
            .or_insert_with(|| TubiInfoData::from_axis_params(&cata_hash, &a, &l));
    }

    if skipped_parse > 0 || skipped_ptset > 0 {
        eprintln!(
            "[cache_flush] collect_tubi_info: collected={} skipped_parse={} skipped_ptset={}",
            out.len(), skipped_parse, skipped_ptset
        );
    }

    out
}

/// 将 foyer/instance_cache 中的“最新 batch”批量写入 SurrealDB（用于备份/落库）。
///
/// 约定：
/// - 该函数只负责“缓存 -> DB”同步，不参与模型生成。
/// - 需由调用方提前 `init_surreal()`。
/// `refno_filter`: 可选的 refno 过滤集合。当指定时，仅同步属于该集合的实例数据
/// （用于 --debug-model 场景，避免同步整个 cache）。
pub async fn flush_latest_instance_cache_to_surreal(
    cache_dir: &Path,
    dbnums: Option<&[u32]>,
    replace_exist: bool,
    verbose: bool,
    refno_filter: Option<&HashSet<RefnoEnum>>,
) -> Result<usize> {
    let cache = InstanceCacheManager::new(cache_dir)
        .await
        .with_context(|| format!("打开 instance_cache 失败: {}", cache_dir.display()))?;

    let mut targets: Vec<u32> = match dbnums {
        Some(v) => v.to_vec(),
        None => cache.list_dbnums(),
    };
    targets.sort_unstable();
    targets.dedup();

    if targets.is_empty() {
        if verbose {
            println!("[cache_flush] instance_cache 为空：{}", cache_dir.display());
        }
        return Ok(0);
    }

    let mut flushed = 0usize;

    for dbnum in targets {
        let batch_ids = cache.list_batches(dbnum);
        if batch_ids.is_empty() {
            if verbose {
                println!("[cache_flush] dbnum={} 没有 batch，跳过", dbnum);
            }
            continue;
        }

        // 合并所有 batch（与 export_prepack_lod 的 cache 导出路径一致）
        let mut hit = 0usize;
        let mut miss = 0usize;
        let mut batches: Vec<CachedInstanceBatch> = Vec::with_capacity(batch_ids.len());
        for batch_id in &batch_ids {
            let Some(batch) = cache.get(dbnum, batch_id).await else {
                miss += 1;
                continue;
            };
            hit += 1;
            batches.push(batch);
        }

        let merged = merge_cached_batches_for_db_flush(batches);

        // 按 refno 过滤（--debug-model / --sync-to-db 场景）
        let merged = match refno_filter {
            Some(filter) => {
                let before = merged.inst_info_map.len();
                let merged = merged.filter_by_refnos(filter);
                if verbose {
                    println!(
                        "[cache_flush] dbnum={} refno_filter applied: {} -> {} inst_info",
                        dbnum, before, merged.inst_info_map.len()
                    );
                }
                merged
            }
            None => merged,
        };

        // 过滤后如果为空，跳过此 dbnum
        if merged.inst_info_map.is_empty()
            && merged.inst_geos_map.is_empty()
            && merged.inst_tubi_map.is_empty()
        {
            if verbose {
                println!("[cache_flush] dbnum={} 过滤后无数据，跳过", dbnum);
            }
            continue;
        }

        if verbose {
            println!(
                "[cache_flush] dbnum={} batches={}/{} inst_info={} inst_geos={} inst_tubi={} neg={} ngmr={} bool={}",
                dbnum, hit, hit + miss,
                merged.inst_info_map.len(),
                merged.inst_geos_map.len(),
                merged.inst_tubi_map.len(),
                merged.neg_relate_map.len(),
                merged.ngmr_neg_relate_map.len(),
                merged.inst_relate_bool_map.len(),
            );
        }

        let tubi_map_for_flush = merged.inst_tubi_map.clone();

        let shape = ShapeInstancesData {
            inst_info_map: merged.inst_info_map.clone(),
            inst_tubi_map: merged.inst_tubi_map.clone(),
            inst_geos_map: merged.inst_geos_map,
            neg_relate_map: merged.neg_relate_map,
            ngmr_neg_relate_map: merged.ngmr_neg_relate_map,
        };

        save_instance_data_optimize(&shape, replace_exist)
            .await
            .with_context(|| format!("写入实例数据失败: dbnum={}", dbnum))?;

        // 同步 tubi_info（从 inst_info_map 反推）
        let tubi_info_map = collect_tubi_info_from_inst_info_map(&shape.inst_info_map);
        if !tubi_info_map.is_empty() {
            let _ = save_tubi_info_batch_with_replace(&tubi_info_map, replace_exist).await?;
        }

        // 同步 tubi_relate（save_instance_data_optimize 不处理此表）
        // RUS-180: 从 inst_info_map 中收集 BRAN/HANG 类型的 owner_refno，
        // 用于清理"管件已删除但 BRAN 仍存在"场景下的残留 tubi_relate。
        let bran_owners: HashSet<RefnoEnum> = if replace_exist {
            shape.inst_info_map.values()
                .filter(|info| matches!(info.owner_type.as_str(), "BRAN" | "HANG"))
                .map(|info| info.owner_refno)
                .collect()
        } else {
            HashSet::new()
        };

        let extra_owners = if bran_owners.is_empty() { None } else { Some(&bran_owners) };
        if !tubi_map_for_flush.is_empty() || (replace_exist && !bran_owners.is_empty()) {
            let written = flush_tubi_relate_to_surreal(
                &tubi_map_for_flush, replace_exist, verbose, extra_owners,
            )
                .await
                .with_context(|| format!("写入 tubi_relate 失败: dbnum={}", dbnum))?;
            if verbose {
                println!("[cache_flush] dbnum={} tubi_relate 写入 {} 条", dbnum, written);
            }
        }

        // 补算 inst_relate_aabb：cache 中 EleGeosInfo.aabb 通常为 None，
        // 需要在 inst_relate + geo_relate 写入 SurrealDB 后，从库中计算并写入。
        let aabb_refnos: Vec<RefnoEnum> = shape.inst_info_map.keys().copied().collect();
        if !aabb_refnos.is_empty() {
            match update_inst_relate_aabbs_by_refnos(&aabb_refnos, replace_exist).await {
                Ok(()) => {
                    if verbose {
                        println!(
                            "[cache_flush] dbnum={} inst_relate_aabb 补算完成: {} 个 refno",
                            dbnum, aabb_refnos.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[cache_flush] dbnum={} inst_relate_aabb 补算失败: {}",
                        dbnum, e
                    );
                }
            }
        }

        // 回写 cache-only 布尔结果状态到 inst_relate_bool
        let mut bool_ok = 0usize;
        let mut bool_err = 0usize;
        for (refno, b) in merged.inst_relate_bool_map {
            let mesh_id = if b.mesh_id.is_empty() {
                None
            } else {
                Some(b.mesh_id.as_str())
            };
            if let Err(e) = save_inst_relate_bool(refno, mesh_id, &b.status, "cache_flush").await {
                bool_err += 1;
                if bool_err <= 3 {
                    eprintln!("[cache_flush] 写入 inst_relate_bool 失败: refno={} err={}", refno, e);
                }
            } else {
                bool_ok += 1;
            }
        }
        if bool_err > 0 && verbose {
            eprintln!(
                "[cache_flush] dbnum={} inst_relate_bool: ok={} err={}",
                dbnum, bool_ok, bool_err
            );
        }

        flushed += 1;
    }

    Ok(flushed)
}

/// 将 `inst_tubi_map` 中的 tubi 数据写入 SurrealDB 的 `tubi_relate` 表。
///
/// 流程：
/// 1. 按 owner_refno 分组，如果 replace_exist 则先删除旧记录
/// 2. 收集并写入依赖表（trans / aabb / vec3）
/// 3. 批量执行 RELATE SQL
async fn flush_tubi_relate_to_surreal(
    inst_tubi_map: &HashMap<RefnoEnum, EleGeosInfo>,
    replace_exist: bool,
    verbose: bool,
    // RUS-180: 额外需要清理 tubi_relate 的 owner_refno 集合（BRAN/HANG），
    // 用于清理"管件已删除但 owner 仍存在"场景下的残留记录。
    extra_cleanup_owners: Option<&HashSet<RefnoEnum>>,
) -> Result<usize> {
    // ---- 第 1 步：收集依赖数据 & 构建 RELATE SQL ----
    let mut trans_map: HashMap<u64, String> = HashMap::new();
    let mut aabb_rows: Vec<String> = Vec::new();
    let mut pts_rows: Vec<String> = Vec::new();
    let mut relate_stmts: Vec<String> = Vec::new();
    let mut owner_refnos: HashSet<RefnoEnum> = HashSet::new();
    // 用于去重 aabb/vec3 hash
    let mut seen_aabb: HashSet<u64> = HashSet::new();
    let mut seen_pts: HashSet<u64> = HashSet::new();

    for info in inst_tubi_map.values() {
        let leave_refno = info.refno;
        let owner_refno = info.owner_refno;
        let tubi = match info.tubi.as_ref() {
            Some(t) => t,
            None => continue,
        };
        let Some(arrive_refno) = tubi.arrive_refno else {
            continue;
        };
        let index = tubi.index.unwrap_or(0);

        owner_refnos.insert(owner_refno);

        // transform
        let trans_hash = gen_plant_transform_hash(&info.world_transform);
        trans_map
            .entry(trans_hash)
            .or_insert_with(|| serde_json::to_string(&info.world_transform).unwrap_or_default());

        // aabb
        let aabb_hash = info.aabb.map(|a| gen_aabb_hash(&a)).unwrap_or(0);
        if let Some(aabb) = info.aabb {
            if seen_aabb.insert(aabb_hash) {
                let d = serde_json::to_string(&aabb).unwrap_or_default();
                aabb_rows.push(format!("{{'id':aabb:⟨{}⟩, 'd':{}}}", aabb_hash, d));
            }
        }

        // start_pt / end_pt
        if let Some(pt) = tubi.start_pt {
            let rv = RsVec3(pt);
            let h = rv.gen_hash();
            if seen_pts.insert(h) {
                let d = serde_json::to_string(&rv).unwrap_or_default();
                pts_rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", h, d));
            }
        }
        if let Some(pt) = tubi.end_pt {
            let rv = RsVec3(pt);
            let h = rv.gen_hash();
            if seen_pts.insert(h) {
                let d = serde_json::to_string(&rv).unwrap_or_default();
                pts_rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", h, d));
            }
        }

        // arrive_axis / leave_axis
        let arrive_hash = tubi
            .arrive_axis_pt
            .map(|a| RsVec3(Vec3::from(a)).gen_hash())
            .unwrap_or(0);
        if let Some(a) = tubi.arrive_axis_pt {
            if seen_pts.insert(arrive_hash) {
                let rv = RsVec3(Vec3::from(a));
                let d = serde_json::to_string(&rv).unwrap_or_default();
                pts_rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", arrive_hash, d));
            }
        }
        let leave_hash = tubi
            .leave_axis_pt
            .map(|a| RsVec3(Vec3::from(a)).gen_hash())
            .unwrap_or(0);
        if let Some(a) = tubi.leave_axis_pt {
            if seen_pts.insert(leave_hash) {
                let rv = RsVec3(Vec3::from(a));
                let d = serde_json::to_string(&rv).unwrap_or_default();
                pts_rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", leave_hash, d));
            }
        }

        // geo hash: cata_hash 是 Option<String>（u64 的字符串形式），默认用 TUBI_GEO_HASH
        let geo_hash = info
            .cata_hash
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(aios_core::prim_geo::basic::TUBI_GEO_HASH);

        // start/end hash
        let start_hash = tubi.start_pt.map(|p| RsVec3(p).gen_hash()).unwrap_or(0);
        let end_hash = tubi.end_pt.map(|p| RsVec3(p).gen_hash()).unwrap_or(0);

        // RELATE SQL
        relate_stmts.push(format!(
            "relate {}->tubi_relate:[{}, {}]->{} \
             set geo=inst_geo:⟨{geo_hash}⟩,aabb=aabb:⟨{aabb_hash}⟩,\
             world_trans=trans:⟨{trans_hash}⟩,\
             start_pt=vec3:⟨{start_hash}⟩,end_pt=vec3:⟨{end_hash}⟩,\
             arrive_axis=vec3:⟨{arrive_hash}⟩,leave_axis=vec3:⟨{leave_hash}⟩,\
             bore_size=0,bad=false,\
             system={},dt=fn::ses_date({});",
            leave_refno.to_pe_key(),
            owner_refno.to_pe_key(),
            index,
            arrive_refno.to_pe_key(),
            owner_refno.to_pe_key(),
            leave_refno.to_pe_key(),
        ));
    }

    if relate_stmts.is_empty() {
        return Ok(0);
    }

    let total = relate_stmts.len();
    if verbose {
        println!(
            "[cache_flush] tubi_relate: {} 条 RELATE, {} owners, trans={}, aabb={}, pts={}",
            total,
            owner_refnos.len(),
            trans_map.len(),
            aabb_rows.len(),
            pts_rows.len(),
        );
    }

    // ---- 第 2 步：如果 replace_exist，先删除旧的 tubi_relate ----
    // RUS-180: 合并本次出现的 owner 与额外清理目标，确保已删除管件的 owner 也被清理。
    if replace_exist {
        let mut all_owners = owner_refnos.clone();
        if let Some(extra) = extra_cleanup_owners {
            all_owners.extend(extra.iter().copied());
        }
        for owner in &all_owners {
            let pe = owner.to_pe_key();
            let sql = format!("DELETE tubi_relate:[{pe}, 0]..[{pe}, ..];");
            SUL_DB.query(&sql).await.with_context(|| format!("删除旧 tubi_relate 失败 owner={owner} sql={sql}"))?;
        }
    }

    // ---- 第 3 步：写入依赖表 trans ----
    if !trans_map.is_empty() {
        save_transforms_to_surreal(&trans_map).await?;
    }

    // ---- 第 4 步：写入依赖表 aabb ----
    for chunk in aabb_rows.chunks(300) {
        let sql = format!("INSERT IGNORE INTO aabb [{}];", chunk.join(","));
        SUL_DB.query(&sql).await.with_context(|| format!("写入 aabb 失败: {sql}"))?;
    }

    // ---- 第 5 步：写入依赖表 vec3 ----
    for chunk in pts_rows.chunks(100) {
        let sql = format!("INSERT IGNORE INTO vec3 [{}];", chunk.join(","));
        SUL_DB.query(&sql).await.with_context(|| format!("写入 vec3 失败: {sql}"))?;
    }

    // ---- 第 6 步：批量执行 RELATE（RUS-184: 事务包裹，保证单 chunk 原子性）----
    for chunk in relate_stmts.chunks(50) {
        let body = chunk.join("");
        let sql = format!("BEGIN TRANSACTION;\n{body}\nCOMMIT TRANSACTION;");
        SUL_DB.query(&sql).await.with_context(|| format!("执行 tubi_relate RELATE 失败: {sql}"))?;
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn merge_inst_geos_should_be_last_write_wins() {
        let r1 = RefnoEnum::from_str("24381/1").unwrap();

        let g1 = aios_core::geometry::EleInstGeosData {
            inst_key: "k".to_string(),
            refno: r1,
            insts: vec![],
            aabb: None,
            type_name: "A".to_string(),
            ..Default::default()
        };
        let g2 = aios_core::geometry::EleInstGeosData {
            inst_key: "k".to_string(),
            refno: r1,
            insts: vec![],
            aabb: None,
            type_name: "B".to_string(),
            ..Default::default()
        };

        let b1 = CachedInstanceBatch {
            dbnum: 1,
            batch_id: "b1".to_string(),
            created_at: 1,
            inst_info_map: HashMap::new(),
            inst_geos_map: HashMap::from([("k".to_string(), g1)]),
            inst_tubi_map: HashMap::new(),
            neg_relate_map: HashMap::new(),
            ngmr_neg_relate_map: HashMap::new(),
            inst_relate_bool_map: HashMap::new(),
        };
        let b2 = CachedInstanceBatch {
            dbnum: 1,
            batch_id: "b2".to_string(),
            created_at: 2,
            inst_info_map: HashMap::new(),
            inst_geos_map: HashMap::from([("k".to_string(), g2)]),
            inst_tubi_map: HashMap::new(),
            neg_relate_map: HashMap::new(),
            ngmr_neg_relate_map: HashMap::new(),
            inst_relate_bool_map: HashMap::new(),
        };

        let merged = merge_cached_batches_for_db_flush(vec![b1, b2]);
        let got = merged.inst_geos_map.get("k").unwrap();
        assert_eq!(got.type_name, "B");
    }

    #[test]
    fn collect_tubi_info_should_return_entries_when_tubi_info_id_present() {
        let r1 = RefnoEnum::from_str("24381/145018").unwrap();
        let mut info = EleGeosInfo::default();
        info.refno = r1;
        info.tubi = Some(aios_core::geometry::TubiData {
            info_id: Some("123_1_2".to_string()),
            ..Default::default()
        });
        info.ptset_map.insert(
            1,
            CateAxisParam {
                number: 1,
                ..Default::default()
            },
        );
        info.ptset_map.insert(
            2,
            CateAxisParam {
                number: 2,
                ..Default::default()
            },
        );

        let mut inst_info_map = HashMap::new();
        inst_info_map.insert(r1, info);

        let m = collect_tubi_info_from_inst_info_map(&inst_info_map);
        assert_eq!(m.len(), 1);
    }
}
