use super::context::NounProcessContext;
use crate::fast_model::prim_model;
use crate::fast_model::foyer_cache::geom_input_cache;
use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use anyhow::{Result, bail};
use std::sync::Arc;

/// 处理 Prim (基本体) 类型的 refno 页面
///
/// # Arguments
/// * `ctx` - 处理上下文
/// * `sender` - 几何数据发送通道
/// * `refnos` - 要处理的 refno 列表
///
/// # 支持的基本体类型
/// - BOX: 长方体
/// - CYL: 圆柱体
/// - CONE: 圆锥体
/// - SPHER: 球体
/// - TORUS: 圆环体
/// - POHE/POLYHE: 多面体
///
/// # 处理流程
/// 1. 空检查
/// 2. 调用 prim_model::gen_prim_geos 生成几何数据
/// 3. 错误处理
pub async fn process_prim_refno_page(
    ctx: &NounProcessContext,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    // cache-only 路由：当 AIOS_GEN_INPUT_CACHE_ONLY=1 时，从缓存读取预取数据
    if geom_input_cache::is_geom_input_cache_only() {
        let prim_inputs = geom_input_cache::load_all_prim_inputs_from_global().await;
        let want: std::collections::HashSet<RefnoEnum> = refnos.iter().copied().collect();
        let filtered: std::collections::HashMap<RefnoEnum, geom_input_cache::PrimInput> = prim_inputs
            .into_iter()
            .filter(|(k, _)| want.contains(k))
            .collect();
        if filtered.is_empty() {
            println!(
                "[prim_processor] cache-only: 缓存中未找到 {} 个 PRIM refno 的输入数据，跳过",
                refnos.len()
            );
            return Ok(());
        }
        if !prim_model::gen_prim_geos_from_cache(&filtered, sender).await? {
            bail!("prim geos generation from cache failed");
        }
        return Ok(());
    }

    // 生成 prim 几何体
    if !prim_model::gen_prim_geos(ctx.db_option.clone(), refnos, sender).await? {
        bail!("prim geos generation failed");
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
        let (sender, _receiver) = flume::unbounded();

        let result = process_prim_refno_page(&ctx, sender, &[]).await;
        assert!(result.is_ok());
    }
}
