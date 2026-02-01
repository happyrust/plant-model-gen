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

    // 提取 dbnum 列表
    let mut dbnums = HashSet::new();
    for refno in refnos {
        let dbnum = crate::data_interface::db_meta_manager::db_meta()
            .get_dbnum_by_refno(*refno)
            .unwrap_or_else(|| refno.refno().get_0());
        dbnums.insert(dbnum);
    }

    let dbnum_vec: Vec<u32> = dbnums.into_iter().collect();

    log::info!("[precheck] 刷新 {} 个 refno 对应的 {} 个数据库的 pe_transform",
               refnos.len(), dbnum_vec.len());

    // 调用 aios_core 的刷新函数
    match aios_core::transform::refresh_pe_transform_for_dbnums(&dbnum_vec).await {
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
