use aios_core::SurrealQueryExt;
use aios_core::SUL_DB;
use serde_json::Value;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 与主程序一致：先初始化数据库连接（Surreal/SQLite 等）
    let db_option_ext = aios_database::options::get_db_option_ext();
    aios_core::initialize_databases(&db_option_ext.inner).await?;

    let args: Vec<String> = std::env::args().collect();
    let mesh_id = args
        .iter()
        .position(|x| x == "--mesh-id")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("用法: cargo run --bin trace_mesh_origin -- --mesh-id <mesh_id>"))?;

    let inst_geo_id = format!("inst_geo:⟨{}⟩", mesh_id);

    println!("[trace] mesh_id={}", mesh_id);
    println!("[trace] inst_geo_id={}", inst_geo_id);

    // 1) inst_geo 本体（重点看 param）
    let sql_inst_geo = format!(
        "SELECT <string>id as id, <string>refno as refno, geo_type, unit_flag, param, meshed, bad, <string>aabb as aabb FROM inst_geo WHERE id = {inst_geo_id} LIMIT 1;"
    );
    let inst_geo_rows: Vec<Value> = SUL_DB.query_take(&sql_inst_geo, 0).await?;
    println!("\n[inst_geo]\n{}", serde_json::to_string_pretty(&inst_geo_rows)?);

    // 2) 哪些 geo_relate 指向了这个 inst_geo（从而拿到 geom_refno / geo_type）
    let sql_geo_relate = format!(
        r#"
SELECT
  <string>id as id,
  geo_type,
  <string>geom_refno as geom_refno,
  geom_refno.noun as geom_noun,
  geom_refno.type as geom_type,
  geom_refno.dbnum as geom_dbno,
  <string>in as in_id,
  <string>out as out_id
FROM geo_relate
WHERE out = {inst_geo_id}
LIMIT 50;
"#
    );
    let geo_relate_rows: Vec<Value> = SUL_DB.query_take(&sql_geo_relate, 0).await?;
    println!(
        "\n[geo_relate -> inst_geo]\n{}",
        serde_json::to_string_pretty(&geo_relate_rows)?
    );

    Ok(())
}
