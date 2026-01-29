use std::path::Path;

use aios_core::geometry::ShapeInstancesData;
use anyhow::{Context, Result};

use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::instance_cache::CachedInstanceBatch;
use crate::fast_model::pdms_inst::save_instance_data_optimize;
use crate::fast_model::utils::save_inst_relate_bool;

/// 将 foyer/instance_cache 中的“最新 batch”批量写入 SurrealDB（用于备份/落库）。
///
/// 约定：
/// - 该函数只负责“缓存 -> DB”同步，不参与模型生成。
/// - 需由调用方提前 `init_surreal()`。
pub async fn flush_latest_instance_cache_to_surreal(
    cache_dir: &Path,
    dbnums: Option<&[u32]>,
    replace_exist: bool,
    verbose: bool,
) -> Result<usize> {
    let cache = InstanceCacheManager::new(cache_dir)
        .await
        .with_context(|| format!("打开 instance_cache 失败: {}", cache_dir.display()))?;

    let mut targets: Vec<u32> = match dbnums {
        Some(v) => v.to_vec(),
        None => cache.list_dbnums(),
    };
    targets.sort_unstable();
    targets.dedup();

    if targets.is_empty() {
        if verbose {
            println!("[cache_flush] instance_cache 为空：{}", cache_dir.display());
        }
        return Ok(0);
    }

    let mut flushed = 0usize;

    for dbnum in targets {
        let batch_ids = cache.list_batches(dbnum);
        let Some(latest_batch_id) = batch_ids.last().cloned() else {
            if verbose {
                println!("[cache_flush] dbnum={} 没有 batch，跳过", dbnum);
            }
            continue;
        };

        let Some(batch) = cache.get(dbnum, &latest_batch_id).await else {
            if verbose {
                println!(
                    "[cache_flush] dbnum={} batch_id={} 读取失败，跳过",
                    dbnum, latest_batch_id
                );
            }
            continue;
        };

        let CachedInstanceBatch {
            inst_info_map,
            inst_geos_map,
            inst_tubi_map,
            neg_relate_map,
            ngmr_neg_relate_map,
            inst_relate_bool_map,
            ..
        } = batch;

        if verbose {
            println!(
                "[cache_flush] dbnum={} batch_id={} inst_info={} inst_geos={} inst_tubi={} neg={} ngmr={} bool={}",
                dbnum,
                latest_batch_id,
                inst_info_map.len(),
                inst_geos_map.len(),
                inst_tubi_map.len(),
                neg_relate_map.len(),
                ngmr_neg_relate_map.len(),
                inst_relate_bool_map.len(),
            );
        }

        let shape = ShapeInstancesData {
            inst_info_map,
            inst_tubi_map,
            inst_geos_map,
            neg_relate_map,
            ngmr_neg_relate_map,
        };

        save_instance_data_optimize(&shape, replace_exist)
            .await
            .with_context(|| format!("写入实例数据失败: dbnum={} batch_id={}", dbnum, latest_batch_id))?;

        // 回写 cache-only 布尔结果状态到 inst_relate_bool（用于 DB 侧查询/诊断）。
        for (refno, b) in inst_relate_bool_map {
            let mesh_id = if b.mesh_id.is_empty() {
                None
            } else {
                Some(b.mesh_id.as_str())
            };
            save_inst_relate_bool(refno, mesh_id, &b.status, "cache_flush").await;
        }

        flushed += 1;
    }

    Ok(flushed)
}
