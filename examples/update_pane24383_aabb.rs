use anyhow::Context;
use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt};

/// 用于修复/回填 inst_relate_aabb(out->aabb.d) 的脚本：
/// - 只针对 inst_relate 中已有的 PANE（dbno=24383）
/// - 重新计算并写入 aabb.d + inst_relate_aabb 关系
///
/// 运行方式：
/// cargo run --release --example update_pane24383_aabb --features "gen_model,sqlite-index"
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    aios_core::init_surreal()
        .await
        .context("初始化 SurrealDB 失败")?;

    // 只筛选 dbno=24383 的 PANE（从 inst_relate 取，避免全表扫描 PANE noun 表）
    let sql = r#"
        SELECT VALUE in.id
        FROM inst_relate
        WHERE in.noun = 'PANE'
          AND type::int(string::split(meta::id(in), '_')[0]) = 24383
    "#;
    let refnos: Vec<RefnoEnum> = SUL_DB
        .query_take(sql, 0)
        .await
        .context("查询 inst_relate(PANE) refnos 失败")?;

    println!(
        "[update_pane24383_aabb] 命中 inst_relate(PANE dbno=24383) 数量: {}",
        refnos.len()
    );
    if refnos.is_empty() {
        return Ok(());
    }

    // 强制重算，确保把 out.d = NONE 的 aabb 补齐。
    aios_database::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos(&refnos, true)
        .await
        .context("update_inst_relate_aabbs_by_refnos 失败")?;

    println!("[update_pane24383_aabb] 完成");
    Ok(())
}

