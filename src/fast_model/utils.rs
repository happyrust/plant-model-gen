use aios_core::error::init_save_database_error;
use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt};
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;
use std::collections::HashMap;
use tokio::sync::OnceCell;
use tokio::task::JoinSet;

static INST_RELATE_AABB_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();
static INST_RELATE_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

/// 确保 inst_relate_aabb 以“关系表”方式工作：in=pe，out=aabb，且 `in` 唯一。
///
/// 历史遗留：某些数据库中 inst_relate_aabb 曾是普通表（refno/aabb），字段类型可能是必填 record，
/// 仅写 in/out 会触发类型强制失败。这里会清理旧字段定义/索引，并清空旧数据，保证新结构稳定写入。
pub async fn ensure_inst_relate_aabb_relation_schema() {
    INST_RELATE_AABB_SCHEMA_INIT
        .get_or_init(|| async {
            // 目标：必须是 RELATION 表，否则 rs-core 的 `in->inst_relate_aabb` 图遍历拿不到 out.d，
            // 导致 export-glb 等流程 world_aabb = null 并反序列化失败。
            //
            // 用户已确认允许清理旧数据，因此直接删除并重建为 RELATION 表（最稳）。
            let _ = SUL_DB.query("REMOVE TABLE inst_relate_aabb;").await;

            let _ = SUL_DB
                .query("DEFINE TABLE inst_relate_aabb TYPE RELATION;")
                .await;

            // TYPE RELATION 会隐式创建 in/out 字段，但默认 TYPE record；这里显式改为更严格的类型。
            let _ = SUL_DB
                .query("REMOVE FIELD in ON TABLE inst_relate_aabb;")
                .await;
            let _ = SUL_DB
                .query("REMOVE FIELD out ON TABLE inst_relate_aabb;")
                .await;
            let _ = SUL_DB
                .query("REMOVE FIELD refno ON TABLE inst_relate_aabb;")
                .await;
            let _ = SUL_DB
                .query("DEFINE FIELD in ON TABLE inst_relate_aabb TYPE record<pe>;")
                .await;
            let _ = SUL_DB
                .query("DEFINE FIELD out ON TABLE inst_relate_aabb TYPE record<aabb>;")
                .await;
            let _ = SUL_DB
                .query(
                    "DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb FIELDS in UNIQUE;",
                )
                .await;
        })
        .await;
}

/// 确保 inst_relate 以“关系表”方式工作：in=pe，out=inst_info。
///
/// 需要重建 inst_relate，保证旧的普通表结构不影响图查询与复用逻辑。
pub async fn ensure_inst_relate_relation_schema() {
    INST_RELATE_SCHEMA_INIT
        .get_or_init(|| async {
            let _ = SUL_DB.query("REMOVE TABLE inst_relate;").await;

            let _ = SUL_DB
                .query("DEFINE TABLE inst_relate TYPE RELATION;")
                .await;

            // TYPE RELATION 会隐式创建 in/out 字段，但默认 TYPE record；这里显式改为更严格的类型。
            let _ = SUL_DB.query("REMOVE FIELD in ON TABLE inst_relate;").await;
            let _ = SUL_DB.query("REMOVE FIELD out ON TABLE inst_relate;").await;
            let _ = SUL_DB
                .query("DEFINE FIELD in ON TABLE inst_relate TYPE record<pe>;")
                .await;
            let _ = SUL_DB
                .query("DEFINE FIELD out ON TABLE inst_relate TYPE record<inst_info>;")
                .await;
            let _ = SUL_DB
                .query("DEFINE INDEX idx_inst_relate_in ON TABLE inst_relate FIELDS in UNIQUE;")
                .await;
            let _ = SUL_DB
                .query("DEFINE INDEX idx_inst_relate_out ON TABLE inst_relate FIELDS out;")
                .await;
        })
        .await;
}

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
                let id_key = if k.starts_with("aabb:") {
                    k.to_string()
                } else {
                    format!("aabb:⟨{}⟩", k)
                };
                sql.push_str(&format!("UPSERT {id_key} SET d = {d};"));
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
    _source: &str,
) {
    ensure_inst_relate_aabb_relation_schema().await;

    if inst_aabb_map.is_empty() {
        return;
    }

    let keys = inst_aabb_map
        .iter()
        .map(|kv| kv.key().clone())
        .collect::<Vec<_>>();

    for chunk in keys.chunks(200) {
        let mut sql = String::new();
        let mut in_keys = Vec::new();
        for refno in chunk {
            let Some(aabb_hash) = inst_aabb_map.get(refno) else { continue };
            let refno_key = refno.to_pe_key();
            in_keys.push(refno_key.clone());
            let aabb_key = {
                let v = aabb_hash.value();
                if v.starts_with("aabb:") {
                    v.to_string()
                } else {
                    format!("aabb:⟨{}⟩", v)
                }
            };
            sql.push_str(&format!(
                "RELATE {refno_key}->inst_relate_aabb->{aabb_key};"
            ));
        }

        if !in_keys.is_empty() {
            sql.insert_str(
                0,
                &format!(
                    "DELETE FROM inst_relate_aabb WHERE in IN [{}];",
                    in_keys.join(",")
                ),
            );
        }

        if sql.is_empty() {
            continue;
        }

        if let Err(e) = SUL_DB.query_take::<surrealdb::types::Value>(&sql, 0).await {
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
