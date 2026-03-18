/// 极简版性能测试
use aios_core::RefnoEnum;
use aios_database::model_relation_store_v3::{global_store_v3, RefnoRelations};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("=== 极简版 ModelRelationStore 测试 ===\n");

    let store = global_store_v3();
    let dbnum = 7997;

    // 1. 准备测试数据
    let mut relations = Vec::new();
    for i in 0..10000 {
        relations.push(RefnoRelations {
            refno: 100000 + i,
            inst_ids: vec![i as u64, i as u64 + 1],
            geo_hashes: vec![i as u64 * 10],
            tubi_segments: vec![],
            bool_results: vec![],
            world_matrices: vec![vec![0u8; 64]],
        });
    }

    // 2. 批量插入
    println!("插入 {} 条...", relations.len());
    let t = Instant::now();
    store.save_relations(dbnum, &relations)?;
    println!("  耗时 {} ms\n", t.elapsed().as_millis());

    // 3. 批量读取
    let query_refnos: Vec<RefnoEnum> = (100000..100100).map(RefnoEnum).collect();
    println!("读取 {} 个 refno...", query_refnos.len());
    let t = Instant::now();
    let loaded = store.load_relations(dbnum, &query_refnos)?;
    println!("  返回 {} 条，耗时 {} ms\n", loaded.len(), t.elapsed().as_millis());

    // 4. 批量删除
    let cleanup_refnos: Vec<RefnoEnum> = (100000..110000).map(RefnoEnum).collect();
    println!("删除 {} 个 refno...", cleanup_refnos.len());
    let t = Instant::now();
    let deleted = store.cleanup_by_refnos(dbnum, &cleanup_refnos)?;
    println!("  删除 {} 条，耗时 {} ms\n", deleted, t.elapsed().as_millis());

    // 5. 统计
    let count = store.get_stats(dbnum)?;
    println!("剩余记录: {}", count);

    println!("\n✅ 测试完成");
    Ok(())
}
