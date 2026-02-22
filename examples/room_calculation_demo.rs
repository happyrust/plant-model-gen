//! 房间计算演示（示例程序）
//!
//! Cargo.toml 已将该示例设置为 required-features = ["gen_model", "sqlite-index"]。
//!
//! 用途：在真实 SurrealDB 数据集上演示“模型生成 -> 房间计算”的最小闭环。
//!
//! 可选环境变量：
//! - DEBUG_REFNOS：逗号分隔的 refno 列表（如 "25688/71821,24383/73962"），用于限定生成范围

use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // 读取配置（默认 DbOption.toml）
    let mut db_option_ext =
        aios_database::options::get_db_option_ext_from_path("db_options/DbOption")
            .context("加载 DbOption 失败")?;

    // 初始化 SurrealDB 连接
    aios_core::init_surreal()
        .await
        .context("初始化 SurrealDB 失败")?;

    // 可选：通过 DEBUG_REFNOS 限定生成范围（等价于 debug_refno 传参）
    if let Ok(debug_refnos) = std::env::var("DEBUG_REFNOS") {
        let refnos: Vec<String> = debug_refnos
            .split(|c| c == ',' || c == ';' || c == ' ' || c == '\t' || c == '\n')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .collect();
        if !refnos.is_empty() {
            aios_core::set_debug_model_enabled(true);
            db_option_ext.inner.debug_model_refnos = Some(refnos);
            db_option_ext.inner.replace_mesh = Some(true);
        }
    }

    // 确保开启生成开关
    db_option_ext.inner.gen_model = true;
    db_option_ext.inner.gen_mesh = true;

    // 1) 生成模型（会自动走 debug_model_refnos 限定路径）
    aios_database::fast_model::gen_all_geos_data(
        vec![],
        &db_option_ext,
        None,
        db_option_ext.target_sesno,
    )
    .await
    .context("模型生成失败")?;

    // 2) 房间计算（落库 room_relate）
    let stats = aios_database::fast_model::build_room_relations(&db_option_ext.inner)
        .await
        .context("房间计算失败")?;

    println!(
        "✅ 房间计算完成: rooms={}, panels={}, components={}, build_time_ms={}",
        stats.total_rooms, stats.total_panels, stats.total_components, stats.build_time_ms
    );

    Ok(())
}
