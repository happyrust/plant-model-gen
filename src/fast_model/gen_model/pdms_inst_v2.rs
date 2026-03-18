/// 使用 SQLite 集中存储的简化版 regen 清理逻辑
use anyhow::Result;
use aios_core::RefnoEnum;
use crate::model_relation_store::global_store;
use std::collections::HashMap;

/// 新版预清理：利用 SQLite 简化逻辑
pub async fn pre_cleanup_for_regen_v2(seed_refnos: &[RefnoEnum]) -> Result<()> {
    if seed_refnos.is_empty() {
        return Ok(());
    }

    let t = std::time::Instant::now();

    // 1. 展开后代
    let all_refnos = aios_core::collect_descendant_filter_ids_with_self(
        seed_refnos, &[], None, true
    ).await?;

    println!(
        "[pre_cleanup_v2] seed={}, 展开后={}",
        seed_refnos.len(),
        all_refnos.len()
    );

    if all_refnos.is_empty() {
        return Ok(());
    }

    // 2. 按 dbnum 分组
    let mut refnos_by_dbnum: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &refno in &all_refnos {
        if let Some(dbnum) = aios_core::get_dbnum_by_refno(refno) {
            refnos_by_dbnum.entry(dbnum).or_default().push(refno);
        }
    }

    // 3. 并发清理各 dbnum（简化为单个函数调用）
    let store = global_store();
    let mut total_deleted = 0;

    for (dbnum, refnos) in refnos_by_dbnum {
        let deleted = store.cleanup_by_refnos(dbnum, &refnos)?;
        total_deleted += deleted;
        println!("  dbnum={} 清理 {} 条记录", dbnum, deleted);
    }

    println!(
        "[pre_cleanup_v2] 完成，删除 {} 条记录，耗时 {} ms",
        total_deleted,
        t.elapsed().as_millis()
    );

    Ok(())
}

/// 保存实例数据到 SQLite（替代 SurrealDB 多表写入）
pub async fn save_instance_data_to_sqlite(
    dbnum: u32,
    inst_relates: &[crate::model_relation_store::InstRelateRecord],
    geo_relates: &[(u64, u64)],
) -> Result<()> {
    let store = global_store();

    // 批量插入，自动事务
    store.insert_inst_relates(dbnum, inst_relates)?;
    store.insert_geo_relates(dbnum, geo_relates)?;

    Ok(())
}
