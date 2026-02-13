use super::context::NounProcessContext;
use crate::fast_model::loop_model;
use crate::fast_model::foyer_cache::geom_input_cache;
use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use anyhow::{Result, bail};
use dashmap::DashMap;
use glam::Vec3;
use std::sync::Arc;

/// 处理 Loop Owner 类型的 refno 页面
///
/// # Arguments
/// * `ctx` - 处理上下文
/// * `loop_sjus_map_arc` - Loop SJUS 映射 (用于 FLOOR/PANE/GWALL 的位置调整)
/// * `sender` - 几何数据发送通道
/// * `refnos` - 要处理的 refno 列表
///
/// # 支持的类型
/// - REVO/NREV: Revolution (旋转体)
/// - EXTR/NXTR/AEXTR: Extrusion (拉伸体)
/// - PANE/FLOOR/GWALL/SCREED: 建筑类型拉伸体
///
/// # 处理流程
/// 1. 空检查
/// 2. 调用 loop_model::gen_loop_geos 生成几何数据
/// 3. 错误处理
pub async fn process_loop_refno_page(
    ctx: &NounProcessContext,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    // cache-only 路由：当 AIOS_GEN_INPUT_CACHE_ONLY=1 时，从缓存读取预取数据
    if geom_input_cache::is_geom_input_cache_only() {
        let filtered = geom_input_cache::load_loop_inputs_for_refnos_from_global(refnos).await?;
        if filtered.len() != refnos.len() {
            let missing: Vec<String> = refnos
                .iter()
                .filter(|r| !filtered.contains_key(r))
                .take(16)
                .map(|r| r.to_string())
                .collect();
            bail!(
                "[loop_processor] cache-only 严格模式命中失败: request={}, hit={}, miss={}, sample={:?}",
                refnos.len(),
                filtered.len(),
                refnos.len() - filtered.len(),
                missing
            );
        }
        if !loop_model::gen_loop_geos_from_inputs(ctx.db_option.clone(), filtered, loop_sjus_map_arc, sender).await? {
            bail!("loop geos generation from cache failed");
        }
        return Ok(());
    }

    // 生成 loop 几何体
    if !loop_model::gen_loop_geos(ctx.db_option.clone(), refnos, loop_sjus_map_arc, sender).await? {
        bail!("loop geos generation failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_core::options::DbOption;
    use crate::options::DbOptionExt;

    #[tokio::test]
    async fn test_empty_refnos() {
        let ctx = NounProcessContext::new(
            Arc::new(DbOptionExt::from(DbOption::default())),
            100,
            4,
        );
        let loop_sjus_map = Arc::new(DashMap::new());
        let (sender, _receiver) = flume::unbounded();

        let result = process_loop_refno_page(&ctx, loop_sjus_map, sender, &[]).await;
        assert!(result.is_ok());
    }
}
