//! 按 geo_hash 查询 inst_geo 记录，打印 param/unit_flag（用于定位某个 mesh_id 对应的基本体类型）。

use aios_core::{SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    let geo_hash = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GEO_HASH").ok())
        .unwrap_or_else(|| "9153972095265005083".to_string());

    let sql = format!(
        "SELECT id, unit_flag ?? false as unit_flag, param FROM inst_geo:⟨{}⟩",
        geo_hash
    );

    let rows: Vec<serde_json::Value> = SUL_DB
        .query_take(&sql, 0)
        .await
        .with_context(|| format!("query inst_geo failed: {}", sql))?;

    if rows.is_empty() {
        println!("⚠️ 未找到 inst_geo:⟨{}⟩", geo_hash);
        return Ok(());
    }

    for (i, row) in rows.iter().enumerate() {
        println!("== row[{i}] ==");
        println!("{}", serde_json::to_string_pretty(row)?);
    }

    Ok(())
}

