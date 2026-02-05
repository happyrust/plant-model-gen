//! 缓存清理模块
//!
//! 提供清理 foyer cache 缓存生成的模型数据功能，支持：
//! - 按 `dbnum` 清空指定数据库的缓存
//! - 按 `refno` 清空指定元件的缓存（自动解析为 dbnum）
//! - 清空所有缓存

use std::path::Path;

use anyhow::Result;
use aios_core::RefnoEnum;

use crate::data_interface::db_meta_manager::db_meta;
use crate::fast_model::instance_cache::InstanceCacheManager;

/// 缓存清理结果统计
#[derive(Debug, Default)]
pub struct CacheCleanStats {
    pub instance_batches_removed: usize,
    pub dbnums_affected: Vec<u32>,
}

/// 按 dbnum 清理缓存
pub async fn clean_cache_by_dbnum(
    cache_dir: &Path,
    dbnums: &[u32],
    verbose: bool,
) -> Result<CacheCleanStats> {
    let cache = InstanceCacheManager::new(cache_dir).await?;
    let mut stats = CacheCleanStats::default();

    for &dbnum in dbnums {
        let removed = cache.remove_dbnum(dbnum);
        if removed > 0 {
            stats.instance_batches_removed += removed;
            stats.dbnums_affected.push(dbnum);
            if verbose {
                println!("[cache_clean] 已清理 dbnum={} 的 {} 个 batch", dbnum, removed);
            }
        } else if verbose {
            println!("[cache_clean] dbnum={} 无缓存数据，跳过", dbnum);
        }
    }

    cache.close().await?;
    Ok(stats)
}

/// 按 refno 清理缓存（自动解析为 dbnum）
pub async fn clean_cache_by_refno(
    cache_dir: &Path,
    refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<CacheCleanStats> {
    // 确保 db_meta 已加载
    if let Err(e) = db_meta().ensure_loaded() {
        anyhow::bail!("db_meta 未加载，无法解析 refno 到 dbnum: {}", e);
    }

    // 将 refno 解析为 dbnum 并去重
    let mut dbnums: Vec<u32> = Vec::new();
    for &refno in refnos {
        if let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) {
            if dbnum > 0 && !dbnums.contains(&dbnum) {
                dbnums.push(dbnum);
                if verbose {
                    println!("[cache_clean] refno {} -> dbnum {}", refno, dbnum);
                }
            }
        } else if verbose {
            println!("[cache_clean] 无法解析 refno {} 的 dbnum，跳过", refno);
        }
    }

    if dbnums.is_empty() {
        if verbose {
            println!("[cache_clean] 未找到有效的 dbnum，无需清理");
        }
        return Ok(CacheCleanStats::default());
    }

    clean_cache_by_dbnum(cache_dir, &dbnums, verbose).await
}

/// 清理所有缓存
pub async fn clean_all_cache(cache_dir: &Path, verbose: bool) -> Result<CacheCleanStats> {
    let cache = InstanceCacheManager::new(cache_dir).await?;
    let all_dbnums = cache.list_dbnums();

    if all_dbnums.is_empty() {
        if verbose {
            println!("[cache_clean] 缓存为空，无需清理");
        }
        cache.close().await?;
        return Ok(CacheCleanStats::default());
    }

    if verbose {
        println!("[cache_clean] 发现 {} 个 dbnum，开始清理...", all_dbnums.len());
    }

    let mut stats = CacheCleanStats::default();
    for dbnum in all_dbnums {
        let removed = cache.remove_dbnum(dbnum);
        if removed > 0 {
            stats.instance_batches_removed += removed;
            stats.dbnums_affected.push(dbnum);
            if verbose {
                println!("[cache_clean] 已清理 dbnum={} 的 {} 个 batch", dbnum, removed);
            }
        }
    }

    cache.close().await?;
    Ok(stats)
}
