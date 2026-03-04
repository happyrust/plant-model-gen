use crate::fast_model::query_provider;
use crate::fast_model::utils::save_transforms_to_surreal;
use aios_core::{RefU64, RefnoEnum, project_primary_db, gen_plant_transform_hash};
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
use aios_core::{query_neareast_along_axis, query_neareast_by_pos_dir};

use aios_core::Transform;
use glam::Vec3;
use parry3d::bounding_volume::Aabb;
use parry3d::bounding_volume::BoundingVolume;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

pub async fn update_cal_equip() -> anyhow::Result<()> {
    update_cal_equip_wtrans().await?;
    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
    cal_equip_nearest_floor().await?;
    Ok(())
}

//将equip的 world transform 这些放到cal_equip 这张表里，存储一些需要计算的缓存数据
//这样可以减少查询次数，提高性能
//这个表的数据是在equip的数据变更时，由equip的数据变更触发的
pub async fn update_cal_equip_wtrans() -> anyhow::Result<()> {
    let mut response = project_primary_db()
        .query(format!(
            r#"select value id from {} where type::record("cal_equi", record::id(id))!=none"#,
            "EQUI"
        ))
        .await?;
    let equips: Vec<RefnoEnum> = response.take(0)?;
    if equips.is_empty() {
        return Ok(());
    }
    let mut transform_map: HashMap<u64, String> = HashMap::new();
    transform_map.insert(0, serde_json::to_string(&Transform::IDENTITY).unwrap());
    let mut sql = String::new();
    for refno in equips {
        let world_trans =
            match crate::fast_model::transform_cache::get_world_transform_cache_first(
                None,
                refno,
            )
            .await
            {
                Ok(transform) => transform.unwrap_or_default(),
                Err(e) => {
                    eprintln!(
                        "[cal_equip] 获取 world_transform 失败, refno={} 错误: {}",
                        refno.to_normal_str(),
                        e
                    );
                    return Err(e);
                }
            };
        let transform_hash = gen_plant_transform_hash(&world_trans);
        if !transform_map.contains_key(&transform_hash) {
            transform_map.insert(transform_hash, serde_json::to_string(&world_trans).unwrap());
        }
        sql.push_str(&format!(
            "create cal_equi:{refno} SET world_trans=trans:⟨{}⟩;",
            transform_hash
        ));
    }
    save_transforms_to_surreal(&transform_map).await?;
    project_primary_db().query(sql).await.unwrap();
    Ok(())
}

//取得设备下的所有aabb，然后取下面的点到楼板的最近距离
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
pub async fn cal_equip_nearest_floor() -> anyhow::Result<()> {
    let mut response = project_primary_db()
        .query(format!(r#"select value record::id(id) from {} where type::record("cal_equi", record::id(id))!=none"#, "EQUI"))
        .await?;
    let equips: Vec<RefnoEnum> = response.take(0)?;
    if equips.is_empty() {
        return Ok(());
    }

    let mut equip_sql = String::new();
    for equip in equips {
        println!("[cal_equip] 处理设备 {}", equip.to_normal_str());

        // 已经计算过 nearest_relate 的设备直接跳过
        let has_nearest_sql = format!(
            "SELECT VALUE array::len(->nearest_relate) FROM {} LIMIT 1",
            equip.to_pe_key()
        );
        let has_nearest: Vec<i64> = project_primary_db().query_take(&has_nearest_sql, 0).await.unwrap_or_default();
        if has_nearest.first().copied().unwrap_or(0) != 0 {
            continue;
        }

        // 使用 TreeIndex 查询获取子孙节点（保持与旧实现一致：只取 1..2 层）
        let provider = query_provider::get_model_query_provider().await?;
        let mut target_refnos = provider
            .get_descendants_filtered(equip, &[], Some(2))
            .await
            .unwrap_or_default();
        target_refnos.retain(|r| *r != equip);
        if target_refnos.is_empty() {
            continue;
        }

        // 批量从 inst_relate 中取对应的 inst_relate_aabb(out) 的 AABB 数据
        let pe_keys = target_refnos
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT VALUE out.d \
             FROM [{pe_keys}]->inst_relate_aabb \
             WHERE out.d != none"
        );
        let raw_values: Vec<JsonValue> = project_primary_db().query_take(&sql, 0).await.unwrap_or_default();
        let Ok(aabbs) = raw_values
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<Aabb>, _>>()
        else {
            continue;
        };
        if aabbs.is_empty() {
            continue;
        }
        let mut final_aabb = Aabb::new_invalid();
        for aabb in aabbs {
            final_aabb.merge(&aabb);
        }
        //得到底部的中心点，去计算最近的楼板
        let btm_pts = &final_aabb.vertices()[..4];
        for btm_pt in btm_pts {
            let pt: Vec3 = (*btm_pt).into();
            if let Ok(Some((nearest, dist))) =
                query_neareast_by_pos_dir(pt, Vec3::NEG_Z, "FLOOR").await
            {
                // dbg!((btm_pt, nearest, dist));
                equip_sql.push_str(&format!(
                    "relate {}->nearest_relate->{} set dist={};",
                    equip.to_pe_key(),
                    nearest.to_pe_key(),
                    dist
                ));
                break;
            }
        }
    }
    if !equip_sql.is_empty() {
        project_primary_db().query(equip_sql).await.unwrap();
    }
    Ok(())
}
