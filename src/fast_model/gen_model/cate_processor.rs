use super::context::NounProcessContext;
use super::utilities::{build_cata_hash_map_from_tree, is_valid_cata_hash};
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
    let target_cata_map = match build_cata_hash_map_from_tree(refnos).await {
        Ok(map) => Arc::new(map),
        Err(e) => {
            // 离线生成不可回查 DB；此处失败即表示 prefetch/元数据准备不完整，必须直接失败。
            if ctx.is_offline_generate() {
                return Err(e);
            }

            // Direct/非离线路径：保守起见仅记录并跳过，避免影响历史行为。
            eprintln!(
                "[cate_processor] build_cata_hash_map_from_tree 失败（将跳过 CATE）: {}",
                e
            );
            super::cache_miss_report::with_global_report(|r| {
                r.record_simple_miss(
                    ctx.gen_stage.as_str(),
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

    // 离线生成 / cata_resolve_cache prefetch 路径已移除（foyer-cache-cleanup），直接走 SurrealDB

    // 生成 cata 几何体
    cata_model::gen_cata_instances(
        ctx.db_option.clone(),
        target_cata_map,
        loop_sjus_map_arc,
        sender,
    )
    .await?;

    Ok(())
}

// gen_cate_instances_from_cache_only 已移除（foyer-cache-cleanup）

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::DbOptionExt;
    use aios_core::options::DbOption;

    #[tokio::test]
    async fn test_empty_refnos() {
        let ctx = NounProcessContext::new(Arc::new(DbOptionExt::from(DbOption::default())), 100, 4);
        let loop_sjus_map = Arc::new(DashMap::new());
        let (sender, _receiver) = flume::unbounded();

        let result = process_cate_refno_page(&ctx, loop_sjus_map, sender, &[]).await;
        assert!(result.is_ok());
    }
}
