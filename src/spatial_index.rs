#![cfg(feature = "sqlite-index")]

use crate::sqlite_index::SqliteAabbIndex;
use aios_core::pdms_types::RefU64;
use anyhow::Context;
use parry3d::bounding_volume::Aabb;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SpatialIndexStats {
    pub total_elements: usize,
    pub index_type: String,
}

/// 兼容旧代码的 SQLite 空间索引包装。
///
/// 说明：
/// - 旧实现叫 `SqliteSpatialIndex`（src/spatial_index.rs），目前仓库里只有更轻量的 `sqlite_index::SqliteAabbIndex`。
/// - web_server/handlers.rs 仍在使用旧 API（with_default_path / get_aabb / query_intersect / get_stats / clear / default_path）。
/// - 这里做一个最小适配层，保证 `web_server` 能编译并可运行相关诊断接口。
pub struct SqliteSpatialIndex {
    inner: SqliteAabbIndex,
}

impl SqliteSpatialIndex {
    pub fn inner(&self) -> &SqliteAabbIndex {
        &self.inner
    }

    pub fn is_enabled() -> bool {
        true
    }

    pub fn default_path() -> PathBuf {
        // 允许通过环境变量覆盖默认索引路径，便于测试/现场诊断使用最小索引文件。
        if let Ok(p) = std::env::var("AIOS_SPATIAL_INDEX_SQLITE") {
            let p = p.trim();
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        // 保持相对路径：运行目录通常是仓库根目录
        PathBuf::from("output").join("spatial_index.sqlite")
    }

    pub fn with_default_path() -> anyhow::Result<Self> {
        let path = Self::default_path();
        let idx =
            SqliteAabbIndex::open(&path).with_context(|| format!("open sqlite index: {path:?}"))?;
        idx.init_schema().context("init sqlite index schema")?;
        Ok(Self { inner: idx })
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let conn = Connection::open(self.inner.path())?;
        conn.execute("DELETE FROM aabb_index", [])?;
        conn.execute("DELETE FROM items", [])?;
        Ok(())
    }

    pub fn get_stats(&self) -> anyhow::Result<SpatialIndexStats> {
        let conn = Connection::open(self.inner.path())?;
        let total: i64 = conn.query_row("SELECT COUNT(1) FROM aabb_index", [], |row| row.get(0))?;
        Ok(SpatialIndexStats {
            total_elements: usize::try_from(total.max(0)).unwrap_or(0),
            index_type: "sqlite-rtree".to_string(),
        })
    }

    pub fn get_aabb(&self, refno: RefU64) -> anyhow::Result<Option<Aabb>> {
        let conn = Connection::open(self.inner.path())?;
        let mut stmt = conn.prepare(
            "SELECT min_x, max_x, min_y, max_y, min_z, max_z FROM aabb_index WHERE id = ?1",
        )?;
        let mut rows = stmt.query([refno.0 as i64])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let minx: f64 = row.get(0)?;
        let maxx: f64 = row.get(1)?;
        let miny: f64 = row.get(2)?;
        let maxy: f64 = row.get(3)?;
        let minz: f64 = row.get(4)?;
        let maxz: f64 = row.get(5)?;
        Ok(Some(Aabb::new(
            [minx as f32, miny as f32, minz as f32].into(),
            [maxx as f32, maxy as f32, maxz as f32].into(),
        )))
    }

    pub fn query_intersect(&self, query: &Aabb) -> anyhow::Result<Vec<RefU64>> {
        let ids = self.inner.query_intersect(
            query.mins.x as f64,
            query.maxs.x as f64,
            query.mins.y as f64,
            query.maxs.y as f64,
            query.mins.z as f64,
            query.maxs.z as f64,
        )?;
        Ok(ids
            .into_iter()
            .filter_map(|id| u64::try_from(id).ok().map(RefU64))
            .collect())
    }

    pub fn get_noun(&self, refno: RefU64) -> anyhow::Result<Option<String>> {
        let conn = Connection::open(self.inner.path())?;
        let noun: Option<String> = conn
            .query_row(
                "SELECT noun FROM items WHERE id = ?1",
                [refno.0 as i64],
                |row| row.get(0),
            )
            .optional()?;
        Ok(noun)
    }
}
