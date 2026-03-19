/// 测试 ModelRelationStore 的性能和正确性
use aios_core::RefnoEnum;
use aios_database::model_relation_store::{InstRelateRecord, global_store};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("=== ModelRelationStore 性能测试 ===\n");

    let store = global_store();
    let dbnum = 7997;

    // 1. 准备测试数据
    println!("1. 准备 10000 条测试数据...");
    let mut records = Vec::new();
    for i in 0..10000 {
        records.push(InstRelateRecord {
            refno: RefnoEnum(100000 + i),
            inst_id: i as u64,
            parent_refno: Some(RefnoEnum(1000)),
            world_matrix: Some(vec![0u8; 64]),
        });
    }

    // 2. 批量插入测试
    println!("2. 批量插入测试...");
    let t = Instant::now();
    store.insert_inst_relates(dbnum, &records)?;
    println!(
        "   插入 {} 条，耗时 {} ms",
        records.len(),
        t.elapsed().as_millis()
    );

    // 3. 查询测试
    println!("3. 查询测试...");
    let query_refnos: Vec<RefnoEnum> = (100000..100100).map(RefnoEnum).collect();
    let t = Instant::now();
    let inst_ids = store.query_inst_ids_by_refnos(dbnum, &query_refnos)?;
    println!(
        "   查询 {} 个 refno，返回 {} 条，耗时 {} ms",
        query_refnos.len(),
        inst_ids.len(),
        t.elapsed().as_millis()
    );

    // 4. 清理测试
    println!("4. 清理测试...");
    let cleanup_refnos: Vec<RefnoEnum> = (100000..101000).map(RefnoEnum).collect();
    let t = Instant::now();
    let deleted = store.cleanup_by_refnos(dbnum, &cleanup_refnos)?;
    println!(
        "   清理 {} 个 refno，删除 {} 条，耗时 {} ms",
        cleanup_refnos.len(),
        deleted,
        t.elapsed().as_millis()
    );

    // 5. 统计信息
    println!("5. 统计信息...");
    let stats = store.get_stats(dbnum)?;
    println!("   inst_relate: {}", stats.inst_relate_count);
    println!("   geo_relate: ", stats.geo_relate_count);
    println!("   inst_geo: {}", stats.inst_geo_count);

    println!("\n✅ 测试完成");
    Ok(())
}
