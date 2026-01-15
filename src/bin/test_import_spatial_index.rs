//! 测试从 instances.json 导入 SQLite 空间索引

use aios_database::sqlite_index::{
    ImportConfig, ImportStats, SqliteAabbIndex, i64_to_refno_str, refno_str_to_i64,
};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    println!("🧪 测试 instances.json 导入 SQLite 空间索引\n");

    // 1. 测试 refno 转换
    println!("1️⃣ 测试 refno 转换");
    let test_refno = "17496_170764";
    let id = refno_str_to_i64(test_refno).expect("转换失败");
    let back = i64_to_refno_str(id);
    println!("   {} -> {} -> {}", test_refno, id, back);
    assert_eq!(test_refno, back, "refno 转换不一致");
    println!("   ✅ refno 转换正确\n");

    // 2. 打开/创建 SQLite 索引
    println!("2️⃣ 创建 SQLite 索引");
    let db_path = Path::new("output/test_spatial_index.sqlite");
    if db_path.exists() {
        std::fs::remove_file(db_path)?;
    }
    let idx = SqliteAabbIndex::open(db_path)?;
    idx.init_schema()?;
    println!("   ✅ SQLite 索引创建成功: {}\n", db_path.display());

    // 3. 导入 instances.json
    println!("3️⃣ 导入 instances.json");
    let json_path = Path::new("output/instances/instances_1112.json");
    if !json_path.exists() {
        println!("   ⚠️ 文件不存在: {}", json_path.display());
        println!("   请先运行: cargo run --bin test_export_dbnum_instances_json -- 1112");
        return Ok(());
    }

    let config = ImportConfig::default();
    println!("   配置: EQUI 粗粒度={}, BRAN/HANG 细粒度={}",
             config.equi_coarse, config.bran_fine);

    let stats = idx.import_from_instances_json(json_path, &config)?;
    println!("   ✅ 导入完成!");
    println!("   📊 统计:");
    println!("      - EQUI (粗粒度): {}", stats.equi_count);
    println!("      - Children (细粒度): {}", stats.children_count);
    println!("      - Tubings (细粒度): {}", stats.tubings_count);
    println!("      - 总计遍历: {}", stats.total_inserted);
    println!("      - 去重后唯一记录: {}\n", stats.unique_count);

    // 4. 验证查询
    println!("4️⃣ 验证空间查询");
    let all_aabbs = idx.query_all_aabbs()?;
    println!("   查询到 {} 条 AABB 记录", all_aabbs.len());

    if !all_aabbs.is_empty() {
        let (id, minx, maxx, miny, maxy, minz, maxz) = &all_aabbs[0];
        let refno = i64_to_refno_str(*id);
        println!("   示例: refno={}, AABB=[{:.1},{:.1}]x[{:.1},{:.1}]x[{:.1},{:.1}]",
                 refno, minx, maxx, miny, maxy, minz, maxz);

        // 测试相交查询
        let intersect_ids = idx.query_intersect(*minx, *maxx, *miny, *maxy, *minz, *maxz)?;
        println!("   相交查询返回 {} 条记录", intersect_ids.len());
    }

    println!("\n✅ 所有测试通过!");
    Ok(())
}
