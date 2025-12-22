//! DuckDB 读取器 - 用于空间查询和数据读取
//!
//! 从 `assets/web_duckdb/` 目录读取最新的 DuckDB 文件，
//! 提供空间查询功能替代 SQLite。

use anyhow::{Context, Result};
use duckdb::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 获取最新的 DuckDB 文件路径
fn get_latest_duckdb_path() -> Result<PathBuf> {
    let web_dir = Path::new("assets").join("web_duckdb");
    let latest_path = web_dir.join("latest.json");
    
    if !latest_path.exists() {
        anyhow::bail!("latest.json 不存在: {:?}", latest_path);
    }
    
    let content = std::fs::read_to_string(&latest_path)?;
    let latest: serde_json::Value = serde_json::from_str(&content)?;
    
    let db_filename = latest["db_filename"]
        .as_str()
        .context("latest.json 中缺少 db_filename")?;
    
    let db_path = web_dir.join(db_filename);
    if !db_path.exists() {
        anyhow::bail!("DuckDB 文件不存在: {:?}", db_path);
    }
    
    Ok(db_path)
}

/// DuckDB 读取器
pub struct DuckDBReader {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl DuckDBReader {
    /// 打开最新的 DuckDB 数据库
    pub fn open_latest() -> Result<Self> {
        let db_path = get_latest_duckdb_path()?;
        Self::open(&db_path)
    }
    
    /// 打开指定的 DuckDB 数据库
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            db_path,
            duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
        )
        .context("打开 DuckDB 数据库失败")?;
        
        println!("📖 [DuckDB Reader] 已打开: {:?}", db_path);
        
        Ok(Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_path_buf(),
        })
    }
    
    /// 获取数据库路径
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
    
    /// 空间查询：按包围盒查询 refnos
    pub fn query_by_bounding_box(
        &self,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
    ) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        
        let mut stmt = conn.prepare(
            r#"
            SELECT refno FROM aabb
            WHERE max_x >= ?1 AND min_x <= ?2
              AND max_y >= ?3 AND min_y <= ?4
              AND max_z >= ?5 AND min_z <= ?6
            "#,
        )?;
        
        let refnos: Vec<String> = stmt
            .query_map(
                duckdb::params![min_x, max_x, min_y, max_y, min_z, max_z],
                |row| row.get(0),
            )?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(refnos)
    }
    
    /// 查询指定 refno 的包围盒
    pub fn query_aabb(&self, refno: &str) -> Result<Option<(f64, f64, f64, f64, f64, f64)>> {
        let conn = self.conn.lock().unwrap();
        
        let mut stmt = conn.prepare(
            "SELECT min_x, min_y, min_z, max_x, max_y, max_z FROM aabb WHERE refno = ?1",
        )?;
        
        let result: Option<(f64, f64, f64, f64, f64, f64)> = stmt
            .query_row(duckdb::params![refno], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .ok();
        
        Ok(result)
    }
    
    /// 查询指定 refno 的几何体信息
    pub fn query_geos(&self, refno: &str) -> Result<Vec<(String, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        
        let mut stmt = conn.prepare(
            "SELECT id, geo_hash, geo_transform FROM geo WHERE refno = ?1",
        )?;
        
        let geos: Vec<(String, String, Option<String>)> = stmt
            .query_map(duckdb::params![refno], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(geos)
    }
    
    /// 查询实例信息
    pub fn query_instance(&self, refno: &str) -> Result<Option<(String, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap();
        
        let mut stmt = conn.prepare(
            "SELECT refno, noun, owner_refno FROM instance WHERE refno = ?1",
        )?;
        
        let result: Option<(String, String, Option<String>)> = stmt
            .query_row(duckdb::params![refno], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .ok();
        
        Ok(result)
    }
    
    /// 获取统计信息
    pub fn get_stats(&self) -> Result<(i64, i64, i64)> {
        let conn = self.conn.lock().unwrap();
        
        let instance_count: i64 = conn.query_row("SELECT COUNT(*) FROM instance", [], |row| row.get(0))?;
        let geo_count: i64 = conn.query_row("SELECT COUNT(*) FROM geo", [], |row| row.get(0))?;
        let aabb_count: i64 = conn.query_row("SELECT COUNT(*) FROM aabb", [], |row| row.get(0))?;
        
        Ok((instance_count, geo_count, aabb_count))
    }
}

/// 全局 DuckDB 读取器实例
static DUCKDB_READER: once_cell::sync::Lazy<Mutex<Option<DuckDBReader>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// 获取全局 DuckDB 读取器
pub fn get_duckdb_reader() -> Result<std::sync::MutexGuard<'static, Option<DuckDBReader>>> {
    Ok(DUCKDB_READER.lock().unwrap())
}

/// 初始化全局 DuckDB 读取器
pub fn init_duckdb_reader() -> Result<()> {
    let reader = DuckDBReader::open_latest()?;
    let mut guard = DUCKDB_READER.lock().unwrap();
    *guard = Some(reader);
    Ok(())
}

/// 刷新全局 DuckDB 读取器（重新加载最新文件）
pub fn refresh_duckdb_reader() -> Result<()> {
    init_duckdb_reader()
}

// ============== 空间查询扩展 ==============

use aios_core::RefU64;
use parry3d::bounding_volume::Aabb;
use nalgebra::{Point3, Vector3};

/// 空间查询结果项（与 spatial_index.rs 中的 SpatialHit 一致）
#[derive(Debug, Clone)]
pub struct DuckDBSpatialHit {
    pub refno: RefU64,
    pub bbox: Option<Aabb>,
    pub distance: Option<f32>,
}

impl DuckDBReader {
    /// 空间相交查询：返回与给定 AABB 相交的所有 refno
    pub fn query_intersect(&self, query_aabb: &Aabb) -> Result<Vec<RefU64>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            r#"
            SELECT refno FROM aabb
            WHERE max_x >= ?1 AND min_x <= ?2
              AND max_y >= ?3 AND min_y <= ?4
              AND max_z >= ?5 AND min_z <= ?6
            "#,
        )?;

        let refnos: Vec<RefU64> = stmt
            .query_map(
                duckdb::params![
                    query_aabb.mins.x as f64, query_aabb.maxs.x as f64,
                    query_aabb.mins.y as f64, query_aabb.maxs.y as f64,
                    query_aabb.mins.z as f64, query_aabb.maxs.z as f64
                ],
                |row| {
                    let refno_str: String = row.get(0)?;
                    // 解析 refno 格式 "dbno_sesno"
                    let parts: Vec<&str> = refno_str.split('_').collect();
                    if parts.len() >= 2 {
                        if let (Ok(dbno), Ok(sesno)) = (
                            parts[0].parse::<u32>(),
                            parts[1].parse::<u32>(),
                        ) {
                            return Ok(RefU64::from_two_nums(dbno, sesno));
                        }
                    }
                    Ok(RefU64(0))
                },
            )?
            .filter_map(|r| r.ok())
            .filter(|r| r.0 != 0)
            .collect();

        Ok(refnos)
    }

    /// 查询指定 refno 的包围盒（返回 Aabb）
    pub fn query_aabb_parry(&self, refno: &str) -> Result<Option<Aabb>> {
        let result = self.query_aabb(refno)?;
        Ok(result.map(|(min_x, min_y, min_z, max_x, max_y, max_z)| {
            Aabb::new(
                Point3::new(min_x as f32, min_y as f32, min_z as f32),
                Point3::new(max_x as f32, max_y as f32, max_z as f32),
            )
        }))
    }

    /// 球形范围查询：返回距离中心点一定半径内的 refnos
    pub fn query_within_radius(
        &self,
        center: Point3<f32>,
        radius: f32,
    ) -> Result<Vec<DuckDBSpatialHit>> {
        let query_aabb = Aabb::new(
            Point3::new(center.x - radius, center.y - radius, center.z - radius),
            Point3::new(center.x + radius, center.y + radius, center.z + radius),
        );

        let refnos = self.query_intersect(&query_aabb)?;
        let mut hits = Vec::new();

        for refno in refnos {
            let refno_str = refno.to_string();
            if let Some(aabb) = self.query_aabb_parry(&refno_str)? {
                let distance = distance_point_aabb(center, &aabb);
                if distance <= radius {
                    hits.push(DuckDBSpatialHit {
                        refno,
                        bbox: Some(aabb),
                        distance: Some(distance),
                    });
                }
            }
        }

        // 按距离排序
        hits.sort_by(|a, b| {
            a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(hits)
    }

    /// 近邻查询（K最近邻）
    pub fn query_knn(
        &self,
        point: Point3<f32>,
        k: usize,
        search_radius: f32,
    ) -> Result<Vec<DuckDBSpatialHit>> {
        let mut hits = self.query_within_radius(point, search_radius)?;
        hits.truncate(k);
        Ok(hits)
    }
}

/// 点到 AABB 的最短距离
fn distance_point_aabb(p: Point3<f32>, bb: &Aabb) -> f32 {
    let dx = if p.x < bb.mins.x {
        bb.mins.x - p.x
    } else if p.x > bb.maxs.x {
        p.x - bb.maxs.x
    } else {
        0.0
    };
    let dy = if p.y < bb.mins.y {
        bb.mins.y - p.y
    } else if p.y > bb.maxs.y {
        p.y - bb.maxs.y
    } else {
        0.0
    };
    let dz = if p.z < bb.mins.z {
        bb.mins.z - p.z
    } else if p.z > bb.maxs.z {
        p.z - bb.maxs.z
    } else {
        0.0
    };
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_latest_path() {
        // 只在有 latest.json 时测试
        if Path::new("assets/web_duckdb/latest.json").exists() {
            let path = get_latest_duckdb_path().unwrap();
            assert!(path.exists());
        }
    }
}

