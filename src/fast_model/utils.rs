use aios_core::error::init_save_database_error;
use aios_core::{RefnoEnum, SurrealQueryExt, model_primary_db};
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;
use std::collections::HashMap;
use tokio::sync::OnceCell;
use tokio::task::JoinSet;

static SURREAL_INIT: OnceCell<()> = OnceCell::const_new();
static INST_RELATE_AABB_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();
static INST_RELATE_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

/// 确保 SurrealDB 连接已初始化（幂等，仅首次真正执行 `init_surreal`）。
///
/// `aios_core::init_surreal()` 每次调用都会尝试 connect + signin + use_ns_db，
/// 在并发 spawn task 中多次调用会导致 WebSocket 连接竞争/死锁。
/// 此函数用 `OnceCell` 保证只执行一次。
pub async fn ensure_surreal_init() -> anyhow::Result<()> {
    SURREAL_INIT
        .get_or_try_init(|| async {
            aios_core::init_surreal().await
        })
        .await?;
    Ok(())
}

/// 确保 inst_relate_aabb 以"关系表"方式工作：in=pe，out=aabb，且 `in` 唯一。
pub async fn ensure_inst_relate_aabb_relation_schema() {
    INST_RELATE_AABB_SCHEMA_INIT
        .get_or_init(|| async {
            // 检查表是否已经是 RELATION 类型：尝试查询一条带 in/out 的记录。
            // 如果 INFO FOR TABLE 返回的定义中包含 "RELATION"，说明已迁移过，跳过重建。
            let already_relation = match model_primary_db()
                .query("INFO FOR TABLE inst_relate_aabb;")
                .await
            {
                Ok(mut resp) => {
                    // INFO FOR TABLE 返回一个 object，检查其中是否包含 RELATION 标记
                    match resp.take::<Option<serde_json::Value>>(0) {
                        Ok(Some(val)) => {
                            let s = val.to_string();
                            s.contains("RELATION")
                        }
                        _ => false,
                    }
                }
                Err(_) => false,
            };

            if !already_relation {
                // 表不存在或不是 RELATION 类型，重建为 RELATION
                let _ = model_primary_db().query("REMOVE TABLE inst_relate_aabb;").await;
                let _ = model_primary_db()
                    .query("DEFINE TABLE inst_relate_aabb TYPE RELATION;")
                    .await;
            }
            let _ = model_primary_db()
                .query("DEFINE FIELD in ON TABLE inst_relate_aabb TYPE record<pe>;")
                .await;
            let _ = model_primary_db()
                .query("DEFINE FIELD out ON TABLE inst_relate_aabb TYPE record<aabb>;")
                .await;
            let _ = model_primary_db()
                .query(
                    "DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb FIELDS in UNIQUE;",
                )
                .await;
        })
        .await;
}

/// 确保 inst_relate 以“关系表”方式工作：in=pe，out=inst_info。
pub async fn ensure_inst_relate_relation_schema() {
    INST_RELATE_SCHEMA_INIT
        .get_or_init(|| async {
            let _ = model_primary_db().query("REMOVE TABLE inst_relate;").await;

            let _ = model_primary_db()
                .query("DEFINE TABLE inst_relate TYPE RELATION;")
                .await;

            // TYPE RELATION 会隐式创建 in/out 字段，但默认 TYPE record；这里显式改为更严格的类型。
            let _ = model_primary_db().query("REMOVE FIELD in ON TABLE inst_relate;").await;
            let _ = model_primary_db().query("REMOVE FIELD out ON TABLE inst_relate;").await;
            let _ = model_primary_db()
                .query("DEFINE FIELD in ON TABLE inst_relate TYPE record<pe>;")
                .await;
            let _ = model_primary_db()
                .query("DEFINE FIELD out ON TABLE inst_relate TYPE record<inst_info>;")
                .await;
            let _ = model_primary_db()
                .query("DEFINE INDEX idx_inst_relate_in ON TABLE inst_relate FIELDS in UNIQUE;")
                .await;
            let _ = model_primary_db()
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
            let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
            for k in chunk {
                let v = aabb_map.get(k).unwrap();
                let d = serde_json::to_string(v.value()).unwrap();
                let id_key = if k.starts_with("aabb:") {
                    k.to_string()
                } else {
                    format!("aabb:⟨{}⟩", k)
                };
                rows.push(format!("{{'id':{id_key}, 'd':{d}}}"));
            }
            let sql = format!("INSERT IGNORE INTO aabb [{}];", rows.join(","));
            match model_primary_db().query(&sql).await {
                Ok(_) => {}
                Err(_) => {
                    init_save_database_error(&sql, &std::panic::Location::caller().to_string());
                }
            }
        }
    }
}

/// 保存布尔结果状态
///
/// 修复(RUS-182)：改为返回 Result，让调用方能感知写入失败。
pub async fn save_inst_relate_bool(
    refno: RefnoEnum,
    mesh_id: Option<&str>,
    status: &str,
    source: &str,
) -> anyhow::Result<()> {
    // SurrealQL：使用 UPSERT 保证幂等写入（SurrealDB 不支持 "INSERT OR REPLACE"）
    let refno_str = refno.to_string();
    let id_key = format!("inst_relate_bool:⟨{}⟩", refno_str);
    // inst_relate_bool.refno 约定为 pe 记录引用（与 surreal_schema.sql 一致）
    let refno_key = format!("pe:⟨{}⟩", refno_str);
    let mesh_str = mesh_id.map(|m| format!("'{}'", m)).unwrap_or_else(|| "NONE".to_string());
    let sql = format!(
        "UPSERT {id_key} CONTENT {{ refno: {refno_key}, mesh_id: {mesh_str}, status: '{status}', source: '{source}', updated_at: time::now() }};",
    );

    if let Err(e) = aios_core::model_query_response(&sql).await {
        let msg = format!("{sql}\n-- err: {e}");
        init_save_database_error(
            &msg,
            &std::panic::Location::caller().to_string(),
        );
        anyhow::bail!("save_inst_relate_bool 失败: refno={refno} err={e}");
    }
    Ok(())
}

/// 保存 catalog 级布尔结果状态（与实例级布尔分表，避免互相覆盖）
pub async fn save_inst_relate_cata_bool(
    refno: RefnoEnum,
    mesh_id: Option<&str>,
    status: &str,
    source: &str,
) {
    let refno_key = refno.to_pe_key();
    let mut sql = format!(
        "LET $inst_info = (SELECT VALUE out FROM {refno_key}->inst_relate LIMIT 1)[0];"
    );

    // 始终先删除旧记录，保证每个 inst_info 仅保留一条最新状态关系。
    sql.push_str(
        "IF $inst_info != NONE { LET $old_ids = SELECT VALUE id FROM inst_relate_cata_bool WHERE in = $inst_info; DELETE $old_ids;",
    );
    if let Some(mesh_id) = mesh_id {
        let mesh_key = format!("inst_geo:⟨{}⟩", mesh_id);
        sql.push_str(&format!(
            "INSERT RELATION INTO inst_relate_cata_bool [{{ in: $inst_info, out: {mesh_key}, status: '{status}', source: '{source}', updated_at: time::now() }}];"
        ));
    }
    sql.push_str("};");

    if let Err(e) = aios_core::model_query_response(&sql).await {
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
        let mut relation_ids = Vec::new();
        let mut relation_records = Vec::new();

        for refno in chunk {
            let Some(aabb_hash) = inst_aabb_map.get(refno) else { continue };
            let refno_key = refno.to_pe_key();
            let aabb_key = {
                let v = aabb_hash.value();
                if v.starts_with("aabb:") {
                    v.to_string()
                } else {
                    format!("aabb:⟨{}⟩", v)
                }
            };

            // 使用批量插入语法，指定 ID 为 refno，这样导出代码可以通过 inst_relate_aabb:refno 查询
            let refno_str = refno.to_string();
            let relation_id = format!("inst_relate_aabb:⟨{}⟩", refno_str);
            relation_ids.push(relation_id.clone());
            relation_records.push(format!(
                "{{ id: {}, in: {}, out: {} }}",
                relation_id, refno_key, aabb_key
            ));
        }

        if relation_records.is_empty() {
            continue;
        }

        // 先删除旧记录（通过 ID），再批量插入新记录
        let mut sql = String::new();
        // if !relation_ids.is_empty() {
        //     sql.push_str(&format!(
        //         "DELETE [{}];",
        //         relation_ids.join(",")
        //     ));
        // }
        sql.push_str(&format!(
            "INSERT RELATION INTO inst_relate_aabb [{}];",
            relation_records.join(",")
        ));

        if let Err(e) = model_primary_db().query_take::<surrealdb::types::Value>(&sql, 0).await {
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
            let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
            for &k in chunk {
                let v = vec3_map.get(&k).unwrap();
                rows.push(format!("{{'id':vec3:⟨{}⟩, 'd':{}}}", k, v.value()));
            }
            let sql = format!("INSERT IGNORE INTO vec3 [{}];", rows.join(","));
            match model_primary_db().query(&sql).await {
                Ok(_) => {}
                Err(_e) => {
                    init_save_database_error(&sql, &std::panic::Location::caller().to_string());
                }
            };
        }
    }
}

pub async fn save_transforms_to_surreal(trans_map: &HashMap<u64, String>) -> anyhow::Result<()> {
    use anyhow::Context;

    if !trans_map.is_empty() {
        let keys = trans_map.keys().collect::<Vec<_>>();
        for chunk in keys.chunks(100) {
            let mut part = HashMap::with_capacity(chunk.len());
            for &k in chunk {
                if let Some(v) = trans_map.get(&k) {
                    part.insert(*k, v.clone());
                }
            }
            let sql = build_save_transforms_sql(&part);
            model_primary_db()
                .query(&sql)
                .await
                .with_context(|| format!("写入 trans 失败: {sql}"))?;
        }
    }
    Ok(())
}

fn build_save_transforms_sql(trans_map: &HashMap<u64, String>) -> String {
    if trans_map.is_empty() {
        return String::new();
    }

    let mut rows: Vec<String> = Vec::with_capacity(trans_map.len());
    for (k, v) in trans_map {
        rows.push(format!("{{'id':trans:⟨{}⟩, 'd':{}}}", k, v));
    }
    format!("INSERT IGNORE INTO trans [{}];", rows.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_save_transforms_sql_should_use_insert_ignore() {
        let mut trans_map = HashMap::new();
        trans_map.insert(1u64, "{\"translation\":[0,0,0]}".to_string());

        let sql = build_save_transforms_sql(&trans_map);
        assert!(sql.contains("INSERT IGNORE INTO trans"));
        assert!(!sql.contains("UPSERT trans:⟨1⟩"));
    }
}
