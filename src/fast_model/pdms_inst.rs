use std::collections::{HashMap, hash_map::Entry};

use aios_core::geometry::ShapeInstancesData;
use aios_core::parsed_data::TubiInfoData;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::*;
use aios_core::rs_surreal::delete_inst_relate_cascade;
use aios_core::types::*;
use aios_core::{SUL_DB, SurrealQueryExt, get_db_option};
use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;
use bevy_transform::prelude::Transform;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use itertools::Itertools;
use rkyv::vec;
use tokio::task::JoinHandle;

use crate::data_interface::tidb_manager::AiosDBManager;
use crate::fast_model::debug_model_debug;
use crate::fast_model::utils;
// use crate::fast_model::EXIST_MESH_GEOS;

/// 保存 instance 数据到数据库（事务化批处理版本）
pub async fn save_instance_data_optimize(
    inst_mgr: &ShapeInstancesData,
    replace_exist: bool,
) -> anyhow::Result<()> {

    debug_model_debug!(
        "save_instance_data_optimize start: inst_info={}, inst_geo_keys={}, tubi_keys={}, replace_exist={}",
        inst_mgr.inst_info_map.len(),
        inst_mgr.inst_geos_map.len(),
        inst_mgr.inst_tubi_map.len(),
        replace_exist
    );

    // 单条 INSERT 里拼接的记录数，过大容易触发 SurrealDB 事务取消/超时；取小一点更稳。
    const CHUNK_SIZE: usize = 100;
    // SurrealDB 在高并发/大事务时容易出现 session 丢失、匿名访问等错误；这里优先保证稳定性。
    const MAX_TX_STATEMENTS: usize = 5;
    const MAX_CONCURRENT_TX: usize = 5;

    // 统一迁移/修复 inst_relate_aabb 的历史 schema（refno/aabb -> in/out），避免写入时触发类型强制失败
    utils::ensure_inst_relate_aabb_relation_schema().await;

    let mut aabb_map: HashMap<u64, String> = HashMap::new();
    let mut transform_map: HashMap<u64, String> = HashMap::new();
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
    if replace_exist {
        // replace 模式下需要确保 inst_info_map 对应的 inst_relate 都被级联删除，
        // 否则会出现同一 inst_relate ID 已存在但 out 指向不同 inst_info 的冲突（SurrealDB 的 in/out 不可变）。
        // 注意：inst_tubi_map 不再创建 inst_relate（tubing 使用 tubi_relate），所以不需要删除
        let refnos: Vec<RefnoEnum> = inst_mgr.inst_info_map.keys().copied().collect();
        debug_model_debug!(
            "save_instance_data_optimize deleting existing inst_relate for {} refnos",
            refnos.len()
        );
        delete_inst_relate_cascade(&refnos, CHUNK_SIZE).await?;
    }

    // inst_geo & geo_relate
    let mut geo_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
    let mut inst_geo_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);
    let mut geo_relate_buffer: Vec<String> = Vec::with_capacity(CHUNK_SIZE);

    for inst_geo_data in inst_mgr.inst_geos_map.values() {
        for inst in &inst_geo_data.insts {
            if inst.transform.translation.is_nan()
                || inst.transform.rotation.is_nan()
                || inst.transform.scale.is_nan()
            {
                debug_model_debug!(
                    "[WARN] skip inst geo due to NaN transform: refno={:?}, geo_hash={}",
                    inst.refno,
                    inst.geo_hash
                );
                continue;
            }

            let transform_hash = gen_bytes_hash(&inst.transform);
            if let Entry::Vacant(entry) = transform_map.entry(transform_hash) {
                entry.insert(serde_json::to_string(&inst.transform)?);
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
            let relate_id = gen_bytes_hash(&relate_json);
            geo_relate_buffer.push(format!("{{ {relate_json}, id: '{relate_id}' }}"));

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

    // tubi -> aabb & transform maps
    for tubi in inst_mgr.inst_tubi_map.values() {
        if let Some(aabb) = tubi.aabb {
            let aabb_hash = gen_bytes_hash(&aabb);
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }
        }

        let transform_hash = gen_bytes_hash(&tubi.world_transform);
        if let Entry::Vacant(entry) = transform_map.entry(transform_hash) {
            entry.insert(serde_json::to_string(&tubi.world_transform)?);
        }
    }

    // neg_relate - 新结构
    // 关系方向：切割几何 -[neg_relate]-> 正实体
    // - in: geo_relate ID (切割几何)
    // - out: 正实体 refno (被减实体)
    // - pe: 负实体 refno (负载体，原来的 in)
    if !inst_mgr.neg_relate_map.is_empty() {
        println!("🔍 [DEBUG] 开始创建 neg_relate 关系 (新结构: in=geo_relate):");
        for (target, refnos) in &inst_mgr.neg_relate_map {
            println!("  目标: {}, 负实体数量: {}", target, refnos.len());
        }

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

                        if neg_buffer.len() >= CHUNK_SIZE {
                            let statement = format!(
                                "INSERT RELATION INTO neg_relate [{}];",
                                neg_buffer.join(",")
                            );
                            neg_batcher.push(statement).await?;
                            neg_buffer.clear();
                        }
                    }
                } 
            }
        }

        if !neg_buffer.is_empty() {
            let statement = format!(
                "INSERT RELATION INTO neg_relate [{}];",
                neg_buffer.join(",")
            );
            neg_batcher.push(statement).await?;
        }

        neg_batcher.finish().await?;
    }

    // ngmr_relate - 新结构
    // 关系方向：切割几何 -[ngmr_relate]-> 正实体
    // - in: geo_relate ID (CataCrossNeg 切割几何)
    // - out: 目标k (正实体)
    // - pe: ele_refno (负载体，原来的 in)
    // - ngmr: ngmr_geom_refno (NGMR 几何引用，保留用于调试)
    if !inst_mgr.ngmr_neg_relate_map.is_empty() {
        println!("🔍 [DEBUG] 开始创建 ngmr_relate 关系 (新结构: in=geo_relate):");
        for (k, refnos) in &inst_mgr.ngmr_neg_relate_map {
            println!("  目标: {}, NGMR 数量: {}", k, refnos.len());
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

                        if ngmr_buffer.len() >= CHUNK_SIZE {
                            let statement = format!(
                                "INSERT RELATION INTO ngmr_relate [{}];",
                                ngmr_buffer.join(",")
                            );
                            ngmr_batcher.push(statement).await?;
                            ngmr_buffer.clear();
                        }
                    }
                }
            }
        }

        if !ngmr_buffer.is_empty() {
            let statement = format!(
                "INSERT RELATION INTO ngmr_relate [{}];",
                ngmr_buffer.join(",")
            );
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

        if info.world_transform.translation.is_nan()
            || info.world_transform.rotation.is_nan()
            || info.world_transform.scale.is_nan()
        {
            continue;
        }

        // 使用压缩格式存储 ptset（减少约 70-80% 存储空间）
        inst_info_buffer.push(info.gen_sur_json_compact(false));
        if inst_info_buffer.len() >= CHUNK_SIZE {
            let statement = format!(
                "INSERT IGNORE INTO {} [{}];",
                stringify!(inst_info),
                inst_info_buffer.join(",")
            );
            inst_info_batcher.push(statement).await?;
            inst_info_buffer.clear();
        }

        let transform_hash = gen_bytes_hash(&info.world_transform);
        if let Entry::Vacant(entry) = transform_map.entry(transform_hash) {
            entry.insert(serde_json::to_string(&info.world_transform)?);
        }

        if let Some(aabb) = info.aabb {
            let aabb_hash = gen_bytes_hash(&aabb);
            if let Entry::Vacant(entry) = aabb_map.entry(aabb_hash) {
                entry.insert(serde_json::to_string(&aabb)?);
            }

            // inst_relate_aabb 为关系表：in=pe, out=aabb（只存关系，不存其他字段）
            // 使用批量 DELETE + INSERT RELATION 做幂等更新
            let aabb_row_sql = format!(
                "{{id: {0}, in: {1}, out: aabb:⟨{2}⟩}}",
                key.to_table_key("inst_relate_aabb"),
                key.to_pe_key(),
                aabb_hash
            );
            inst_relate_aabb_buffer.push(aabb_row_sql);
            inst_relate_aabb_ins.push(key.to_pe_key());
        }

        // inst_relate 不再保存 world_trans；世界变换统一从 pe_transform 获取。
        let relate_sql = format!(
            "{{id: {0}, in: {1}, out: inst_info:⟨{2}⟩, generic: '{3}', zone_refno: fn::find_ancestor_type({1}, 'ZONE'), spec_value: (fn::find_ancestor_type({1}, 'ZONE').owner.spec_value) ?? 0, dt: fn::ses_date({1}), has_cata_neg: {4}, solid: {5}, owner_refno: {6}, owner_type: '{7}'}}",
            key.to_inst_relate_key(),
            key.to_pe_key(),
            info.id_str(),
            info.generic_type.to_string(),
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

            // 延后处理 inst_relate_aabb（必须在 aabb UPSERT 之后写关系，避免 out 侧空记录 d=NONE）
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

    // 注意：inst_relate_aabb(out) 指向 aabb 表的记录。
    // 若先写关系再写 aabb 内容，SurrealDB 可能会"隐式创建"空的 aabb 记录（d = NONE）。
    // 这里把 inst_relate_aabb 的写入延后到 aabb UPSERT 之后，保证 out 侧不会出现空记录。

    // inst_tubi_map 不再创建 inst_relate（tubing 使用专门的 tubi_relate 表）
    // 只收集 transform 和 aabb 数据用于其他用途
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

            let transform_hash = gen_bytes_hash(&info.world_transform);
            if let Entry::Vacant(entry) = transform_map.entry(transform_hash) {
                entry.insert(serde_json::to_string(&info.world_transform)?);
            }

            // 收集 aabb 数据（用于 tubi_relate）
            if let Some(aabb) = info.aabb {
                let aabb_hash = gen_bytes_hash(&aabb);
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

    // 调试：确认 inst_relate 是否已写入数据库
    if !inst_keys.is_empty() {
        let pe_list = inst_keys.iter().map(|k| k.to_pe_key()).join(",");
        let verify_sql = format!(
            "SELECT count() AS cnt FROM inst_relate WHERE in IN [{}];",
            pe_list
        );
        match SUL_DB.query_response(&verify_sql).await {
            Ok(mut resp) => match resp.take::<Vec<serde_json::Value>>(0) {
                Ok(counts) => debug_model_debug!(
                    "🔍 [DEBUG] inst_relate verify counts for [{}]: {:?}",
                    pe_list,
                    counts
                ),
                Err(err) => debug_model_debug!(
                    "❌ [DEBUG] inst_relate verify take failed (sql: {}): {}",
                    verify_sql,
                    err
                ),
            },
            Err(e) => {
                debug_model_debug!(
                    "❌ [DEBUG] inst_relate verify query failed (sql: {}): {}",
                    verify_sql,
                    e
                );
            }
        }
    }

    debug_model_debug!("🔍 [DEBUG] Finishing inst_info_batcher...");
    inst_info_batcher.finish().await?;
    debug_model_debug!("✅ [DEBUG] inst_info_batcher finished successfully");

    // aabb
    if !aabb_map.is_empty() {
        let mut aabb_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);
        let mut sql_buffer = String::new();
        let mut buffered = 0usize;

        for (&hash, value) in &aabb_map {
            // 注意：aabb 记录可能先被 inst_relate_aabb(out) 侧“隐式创建”为一个空记录（d = NONE）。
            // 这里必须用 UPSERT 覆盖/补齐 d，不能用 INSERT IGNORE。
            sql_buffer.push_str(&format!("UPSERT aabb:⟨{}⟩ SET d = {};", hash, value));
            buffered += 1;
            if buffered >= CHUNK_SIZE {
                aabb_batcher.push(std::mem::take(&mut sql_buffer)).await?;
                buffered = 0;
            }
        }

        if !sql_buffer.is_empty() {
            aabb_batcher.push(sql_buffer).await?;
        }

        aabb_batcher.finish().await?;
    }

    // inst_relate_aabb（关系表：in=pe, out=aabb），必须在 aabb UPSERT 之后写入，避免 out 侧空记录（d = NONE）
    if !inst_relate_aabb_chunks.is_empty() || !inst_relate_aabb_buffer.is_empty() {
        let mut inst_aabb_batcher = TransactionBatcher::new(MAX_TX_STATEMENTS, MAX_CONCURRENT_TX);

        // 统一把积累的 chunks + 剩余 buffer 一次性落库
        let mut total = 0usize;
        macro_rules! flush_pairs {
            ($rows:expr, $ins:expr) => {{
                let n = ($ins).len().min(($rows).len());
                if n > 0 {
                    for idx in (0..n).step_by(CHUNK_SIZE) {
                        let end = (idx + CHUNK_SIZE).min(n);
                        let delete_stmt = format!(
                            "DELETE inst_relate_aabb WHERE in IN [{}];",
                            ($ins)[idx..end].join(",")
                        );
                        let insert_stmt = format!(
                            "INSERT RELATION INTO inst_relate_aabb [{}];",
                            ($rows)[idx..end].join(",")
                        );
                        inst_aabb_batcher.push(delete_stmt).await?;
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
            "save_instance_data_optimize flushing inst_relate_aabb after aabb upsert: {}",
            total
        );
        inst_aabb_batcher.finish().await?;
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
        let db = SUL_DB.clone();
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

            match db.query(query.clone()).await {
                Ok(mut resp) => {
                    if let Err(e) = take_all_results_or_err!(resp) {
                        // 某些情况下 inst_relate_aabb 的唯一索引可能“脏”了（表里查不到记录但索引仍占用值），
                        // 这会导致所有 INSERT 失败并连带回滚同一事务块（inst_relate 也写不进去）。
                        let es = e.to_string();
                        let is_inst_relate_aabb_unique_conflict =
                            es.contains("idx_inst_relate_aabb_refno") && es.contains("already contains");

                        if is_inst_relate_aabb_unique_conflict {
                            debug_model_debug!(
                                "⚠️ [DEBUG] 检测到 inst_relate_aabb 唯一索引冲突，尝试重建索引并重试一次..."
                            );
                            let repair_sql = "REMOVE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb; \
DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb FIELDS in UNIQUE;";
                            let _ = db.query(repair_sql).await;

                            match db.query(query).await {
                                Ok(mut resp2) => take_all_results_or_err!(resp2).map_err(|e2| {
                                    debug_model_debug!(
                                        "❌ [DEBUG] TransactionBatcher retry failed: {}\n--- transaction block ---\n{}",
                                        e2,
                                        debug_query
                                    );
                                    e2
                                }),
                                Err(err2) => {
                                    debug_model_debug!(
                                        "❌ [DEBUG] TransactionBatcher retry query error: {}\n--- transaction block ---\n{}",
                                        err2,
                                        debug_query
                                    );
                                    Err(anyhow::Error::from(err2))
                                }
                            }
                        } else {
                            debug_model_debug!(
                                "❌ [DEBUG] TransactionBatcher statement error: {}\n--- transaction block ---\n{}",
                                e,
                                debug_query
                            );
                            Err(e)
                        }
                    } else {
                        Ok(())
                    }
                }
                Err(err) => {
                    debug_model_debug!(
                        "❌ [DEBUG] TransactionBatcher query error: {}\n--- transaction block ---\n{}",
                        err,
                        debug_query
                    );
                    Err(anyhow::Error::from(err))
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
        let values: Vec<String> = chunk
            .iter()
            .map(|e| e.value().to_surreal_json())
            .collect();
        
        let sql = format!("INSERT INTO tubi_info [{}];", values.join(","));
        SUL_DB.query(&sql).await?;
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
        
        let sql = format!(
            "SELECT VALUE record::id(id) FROM tubi_info WHERE id IN [{}];",
            id_list
        );
        
        let result: Vec<String> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        existing.extend(result);
    }
    
    Ok(existing)
}
