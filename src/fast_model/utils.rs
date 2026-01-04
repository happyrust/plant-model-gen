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
                let json = format!(
                    "{{'id':aabb:⟨{}⟩, 'd':{}}}",
                    k,
                    serde_json::to_string(v.value()).unwrap()
                );
                sql.push_str(&format!("INSERT IGNORE INTO aabb {};", json));
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
    let mesh_str = mesh_id
        .map(|m| format!("'{}'", m))
        .unwrap_or_else(|| "NONE".to_string());
    let sql = format!(
        "INSERT OR REPLACE INTO inst_relate_bool {{ id: {}, refno: {}, mesh_id: {}, status: '{}', source: '{}', updated_at: time::now() }};",
        refno.to_table_key("inst_relate_bool"),
        refno.to_pe_key(),
        mesh_str,
        status,
        source,
    );

    if let Err(_) = SUL_DB.query(&sql).await {
        init_save_database_error(&sql, &std::panic::Location::caller().to_string());
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
            sql.push_str(&format!(
                "INSERT INTO inst_relate_aabb {{ id: {}, refno: {}, aabb: aabb:⟨{}⟩, source: '{}', updated_at: time::now() }} \
                 ON DUPLICATE KEY UPDATE aabb = aabb:⟨{}⟩, source = '{}', updated_at = time::now();",
                refno.to_table_key("inst_relate_aabb"),
                refno.to_pe_key(),
                aabb_hash.value(),
                source,
                aabb_hash.value(),
                source,
            ));
        }

        if let Err(_) = SUL_DB.query(&sql).await {
            init_save_database_error(&sql, &std::panic::Location::caller().to_string());
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
