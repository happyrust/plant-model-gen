//! [foyer-removal] 桩模块：cache_flush 已移除，此处仅提供编译兼容。

use std::path::Path;
use aios_core::RefnoEnum;
use std::collections::HashSet;

/// 将最新的 instance cache 刷入 SurrealDB（桩实现）
pub async fn flush_latest_instance_cache_to_surreal(
    _cache_dir: &Path,
    _dbnums: Option<&[u32]>,
    _replace_exist: bool,
    _verbose: bool,
    _refno_filter: Option<&HashSet<RefnoEnum>>,
) -> anyhow::Result<usize> {
    eprintln!("[foyer-removal] cache_flush 已移除，跳过 flush");
    Ok(0)
}
