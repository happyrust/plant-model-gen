use super::context::NounProcessContext;
use super::utilities::{build_cata_hash_map_from_session, is_valid_cata_hash};
use crate::fast_model::cata_model;
use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use anyhow::Result;
use dashmap::DashMap;
use glam::Vec3;
use std::sync::Arc;

/// 处理 Cate (元件库) 类型的 refno 页面
///
/// # Arguments
/// * `ctx` - 处理上下文
/// * `loop_sjus_map_arc` - Loop SJUS 映射
/// * `sender` - 几何数据发送通道
/// * `refnos` - 要处理的 refno 列表
pub async fn process_cate_refno_page(
    ctx: &NounProcessContext,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    // 查询 refnos 对应的 cata hash 分组
    let generation_read = Arc::clone(&ctx.generation_read);
    let target_cata_map = match build_cata_hash_map_from_session(&generation_read, refnos).await {
        Ok(map) => Arc::new(map),
        Err(e) => {
            // Direct 路径保留历史错误语义：记录并跳过当前 CATE 页面。
            eprintln!(
                "[cate_processor] build_cata_hash_map_from_tree 失败（将跳过 CATE）: {}",
                e
            );
            super::cache_miss_report::with_global_report(|r| {
                r.record_simple_miss(
                    "generate",
                    "cate:cata_hash_map_build_failed",
                    Some("build_cata_hash_map_from_tree failed (missing db_meta or tree files?)"),
                )
            });
            return Ok(());
        }
    };

    if target_cata_map.is_empty() {
        return Ok(());
    }

    // 生成 cata 几何体
    cata_model::gen_cata_instances_versioned(
        ctx.db_option.clone(),
        generation_read,
        target_cata_map,
        loop_sjus_map_arc,
        sender,
        ctx.generation_contract.respect_tufl(),
    )
    .await?;

    Ok(())
}
