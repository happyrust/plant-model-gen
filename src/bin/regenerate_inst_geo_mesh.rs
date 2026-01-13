use aios_core::{RecordId, SUL_DB, SurrealQueryExt};
use aios_core::utils::RecordIdExt;

use aios_database::fast_model::mesh_generate::gen_inst_meshes_by_geo_ids;
use aios_database::options::MeshFormat;

fn parse_inst_geo_id(args: &[String]) -> anyhow::Result<RecordId> {
    let raw = args
        .iter()
        .position(|x| x == "--geo-id" || x == "--mesh-id" || x == "--id")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "用法: cargo run --bin regenerate_inst_geo_mesh -- --mesh-id <u64>\n\
                 或: cargo run --bin regenerate_inst_geo_mesh -- --geo-id inst_geo:⟨<u64>⟩"
            )
        })?;

    if let Some((table, key)) = raw.split_once(':') {
        let key = key
            .trim()
            .trim_start_matches('⟨')
            .trim_end_matches('⟩')
            .trim_start_matches('`')
            .trim_end_matches('`')
            .to_string();
        anyhow::ensure!(!key.is_empty(), "geo-id 解析失败: key 为空 ({raw})");
        return Ok(RecordId::new(table.to_string(), key));
    }

    let key = raw.trim().to_string();
    anyhow::ensure!(!key.is_empty(), "mesh-id 解析失败: 为空");
    Ok(RecordId::new("inst_geo".to_string(), key))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let db_option_ext = aios_database::options::get_db_option_ext();
    aios_core::initialize_databases(&db_option_ext.inner).await?;

    let geo_id = parse_inst_geo_id(&args)?;
    let dir = db_option_ext.inner.get_meshes_path();
    let precision = db_option_ext.inner.mesh_precision.clone();

    println!("[regen] geo_id={}", geo_id.to_raw());
    println!("[regen] meshes_dir={}", dir.display());
    println!("[regen] default_lod={:?}", precision.default_lod);

    gen_inst_meshes_by_geo_ids(&dir, &precision, &[geo_id.clone()], &[MeshFormat::PdmsMesh])
        .await?;

    // 额外打印该 inst_geo 的 meshed/bad，方便确认是否成功回写
    let sql = format!(
        "SELECT <string>id as id, meshed, bad FROM inst_geo WHERE id = {} LIMIT 1;",
        geo_id.to_raw()
    );
    let rows: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
    println!("[regen] inst_geo status: {}", serde_json::to_string_pretty(&rows)?);

    Ok(())
}
