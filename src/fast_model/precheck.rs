//! precheck - 模型生成前置预检查
//! 确保 pe_transform(world_trans) 等必要数据可用

use aios_core::RefnoEnum;

/// 确保指定 refnos 的 pe_transform 数据存在
/// 如果不存在，则尝试生成
pub async fn ensure_pe_transform_for_refnos(refnos: &[RefnoEnum]) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }
    
    // 目前暂时跳过预检查，后续可实现检查和刷新逻辑
    // TODO: 实现 pe_transform 缺失检查和刷新
    log::debug!("[precheck] 跳过 {} 个 refno 的 pe_transform 预检查", refnos.len());
    
    Ok(())
}
