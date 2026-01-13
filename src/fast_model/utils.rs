use aios_core::error::init_save_database_error;
use aios_core::{RefnoEnum, SUL_DB};
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;
use std::collections::HashMap;
use tokio::task::JoinSet;

pub async fn save_aabb_to_surreal(aabb_map: &DashMap<String, Aabb>) {
    if !aabb_map.is_empty() {
        let keys = aabb_map
            .iter()
            .map(|kv| kv.key().clone())
            .collect::<Vec<_>>();
        for chunk in keys.chunks(300) {
            let mut sql = "".to_string();
            for k in chunk {
                let v = aabb_map.get(k).unwrap();
                // 注意：aabb 记录可能先被 RELATE out 侧“隐式创建”为一个空记录（d = NONE）。
                // 这里必须用 UPSERT 覆盖/补齐 d，不能用 INSERT IGNORE。
                let d = serde_json::to_string(v.value()).unwrap();
                sql.push_str(&format!("UPSERT aabb:⟨{}⟩ SET d = {};", k, d));
            }
            match SUL_DB.query(&sql).await {
                Ok(_) => {}
                Err(_) => {
                    init_save_database_error(&sql, &std::panic::Location::caller().to_string());
                }
            }
        }
    }
}

/// 保存布尔结果状态
pub async fn save_inst_relate_bool(
    refno: RefnoEnum,
    mesh_id: Option<&str>,
    status: &str,
    source: &str,
) {
    // SurrealQL：使用 UPSERT 保证幂等写入（SurrealDB 不支持 “INSERT OR REPLACE”）
    let refno_str = refno.to_string();
    let id_key = format!("inst_relate_bool:⟨{}⟩", refno_str);
    // inst_relate_bool.refno 约定为 pe 记录引用（与 surreal_schema.sql 一致）
    let refno_key = format!("pe:⟨{}⟩", refno_str);
    let mesh_str = mesh_id.map(|m| format!("'{}'", m)).unwrap_or_else(|| "NONE".to_string());
    let sql = format!(
        "UPSERT {id_key} CONTENT {{ refno: {refno_key}, mesh_id: {mesh_str}, status: '{status}', source: '{source}', updated_at: time::now() }};",
    );

    if let Err(e) = SUL_DB.query(&sql).await {
        init_save_database_error(
            &format!("{sql}\n-- err: {e}"),
            &std::panic::Location::caller().to_string(),
        );
    }
}

/// 保存 catalog 级布尔结果状态（与实例级布尔分表，避免互相覆盖）
pub async fn save_inst_relate_cata_bool(
    refno: RefnoEnum,
    mesh_id: Option<&str>,
    status: &str,
    source: &str,
) {
    let refno_str = refno.to_string();
    let id_key = format!("inst_relate_cata_bool:⟨{}⟩", refno_str);
    let refno_key = format!("pe:⟨{}⟩", refno_str);
    let mesh_str = mesh_id
        .map(|m| format!("'{}'", m))
        .unwrap_or_else(|| "NONE".to_string());
    let sql = format!(
        "UPSERT {id_key} CONTENT {{ refno: {refno_key}, mesh_id: {mesh_str}, status: '{status}', source: '{source}', updated_at: time::now() }};",
    );

    if let Err(e) = SUL_DB.query(&sql).await {
        init_save_database_error(
            &format!("{sql}\n-- err: {e}"),
            &std::panic::Location::caller().to_string(),
        );
    }
}

/// 批量保存实例 AABB 到独立表 inst_relate_aabb
pub async fn save_inst_relate_aabb(
    inst_aabb_map: &DashMap<RefnoEnum, String>,
    source: &str,
) {
    if inst_aabb_map.is_empty() {
        return;
    }

    let keys = inst_aabb_map
        .iter()
        .map(|kv| kv.key().clone())
        .collect::<Vec<_>>();

    for chunk in keys.chunks(200) {
        let mut sql = String::new();
        for refno in chunk {
            let Some(aabb_hash) = inst_aabb_map.get(refno) else { continue };
            // SurrealQL：inst_relate_aabb 是关系表，使用 RELATE 明确写入 in/out。
            // 为了幂等，这里先按固定 id 删除旧关系再重建。
            let refno_str = refno.to_string();
            let in_key = format!("inst_relate:⟨{}⟩", refno_str);
            // 兼容两种输入：
            // - 计算路径：aabb_hash 是纯 hash（如 "754..."）
            // - 回退路径：aabb_hash 可能是完整 record 字符串（如 "aabb:⟨754...⟩"）
            let out_key = {
                let v = aabb_hash.value();
                if v.starts_with("aabb:") {
                    v.to_string()
                } else {
                    format!("aabb:⟨{}⟩", v)
                }
            };
            let edge_key = format!("inst_relate_aabb:⟨{}⟩", refno_str);
            let refno_key = format!("pe:⟨{}⟩", refno_str);

            sql.push_str(&format!(
                "DELETE {edge_key}; RELATE {in_key}->{edge_key}->{out_key} SET refno = {refno_key}, aabb = {out_key}, source = '{source}', updated_at = time::now();"
            ));
        }

        if let Err(e) = SUL_DB.query(&sql).await {
            init_save_database_error(
                &format!("{sql}\n-- err: {e}"),
                &std::panic::Location::caller().to_string(),
            );
        }
    }
}

pub async fn save_pts_to_surreal(vec3_map: &DashMap<u64, String>) {
    if !vec3_map.is_empty() {
        let keys = vec3_map.iter().map(|kv| *kv.key()).collect::<Vec<_>>();
        for chunk in keys.chunks(100) {
            let mut sql = "".to_string();
            for &k in chunk {
                let v = vec3_map.get(&k).unwrap();
                let json = format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", k, v.value());
                sql.push_str(&format!("INSERT IGNORE INTO vec3 {};", json));
            }
            match SUL_DB.query(&sql).await {
                Ok(_) => {}
                Err(_e) => {
                    init_save_database_error(&sql, &std::panic::Location::caller().to_string());
                }
            };
        }
    }
}

pub async fn save_transforms_to_surreal(trans_map: &HashMap<u64, String>) -> anyhow::Result<()> {
    if !trans_map.is_empty() {
        let keys = trans_map.keys().collect::<Vec<_>>();
        for chunk in keys.chunks(100) {
            let mut sql = "".to_string();
            for &k in chunk {
                let v = trans_map.get(&k).unwrap();
                let json = format!("{{'id':trans:⟨{}⟩, 'd':{}}}", k, v);
                sql.push_str(&format!("INSERT IGNORE INTO trans {};", json));
            }
            SUL_DB.query(sql).await.unwrap();
        }
    }
    Ok(())
}
