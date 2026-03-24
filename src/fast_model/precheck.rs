//! precheck - 模型生成前置预检查
//! 确保 pe_transform(world_trans) 等必要数据可用

use aios_core::RefnoEnum;
use std::collections::HashSet;

/// 确保指定 refnos 的 pe_transform 数据存在
/// 如果不存在，则尝试生成
pub async fn ensure_pe_transform_for_refnos(refnos: &[RefnoEnum]) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();

    // 提取 dbnum 列表
    let mut dbnums = HashSet::new();
    for refno in refnos {
        if let Some(dbnum) = db_meta.get_dbnum_by_refno(*refno) {
            dbnums.insert(dbnum);
        } else {
            // 禁止回退用 ref0 当 dbnum；映射缺失时仅告警，不阻断流程。
            log::warn!(
                "[precheck] 缺少 ref0->dbnum 映射，跳过 pe_transform 刷新: refno={}",
                refno
            );
        }
    }

    let dbnum_vec: Vec<u32> = dbnums.into_iter().collect();
    if dbnum_vec.is_empty() {
        return Ok(());
    }

    log::info!(
        "[precheck] 刷新 {} 个 refno 对应的 {} 个数据库的 pe_transform",
        refnos.len(),
        dbnum_vec.len()
    );

    // 调用 aios_core 的刷新函数
    match crate::pe_transform_refresh::refresh_pe_transform_for_dbnums_compat(&dbnum_vec).await {
        Ok(count) => {
            log::info!("[precheck] pe_transform 刷新完成，处理 {} 个节点", count);
            Ok(())
        }
        Err(e) => {
            log::warn!("[precheck] pe_transform 刷新失败: {}", e);
            Ok(()) // 不阻断流程
        }
    }
}
