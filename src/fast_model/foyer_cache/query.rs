//! foyer cache 专用的几何实例查询（导出期）
//!
//! 本模块用于提供 **cache-only（不访问 SurrealDB）** 的实例查询能力：
//! - 导出（OBJ/GLB/glTF/XKT）侧：获取 `GeomInstQuery`（等价于 SurrealDB `query_insts` 的返回结构）
//! - 房间计算/空间索引等下游：复用同一套“从 instance_cache 组装实例”的语义
//!
//! # 关键语义
//! - 不回退 SurrealDB；缓存缺失则返回错误或显式“缺失列表”（由调用方决定是否跳过）。
//! - instance_cache 可能存在多个 batch（多次 `--regen-model` 会追加），因此默认按“最新优先”读取，
//!   以避免旧 batch 的 transform/geo 数据污染（典型表现为尺寸被平方放大、布尔 mesh 选错等）。

use std::path::Path;

use aios_core::{GeomInstQuery, RefnoEnum};
use anyhow::Result;

use crate::fast_model::foyer_cache::FoyerCacheContext;

/// 缓存路径：从 foyer/instance_cache 读取几何实例数据，构造与 SurrealDB `query_insts` 等价的 `GeomInstQuery`。
///
/// 约定：该函数**不回退** SurrealDB；若缓存缺失则直接返回错误。
#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "cache_query_geometry_instances"))]
pub async fn query_geometry_instances_ext_from_cache(
    refnos: &[RefnoEnum],
    cache_dir: &Path,
    enable_holes: bool,
    include_negative: bool,
    verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::instance_cache::InstanceCacheManager;
    use aios_core::geometry::GeoBasicType;
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::rs_surreal::inst::ModelHashInst;
    use aios_core::types::PlantAabb;
    use aios_core::RefU64;
    use std::collections::{HashMap, HashSet};

    // cache-only：enable_holes=true 时，优先使用 instance_cache 中记录的 inst_relate_bool(Success)。
    // include_negative 暂仅保留签名一致性（缓存里仍会带 Neg/CateNeg 等记录，但导出默认不包含）。
    let _ = include_negative;

    if refnos.is_empty() {
        if verbose {
            println!("⚠️  输入参考号为空，跳过缓存查询");
        }
        return Ok(Vec::new());
    }

    db_meta().ensure_loaded()?;

    // 先按 dbnum 分组，避免跨库扫描 batch。
    //
    // 注意：缓存内的 key/refno 可能是 Refno 或 SesRef([refno,sesno]) 两种形式。
    // 为了与上层（room_calc / export）常用的 Refno 输入兼容，这里按 RefU64 归一化匹配。
    let mut by_dbnum: HashMap<u32, HashMap<RefU64, RefnoEnum>> = HashMap::new();
    let mut unresolved: Vec<RefnoEnum> = Vec::new();
    for &r in refnos {
        match db_meta().get_dbnum_by_refno(r) {
            Some(dbnum) => {
                by_dbnum.entry(dbnum).or_default().insert(r.refno(), r);
            }
            None => unresolved.push(r),
        }
    }
    if !unresolved.is_empty() {
        anyhow::bail!(
            "无法从 db_meta_info.json 推导 dbnum（请先生成 output/scene_tree/db_meta_info.json）: {:?}",
            unresolved
        );
    }

    if verbose {
        println!(
            "📦 缓存查询几何体数据: refnos={}, dbnums={}",
            refnos.len(),
            by_dbnum.len()
        );
        println!("   - 缓存目录: {}", cache_dir.display());
    }

    let cache = InstanceCacheManager::new(cache_dir).await?;

    // world_aabb：优先从 cache 的 inst_info_map 读取；若缺失则回退到 SQLite 空间索引（若启用）。
    #[cfg(feature = "sqlite-index")]
    let sqlite_idx = crate::spatial_index::SqliteSpatialIndex::with_default_path().ok();

    #[derive(Default)]
    struct Acc {
        owner: RefnoEnum,
        world_trans: PlantTransform,
        world_aabb: Option<PlantAabb>,
        has_neg: bool,
        has_cata_neg: bool,
        insts: Vec<ModelHashInst>,
    }

    let mut out: Vec<GeomInstQuery> = Vec::new();
    let mut missing: Vec<RefnoEnum> = Vec::new();
    let mut missing_cata_bool: Vec<RefnoEnum> = Vec::new();

    for (dbnum, want_map) in by_dbnum {
        let batch_ids = cache.list_batches(dbnum);
        if batch_ids.is_empty() {
            missing.extend(want_map.values().copied());
            continue;
        }

        // 先收集本 dbnum 下的 bool 成功结果（two-pass，保证 bool 覆盖原始 inst_geos）。
        //
        // 注意：instance_cache 可能存在多个 batch（多次 --regen-model 会追加 batch），
        // 若按旧到新遍历并“首次命中即使用”，会导致：
        // - 选中旧的 bool mesh_id（磁盘上可能已不存在） -> 导出表现为“某些子孙节点没导出来”
        // - 选中旧的 inst_geo transform（例如 RTOR scale 非 1）-> 尺寸被平方放大
        //
        // 因此这里按 created_at 选择“最新的 Success”。
        let mut bool_success: HashMap<RefU64, (String, i64)> = HashMap::new();
        if enable_holes {
            println!(
                "[cache_query] enable_holes=true, 扫描 {} 个 batch 查找 bool 结果 (dbnum={})",
                batch_ids.len(), dbnum
            );
            for batch_id in batch_ids.iter().rev() {
                let Some(batch) = cache.get(dbnum, batch_id).await else {
                    continue;
                };
                if !batch.inst_relate_bool_map.is_empty() {
                    println!(
                        "[cache_query] batch_id={} 含 {} 条 inst_relate_bool 记录",
                        batch_id, batch.inst_relate_bool_map.len()
                    );
                    for (r, b) in &batch.inst_relate_bool_map {
                        println!(
                            "[cache_query]   refno={} status={} mesh_id={} created_at={}",
                            r, b.status, b.mesh_id, b.created_at
                        );
                    }
                }
                for (r, b) in batch.inst_relate_bool_map {
                    let k = r.refno();
                    if !want_map.contains_key(&k) {
                        continue;
                    }
                    if b.status != "Success" || b.mesh_id.is_empty() {
                        continue;
                    }
                    match bool_success.get(&k) {
                        None => {
                            bool_success.insert(k, (b.mesh_id, b.created_at));
                        }
                        Some((_, ts)) if b.created_at > *ts => {
                            bool_success.insert(k, (b.mesh_id, b.created_at));
                        }
                        _ => {}
                    }
                }
            }
            println!(
                "[cache_query] bool_success 结果: {} 条",
                bool_success.len()
            );
            for (k, (mesh_id, ts)) in &bool_success {
                println!("[cache_query]   refno_u64={} mesh_id={} ts={}", k, mesh_id, ts);
            }
        }

        let mut acc_map: HashMap<RefU64, Acc> = HashMap::new();
        // want_refno 中哪些是“元件库负实体（cata_neg）”目标：这类必须走布尔结果（CatePos）。
        // 取“最新 inst_info”给出的 has_cata_neg（避免旧 batch 的脏数据影响导出路径）。
        let mut want_has_cata_neg: HashSet<RefU64> = HashSet::new();
        // 只取最新 batch 的 inst_info/inst_geos/inst_tubi，避免多 batch 合并导致重复/错误缩放。
        let mut seen_meta: HashSet<RefU64> = HashSet::new();
        let mut seen_geos: HashSet<RefU64> = HashSet::new();
        let mut seen_tubi: HashSet<RefU64> = HashSet::new();
        // 某些 dbnum 会出现“新 batch 只有 inst_info（world_transform）但没有 inst_geos”的情况。
        // 若直接用最新 inst_info + 旧 inst_geos，会造成 world/local 不配套，典型表现为尺寸被平方放大。
        // 因此：一旦某 refno 选择了某个 batch 的 inst_geos，则 meta(world_trans/aabb/has_neg/has_cata_neg)
        // 必须优先对齐到同一 batch（若该 batch 有 inst_info）。
        let mut meta_locked_by_geos: HashSet<RefU64> = HashSet::new();
        // tubi 需要“每段自己的 world_transform(含长度 scale)”；它与同 refno 的 inst_info.world_transform
        // 可能不同（例如 refno 同时包含弯头构件与直段 tubing），因此不能强行复用 acc.world_trans。
        // 这里把 tubi 的 world_transform 作为“实例 transform”单独保存，导出侧用 identity world_trans 直接落地。
        let mut tubi_world_insts: HashMap<
            RefU64,
            Vec<(RefnoEnum, PlantTransform, Option<PlantAabb>, String)>,
        > = HashMap::new();

        for batch_id in batch_ids.iter().rev() {
            let Some(batch) = cache.get(dbnum, batch_id).await else {
                continue;
            };

            // 先从 inst_info_map 组装“最新元数据”（owner/world_trans/aabb/has_neg/has_cata_neg）。
            // 这一步很关键：enable_holes=true 时，raw inst_geos 可能会被跳过，但导出 bool mesh 仍需要正确的世界变换。
            for (refno, info) in batch.inst_info_map.iter() {
                let k = refno.refno();
                if !want_map.contains_key(&k) {
                    continue;
                }
                if !seen_meta.insert(k) {
                    continue;
                }

                if info.has_cata_neg {
                    want_has_cata_neg.insert(k);
                }

                let entry = acc_map.entry(k).or_insert_with(Acc::default);
                let insts = std::mem::take(&mut entry.insts);

                let owner = if info.owner_refno.is_valid() {
                    info.owner_refno
                } else {
                    *refno
                };
                let mut world_aabb: Option<PlantAabb> = info.aabb.map(Into::into);
                #[cfg(feature = "sqlite-index")]
                {
                    if world_aabb.is_none() {
                        if let Some(idx) = sqlite_idx.as_ref() {
                            let id: aios_core::RefU64 = (*refno).into();
                            if let Ok(Some(aabb)) = idx.get_aabb(id) {
                                world_aabb = Some(aabb.into());
                            }
                        }
                    }
                }
                let has_neg = batch
                    .neg_relate_map
                    .get(refno)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                *entry = Acc {
                    owner,
                    world_trans: PlantTransform::from(info.world_transform),
                    world_aabb,
                    has_neg,
                    has_cata_neg: info.has_cata_neg,
                    insts,
                };
            }

            // tubing 节点在 cache 中以 inst_tubi_map(EleGeosInfo) 形式存在：
            // - 通常不会出现在 inst_geos_map（否则会被当作普通构件几何）
            // - 导出/房间计算期需要把它们拼成一条“带 is_tubi=true 的几何实例”
            //
            // 注意：这里用 world_trans + local(identity) 表达 tubing 的世界变换，
            // 以复用导出侧统一的 world_trans * geo_transform 逻辑。
            {
                use aios_core::prim_geo::basic::TUBI_GEO_HASH;
                for (refno, info) in batch.inst_tubi_map.iter() {
                    let k = refno.refno();
                    if !want_map.contains_key(&k) {
                        continue;
                    }
                    if !seen_tubi.insert(k) {
                        continue;
                    }

                    // 记录该 tubi 段的独立 world_transform（通常包含沿轴向的长度 scale）。
                    let owner = if info.owner_refno.is_valid() {
                        info.owner_refno
                    } else {
                        *refno
                    };
                    let geo_hash = info
                        .cata_hash
                        .clone()
                        .unwrap_or_else(|| TUBI_GEO_HASH.to_string());
                    tubi_world_insts
                        .entry(k)
                        .or_default()
                        .push((
                            owner,
                            PlantTransform::from(info.world_transform),
                            info.aabb.map(Into::into),
                            geo_hash,
                        ));

                    let entry = acc_map.entry(k).or_insert_with(|| Acc {
                        owner: *refno,
                        world_trans: PlantTransform::default(),
                        world_aabb: None,
                        has_neg: false,
                        has_cata_neg: false,
                        insts: Vec::new(),
                    });

                    entry.owner = if info.owner_refno.is_valid() {
                        info.owner_refno
                    } else {
                        *refno
                    };
                    // tubing 的 EleGeosInfo 也带 world_transform/aabb，可作为 inst_info 缺失时的 fallback。
                    // 若已命中 inst_info 的 meta，则不在此处覆写，避免把 has_neg/has_cata_neg 等信息误清零。
                    if !meta_locked_by_geos.contains(&k) && !seen_meta.contains(&k) {
                        seen_meta.insert(k);
                        entry.world_trans = PlantTransform::from(info.world_transform);
                        entry.world_aabb = info.aabb.map(Into::into);
                    }
                }
            }

            // 遍历 inst_info_map，使用 get_inst_key() 查找对应的几何数据。
            // 这样即使多个 refno 共享相同的 cata_hash，也能为每个 refno 获取几何数据。
            for (refno, info) in batch.inst_info_map.iter() {
                let refno_u64 = refno.refno();
                if !want_map.contains_key(&refno_u64) {
                    continue;
                }

                // 使用 info.get_inst_key() 查找对应的几何数据
                let inst_key = info.get_inst_key();
                let geos_data = match batch.inst_geos_map.get(&inst_key) {
                    Some(data) => data,
                    None => continue, // 没有几何数据，跳过
                };

                // 注意：instance_cache 可能出现“最新 batch 只有 inst_info（world_transform）但没有 inst_geos”的情况。
                // 若此时继续使用“最新 meta + 旧 batch inst_geos”，会造成 world/local 不配套，典型表现为尺寸被平方放大。
                //
                // 因此：一旦我们选择了某个 batch 的 inst_geos，就必须把 meta(owner/world_trans/aabb/has_neg/has_cata_neg)
                // 对齐到**同一 batch**的 inst_info。
                let owner = if info.owner_refno.is_valid() {
                    info.owner_refno
                } else {
                    *refno
                };
                let mut world_aabb: Option<PlantAabb> = info.aabb.map(Into::into);
                #[cfg(feature = "sqlite-index")]
                {
                    if world_aabb.is_none() {
                        if let Some(idx) = sqlite_idx.as_ref() {
                            let id: aios_core::RefU64 = (*refno).into();
                            if let Ok(Some(aabb)) = idx.get_aabb(id) {
                                world_aabb = Some(aabb.into());
                            }
                        }
                    }
                }
                let has_neg = batch
                    .neg_relate_map
                    .get(refno)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);

                let entry = acc_map.entry(refno_u64).or_insert_with(Acc::default);

                // enable_holes=true 且已有 booled mesh：保留 owner/world_trans/has_neg，但不再收集原始 inst_geos。
                if enable_holes && bool_success.contains_key(&refno_u64) {
                    // 有 bool mesh 时，不应再让旧 batch 的 raw inst_geos 混入（否则可能出现重复/错误缩放）。
                    seen_geos.insert(refno_u64);
                    continue;
                }
                // 只取最新 batch 的 raw inst_geos（避免多 batch 合并导致重复/旧数据污染）。
                if !seen_geos.insert(refno_u64) {
                    continue;
                }

                // 标记本 refno 的几何数据已锁定到当前 batch
                meta_locked_by_geos.insert(refno_u64);

                // 锁定 meta 到当前 batch 的 inst_info（避免“最新 meta + 旧 inst_geos”导致尺寸异常）
                let insts = std::mem::take(&mut entry.insts);
                *entry = Acc {
                    owner,
                    world_trans: PlantTransform::from(info.world_transform),
                    world_aabb,
                    has_neg,
                    has_cata_neg: info.has_cata_neg,
                    insts,
                };

                for inst in &geos_data.insts {
                    if !inst.visible {
                        continue;
                    }
                    match inst.geo_type {
                        GeoBasicType::Pos
                        | GeoBasicType::DesiPos
                        | GeoBasicType::CatePos
                        | GeoBasicType::Compound => {}
                        _ => {
                            continue;
                        }
                    }

                    entry.insts.push(ModelHashInst {
                        geo_hash: inst.geo_hash.to_string(),
                        geo_transform: PlantTransform::from(inst.geo_transform),
                        is_tubi: inst.is_tubi,
                        unit_flag: inst.geo_param.is_reuse_unit(),
                    });
                }
            }
        }

        // 对每个想要的 refno（以 RefU64 归一化）组装输出；refno 字段使用调用方输入，避免泄露 SesRef 形式。
        for (want_u64, want_refno) in want_map {
            if enable_holes {
                if let Some((mesh_id, _)) = bool_success.get(&want_u64) {
                    // bool mesh 在 cache bool_worker 中已写盘到 lod_{default}/ 目录；
                    // 其坐标系约定为 refno local space。
                    //
                    // 注意：导出侧（export_obj 等）对 has_neg=true 的约定是：
                    // inst.geo_transform 已经是 world_trans.d（等价 SurrealDB booled_id 查询返回值）。
                    // 若此处使用 identity，则导出时会丢失世界变换，常见表现为子节点（布尔结果 mesh）方位/位置不对。
                    let acc = acc_map.remove(&want_u64).unwrap_or(Acc {
                        owner: want_refno,
                        world_trans: PlantTransform::default(),
                        world_aabb: None,
                        has_neg: true,
                        has_cata_neg: false,
                        insts: Vec::new(),
                    });
                    out.push(GeomInstQuery {
                        refno: want_refno,
                        owner: acc.owner,
                        world_aabb: acc.world_aabb,
                        world_trans: acc.world_trans,
                        insts: vec![ModelHashInst {
                            geo_hash: mesh_id.clone(),
                            geo_transform: acc.world_trans,
                            is_tubi: false,
                            unit_flag: false,
                        }],
                        has_neg: true,
                    });
                    continue;
                }

                // 元件库 cata_neg：必须导出布尔结果（CatePos）。缺失时给出明确错误（不要伪装成“缓存缺失/跳过”）。
                if want_has_cata_neg.contains(&want_u64) {
                    missing_cata_bool.push(want_refno);
                    // 清理 acc_map 中的残留条目，避免后续误用/误报。
                    let _ = acc_map.remove(&want_u64);
                    continue;
                }
            }

            match acc_map.remove(&want_u64) {
                Some(acc) if !acc.insts.is_empty() => out.push(GeomInstQuery {
                    refno: want_refno,
                    owner: acc.owner,
                    world_aabb: acc.world_aabb,
                    world_trans: acc.world_trans,
                    insts: acc.insts,
                    has_neg: acc.has_neg,
                }),
                _ => missing.push(want_refno),
            }

            // 追加 tubing world 实例：用 identity world_trans，使导出端 world_trans * inst.geo_transform == inst.geo_transform。
            if let Some(items) = tubi_world_insts.remove(&want_u64) {
                for (owner, wt, aabb, geo_hash) in items {
                    out.push(GeomInstQuery {
                        refno: want_refno,
                        owner,
                        world_aabb: aabb,
                        world_trans: PlantTransform::default(),
                        insts: vec![ModelHashInst {
                            geo_hash,
                            geo_transform: wt,
                            is_tubi: true,
                            unit_flag: false,
                        }],
                        has_neg: false,
                    });
                }
            }
        }
    }

    if !missing_cata_bool.is_empty() {
        missing_cata_bool.sort();
        missing_cata_bool.dedup();
        anyhow::bail!(
            "以下 refno 存在元件库负实体(cata_neg)，但未找到布尔结果缓存(inst_relate_bool=Success)，无法导出 CatePos：{:?}\n\
             处理建议：\n\
             - 确认本次运行启用了布尔运算（apply_boolean_operation=true / 或命令行 --regen-model 已自动开启）\n\
             - 确认缓存布尔 worker 已执行且成功写入 instance_cache 的 inst_relate_bool_map\n\
             - 确认对应 LOD 目录下存在 booled GLB（例如 assets/meshes/lod_L1/<refno>_L1.glb）",
            missing_cata_bool
        );
    }

    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        if verbose {
            println!(
                "⚠️  缓存中未找到以下 refno 的几何实例数据（可能是无几何节点/仅 tubing/或尚未生成），将跳过：{:?}",
                missing
            );
        }
    }

    if verbose {
        println!("✅ 缓存查询几何体数据完成: {} 个几何体组", out.len());
    }
    Ok(out)
}

/// cache-only：从 `FoyerCacheContext` 查询几何实例（导出期）
pub async fn query_geometry_instances_ext(
    ctx: &FoyerCacheContext,
    refnos: &[RefnoEnum],
    enable_holes: bool,
    include_negative: bool,
    verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    query_geometry_instances_ext_from_cache(
        refnos,
        ctx.cache_dir(),
        enable_holes,
        include_negative,
        verbose,
    )
    .await
}
