//! 查询指定 BRAN/HANG 的 tubi_relate world_trans，用于对比形态异常

use aios_core::{project_primary_db, SurrealQueryExt, init_surreal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_surreal().await?;

    // 固定查询目标：24381_103385（可按需改）
    let sql = r#"
        SELECT
            record::id(id[0]) as refno,
            record::id(in) as owner,
            world_trans.d as world_trans,
            aabb.d as world_aabb,
            record::id(geo) as geo_hash,
            id[1] as index
        FROM tubi_relate:[pe:24381_103385, 0]..[pe:24381_103385, ..];
    "#;

    println!("SQL:\n{sql}");
    let rows: Vec<serde_json::Value> = project_primary_db().query_take(sql, 0).await?;
    println!("rows={}", rows.len());
    for (i, row) in rows.iter().enumerate() {
        println!("#{}: {}", i, serde_json::to_string_pretty(row)?);
    }

    Ok(())
}
