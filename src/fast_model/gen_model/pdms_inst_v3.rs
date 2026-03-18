/// 极简版清理逻辑：单条 DELETE 完成所有清理
use anyhow::Result;
use aios_core::RefnoEnum;
use crate::model_relation_store_v3::{global_store_v3, RefnoRelations};
use std::collections::HashMap;

/// 极简版预清理：从 500+ 行降到 20 行
pub async fn pre_cleanup_for_regen_v3(seed_refnos: &[RefnoEnum]) -> Result<()> {
    if seed_refnos.is_empty() {
        return Ok(());
    }

    let t = std::time::Instant::now();

    // 1. 展开后代
    let all_refnos = aios_core::collect_descendant_filter_ids_with_self(
        seed_refnos, &[], None, true
    ).await?;

    // 2. 按 dbnum 分组
    let mut refnos_by_dbnum: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &refno in &all_refnos {
        if let Some(dbnum) = aios_core::get_dbnum_by_refno(refno) {
            refnos_by_dbnum.entry(dbnum).or_default().push(refno);
        }
    }

    // 3. 单条 DELETE 清理（核心简化）
    let store = global_store_v3();
    let mut total = 0;
    for (dbnum, refnos) in refnos_by_dbnum {
        let deleted = store.cleanup_by_refnos(dbnum, &refnos)?;
        total += deleted;
    }

    println!("[cleanup_v3] 删除 {} 条，耗时 {} ms", total, t.elapsed().as_millis());
    Ok(())
}

/// 保存模型数据：聚合后批量写入
pub async fn save_model_relations_v3(
    dbnum: u32,
    refno_data_map: HashMap<RefnoEnum, RefnoRelations>,
) -> Result<()> {
    let relations: Vec<_> = refno_data_map.into_values().collect();
    global_store_v3().save_relations(dbnum, &relations)?;
    Ok(())
}
