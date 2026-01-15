//! 测试房间计算 -> 导出 instances.json -> 导入 SQLite 空间索引

use aios_database::fast_model::build_room_relations;
use aios_database::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json;
use aios_database::sqlite_index::{ImportConfig, SqliteAabbIndex};
use aios_core::{get_db_option, init_surreal};
use std::path::Path;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🏠 房间计算 -> 空间索引 完整流程测试\n");

    // 1. 初始化数据库
    println!("1️⃣ 初始化数据库连接");
    init_surreal().await?;
    let db_option = get_db_option();
    println!("   ✅ 数据库连接成功\n");

    // 2. 运行房间计算
    println!("2️⃣ 运行房间计算");
    let stats = build_room_relations(&db_option).await?;
    println!("   ✅ 房间计算完成!");
    println!("   📊 统计:");
    println!("      - 房间数: {}", stats.total_rooms);
    println!("      - 面板数: {}", stats.total_panels);
    println!("      - 构件数: {}", stats.total_components);
    println!("      - 耗时: {}ms\n", stats.build_time_ms);

    // 3. 导出 instances.json (使用 dbno=1112 作为示例)
    println!("3️⃣ 导出 instances.json");
    let dbno = 1112u32;
    let output_dir = Path::new("output/instances");
    let db_option_arc = Arc::new((*db_option).clone());

    let export_stats = export_dbnum_instances_json(
        dbno,
        output_dir,
        db_option_arc,
        true, // verbose
        None, // 使用默认毫米单位
    ).await?;

    println!("   ✅ 导出完成!");
    println!("   📊 统计:");
    println!("      - refno_count: {}", export_stats.refno_count);
    println!("      - descendant_count: {}", export_stats.descendant_count);
    println!("      - 文件大小: {} bytes\n", export_stats.output_file_size);

    // 4. 导入到 SQLite 空间索引
    println!("4️⃣ 导入到 SQLite 空间索引");
    let json_path = output_dir.join(format!("instances_{}.json", dbno));
    let sqlite_path = Path::new("output/room_spatial_index.sqlite");

    if sqlite_path.exists() {
        std::fs::remove_file(sqlite_path)?;
    }

    let idx = SqliteAabbIndex::open(sqlite_path)?;
    idx.init_schema()?;

    let config = ImportConfig::default();
    let import_stats = idx.import_from_instances_json(&json_path, &config)?;

    println!("   ✅ 导入完成!");
    println!("   📊 统计:");
    println!("      - EQUI (粗粒度): {}", import_stats.equi_count);
    println!("      - Children (细粒度): {}", import_stats.children_count);
    println!("      - Tubings (细粒度): {}", import_stats.tubings_count);
    println!("      - 去重后唯一记录: {}\n", import_stats.unique_count);

    // 5. 验证空间查询
    println!("5️⃣ 验证空间查询");
    let all_aabbs = idx.query_all_aabbs()?;
    println!("   查询到 {} 条 AABB 记录", all_aabbs.len());

    println!("\n✅ 完整流程测试通过!");
    Ok(())
}
