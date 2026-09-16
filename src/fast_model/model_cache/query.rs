//! [foyer-removal] 桩模块：model_cache::query 已移除，此处仅提供编译兼容。

use aios_core::GeomInstQuery;
use aios_core::RefnoEnum;
use anyhow::Result;
use std::path::Path;

pub async fn query_geometry_instances_ext_from_cache(
    _refnos: &[RefnoEnum],
    _cache_dir: &Path,
    _enable_holes: bool,
    _include_negative: bool,
    _verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    anyhow::bail!("[foyer-removal] model_cache::query 已移除，请使用 SurrealDB 路径")
}
