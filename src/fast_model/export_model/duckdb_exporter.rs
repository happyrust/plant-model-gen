//! DuckDB Stream Writer - 直接批量写入模式
//!
//! 在模型生成过程中直接将 ShapeInstancesData 写入 DuckDB，
//! 使用 Appender 实现高性能批量插入。

#[cfg(feature = "duckdb-feature")]
use anyhow::{Context, Result};
#[cfg(feature = "duckdb-feature")]
use duckdb::Connection;
#[cfg(feature = "duckdb-feature")]
use std::collections::HashSet;
#[cfg(feature = "duckdb-feature")]
use std::path::{Path, PathBuf};
#[cfg(feature = "duckdb-feature")]
use std::sync::Mutex;

#[cfg(feature = "duckdb-feature")]
use aios_core::geometry::ShapeInstancesData;

/// DuckDB 写入模式
#[cfg(feature = "duckdb-feature")]
#[derive(Debug, Clone, Copy)]
pub enum DuckDBWriteMode {
    /// 全量重建（删除旧文件）
    Rebuild,
    /// 追加写入（保留旧文件）
    Append,
}

/// DuckDB 流式写入器
/// 
/// 在模型生成过程中直接将数据写入 DuckDB，使用 Appender 实现高性能批量插入。
#[cfg(feature = "duckdb-feature")]
pub struct DuckDBStreamWriter {
    conn: Mutex<Connection>,
    output_path: PathBuf,
    spatial_enabled: bool,
    aabb_has_bbox: bool,
    aabb_has_dbno: bool,
}

#[cfg(feature = "duckdb-feature")]
impl DuckDBStreamWriter {
    /// 创建新的流式写入器
    /// 
    /// # Arguments
    /// * `output_dir` - DuckDB 输出目录
    /// * `mode` - 写入模式（重建或追加）
    pub fn new(output_dir: impl AsRef<Path>, mode: DuckDBWriteMode) -> Result<Self> {
        let output_path = output_dir.as_ref().join("model.duckdb");
        
        // 确保目录存在
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 删除旧文件（全量重建）
        if matches!(mode, DuckDBWriteMode::Rebuild) && output_path.exists() {
            std::fs::remove_file(&output_path)?;
        }

        let conn = Connection::open(&output_path)
            .context("Failed to open DuckDB database")?;

        let spatial_enabled = Self::try_enable_spatial(&conn);
        let (aabb_has_bbox, aabb_has_dbno) = Self::ensure_schema(&conn, spatial_enabled)?;

        println!("📦 [DuckDB] 创建流式写入器: {:?}", output_path);

        Ok(Self {
            conn: Mutex::new(conn),
            output_path,
            spatial_enabled,
            aabb_has_bbox,
            aabb_has_dbno,
        })
    }

    fn try_enable_spatial(conn: &Connection) -> bool {
        if conn.execute_batch("LOAD spatial;").is_ok() {
            return true;
        }

        if conn.execute_batch("INSTALL spatial;").is_ok() {
            return conn.execute_batch("LOAD spatial;").is_ok();
        }

        false
    }

    fn get_table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
        let mut columns = HashSet::new();
        let mut stmt = conn.prepare(
            "SELECT column_name FROM information_schema.columns WHERE table_name = ?",
        )?;
        let rows = stmt.query_map([table], |row| row.get::<_, String>(0))?;
        for row in rows {
            columns.insert(row?);
        }
        Ok(columns)
    }

    /// 初始化/迁移 Schema
    fn ensure_schema(conn: &Connection, spatial_enabled: bool) -> Result<(bool, bool)> {
        // instance 表
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS instance (
                refno           VARCHAR PRIMARY KEY,
                noun            VARCHAR,
                owner_refno     VARCHAR,
                color_index     INTEGER,
                spec_value      INTEGER
            );
            "#,
        )?;

        // geo 表
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS geo (
                id              VARCHAR PRIMARY KEY,
                refno           VARCHAR NOT NULL,
                geo_hash        VARCHAR NOT NULL,
                geo_transform   VARCHAR
            );
            "#,
        )?;

        let columns = Self::get_table_columns(conn, "aabb")?;
        let mut aabb_has_bbox = columns.contains("bbox");
        let mut aabb_has_dbno = columns.contains("dbnum");

        if columns.is_empty() {
            if spatial_enabled {
                conn.execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS aabb (
                        refno   VARCHAR PRIMARY KEY,
                        dbnum    INTEGER,
                        min_x   DOUBLE,
                        min_y   DOUBLE,
                        min_z   DOUBLE,
                        max_x   DOUBLE,
                        max_y   DOUBLE,
                        max_z   DOUBLE,
                        bbox    GEOMETRY
                    );
                    "#,
                )?;
                aabb_has_bbox = true;
                aabb_has_dbno = true;
            } else {
                conn.execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS aabb (
                        refno   VARCHAR PRIMARY KEY,
                        dbnum    INTEGER,
                        min_x   DOUBLE,
                        min_y   DOUBLE,
                        min_z   DOUBLE,
                        max_x   DOUBLE,
                        max_y   DOUBLE,
                        max_z   DOUBLE
                    );
                    "#,
                )?;
                aabb_has_dbno = true;
            }
        } else {
            if !aabb_has_dbno {
                conn.execute_batch("ALTER TABLE aabb ADD COLUMN dbnum INTEGER;")?;
                aabb_has_dbno = true;
            }

            if spatial_enabled && !aabb_has_bbox {
                conn.execute_batch("ALTER TABLE aabb ADD COLUMN bbox GEOMETRY;")?;
                aabb_has_bbox = true;
            }

            if aabb_has_dbno {
                let _ = conn.execute_batch(
                    "UPDATE aabb SET dbnum = CAST(split_part(refno, '_', 1) AS INTEGER) WHERE dbnum IS NULL;",
                );
            }

            if spatial_enabled && aabb_has_bbox {
                let _ = conn.execute_batch(
                    "UPDATE aabb SET bbox = ST_MakeEnvelope(min_x, min_y, max_x, max_y) WHERE bbox IS NULL;",
                );
            }
        }

        Ok((aabb_has_bbox, aabb_has_dbno))
    }

    /// 写入一批 ShapeInstancesData
    pub fn write_batch(&self, data: &ShapeInstancesData) -> Result<(usize, usize, usize)> {
        let conn = self.conn.lock().unwrap();
        
        let mut instance_count = 0;
        let mut geo_count = 0;
        let mut aabb_count = 0;
        let use_bbox = self.spatial_enabled && self.aabb_has_bbox;
        let use_dbno = self.aabb_has_dbno;

        // 使用事务批量写入
        conn.execute_batch("BEGIN TRANSACTION")?;

        // 1. 写入 instance 和 geo
        for (refno, info) in &data.inst_info_map {
            let refno_str = refno.to_string();
            let dbnum = refno.refno().get_0() as i32;
            let noun = info.generic_type.to_string();
            let owner_refno = if info.owner_refno != *refno {
                Some(info.owner_refno.to_string())
            } else {
                None
            };

            // 插入 instance
            conn.execute(
                "INSERT OR REPLACE INTO instance (refno, noun, owner_refno, color_index, spec_value) VALUES (?1, ?2, ?3, ?4, ?5)",
                duckdb::params![
                    &refno_str,
                    noun,
                    owner_refno,
                    0i32, // color_index
                    Option::<i32>::None // spec_value
                ],
            )?;
            instance_count += 1;

            // 写入 aabb
            if let Some(aabb) = &info.aabb {
                let min_x = aabb.mins.x as f64;
                let min_y = aabb.mins.y as f64;
                let min_z = aabb.mins.z as f64;
                let max_x = aabb.maxs.x as f64;
                let max_y = aabb.maxs.y as f64;
                let max_z = aabb.maxs.z as f64;

                if use_bbox {
                    if use_dbno {
                        conn.execute(
                            "INSERT OR REPLACE INTO aabb (refno, dbnum, min_x, min_y, min_z, max_x, max_y, max_z, bbox) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ST_MakeEnvelope(?9, ?10, ?11, ?12))",
                            duckdb::params![
                                &refno_str,
                                dbnum,
                                min_x,
                                min_y,
                                min_z,
                                max_x,
                                max_y,
                                max_z,
                                min_x,
                                min_y,
                                max_x,
                                max_y
                            ],
                        )?;
                    } else {
                        conn.execute(
                            "INSERT OR REPLACE INTO aabb (refno, min_x, min_y, min_z, max_x, max_y, max_z, bbox) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ST_MakeEnvelope(?8, ?9, ?10, ?11))",
                            duckdb::params![
                                &refno_str,
                                min_x,
                                min_y,
                                min_z,
                                max_x,
                                max_y,
                                max_z,
                                min_x,
                                min_y,
                                max_x,
                                max_y
                            ],
                        )?;
                    }
                } else if use_dbno {
                    conn.execute(
                        "INSERT OR REPLACE INTO aabb (refno, dbnum, min_x, min_y, min_z, max_x, max_y, max_z) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        duckdb::params![
                            &refno_str,
                            dbnum,
                            min_x,
                            min_y,
                            min_z,
                            max_x,
                            max_y,
                            max_z
                        ],
                    )?;
                } else {
                    conn.execute(
                        "INSERT OR REPLACE INTO aabb (refno, min_x, min_y, min_z, max_x, max_y, max_z) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        duckdb::params![
                            &refno_str,
                            min_x,
                            min_y,
                            min_z,
                            max_x,
                            max_y,
                            max_z
                        ],
                    )?;
                }
                aabb_count += 1;
            }
        }

        // 2. 写入 geo (从 inst_geos_map)
        for (_geo_key, geo_data) in &data.inst_geos_map {
            let refno_str = geo_data.refno.to_string();
            
            for (idx, geo) in geo_data.insts.iter().enumerate() {
                let geo_id = format!("{}_{}", refno_str, idx);
                let geo_hash = &geo.geo_hash;
                
                // 序列化变换矩阵
                let transform = geo.geo_transform;
                let transform_json = serde_json::json!({
                    "rotation": [transform.rotation.x, transform.rotation.y, transform.rotation.z, transform.rotation.w],
                    "scale": [transform.scale.x, transform.scale.y, transform.scale.z],
                    "translation": [transform.translation.x, transform.translation.y, transform.translation.z]
                }).to_string();

                conn.execute(
                    "INSERT OR REPLACE INTO geo (id, refno, geo_hash, geo_transform) VALUES (?1, ?2, ?3, ?4)",
                    duckdb::params![geo_id, refno_str, geo_hash, transform_json],
                )?;
                geo_count += 1;
            }
        }

        // 3. 写入 tubi 数据
        for (refno, tubi_info) in &data.inst_tubi_map {
            let refno_str = refno.to_string();
            let dbnum = refno.refno().get_0() as i32;
            let noun = "TUBI";
            let owner_refno = if tubi_info.owner_refno != *refno {
                Some(tubi_info.owner_refno.to_string())
            } else {
                None
            };

            conn.execute(
                "INSERT OR REPLACE INTO instance (refno, noun, owner_refno, color_index, spec_value) VALUES (?1, ?2, ?3, ?4, ?5)",
                duckdb::params![&refno_str, noun, owner_refno, 0i32, Option::<i32>::None],
            )?;
            instance_count += 1;

            if let Some(aabb) = &tubi_info.aabb {
                let min_x = aabb.mins.x as f64;
                let min_y = aabb.mins.y as f64;
                let min_z = aabb.mins.z as f64;
                let max_x = aabb.maxs.x as f64;
                let max_y = aabb.maxs.y as f64;
                let max_z = aabb.maxs.z as f64;

                if use_bbox {
                    if use_dbno {
                        conn.execute(
                            "INSERT OR REPLACE INTO aabb (refno, dbnum, min_x, min_y, min_z, max_x, max_y, max_z, bbox) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ST_MakeEnvelope(?9, ?10, ?11, ?12))",
                            duckdb::params![
                                &refno_str,
                                dbnum,
                                min_x,
                                min_y,
                                min_z,
                                max_x,
                                max_y,
                                max_z,
                                min_x,
                                min_y,
                                max_x,
                                max_y
                            ],
                        )?;
                    } else {
                        conn.execute(
                            "INSERT OR REPLACE INTO aabb (refno, min_x, min_y, min_z, max_x, max_y, max_z, bbox) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ST_MakeEnvelope(?8, ?9, ?10, ?11))",
                            duckdb::params![
                                &refno_str,
                                min_x,
                                min_y,
                                min_z,
                                max_x,
                                max_y,
                                max_z,
                                min_x,
                                min_y,
                                max_x,
                                max_y
                            ],
                        )?;
                    }
                } else if use_dbno {
                    conn.execute(
                        "INSERT OR REPLACE INTO aabb (refno, dbnum, min_x, min_y, min_z, max_x, max_y, max_z) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        duckdb::params![
                            &refno_str,
                            dbnum,
                            min_x,
                            min_y,
                            min_z,
                            max_x,
                            max_y,
                            max_z
                        ],
                    )?;
                } else {
                    conn.execute(
                        "INSERT OR REPLACE INTO aabb (refno, min_x, min_y, min_z, max_x, max_y, max_z) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        duckdb::params![
                            &refno_str,
                            min_x,
                            min_y,
                            min_z,
                            max_x,
                            max_y,
                            max_z
                        ],
                    )?;
                }
                aabb_count += 1;
            }
        }

        conn.execute_batch("COMMIT")?;

        Ok((instance_count, geo_count, aabb_count))
    }

    /// 完成写入并创建索引
    pub fn finalize(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        println!("🔍 [DuckDB] 创建索引...");

        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_instance_noun ON instance(noun);
            CREATE INDEX IF NOT EXISTS idx_geo_refno ON geo(refno);
            CREATE INDEX IF NOT EXISTS idx_geo_hash ON geo(geo_hash);
            "#,
        )?;
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_aabb_z ON aabb(min_z, max_z);")?;
        if self.aabb_has_dbno {
            conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_aabb_dbno ON aabb(dbnum);")?;
        }
        if self.spatial_enabled && self.aabb_has_bbox {
            conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_aabb_rtree ON aabb USING RTREE (bbox);")?;
        }

        // 获取统计信息
        let instance_count: i64 = conn.query_row("SELECT COUNT(*) FROM instance", [], |row| row.get(0))?;
        let geo_count: i64 = conn.query_row("SELECT COUNT(*) FROM geo", [], |row| row.get(0))?;
        let aabb_count: i64 = conn.query_row("SELECT COUNT(*) FROM aabb", [], |row| row.get(0))?;

        // 释放连接锁以获取文件大小
        drop(conn);

        // 获取文件大小
        let file_size = std::fs::metadata(&self.output_path)?.len();

        // 生成 duckdb_meta.json (本地元数据)
        let mut aabb_schema = vec![
            "refno",
            "min_x",
            "min_y",
            "min_z",
            "max_x",
            "max_y",
            "max_z",
        ];
        if self.aabb_has_dbno {
            aabb_schema.insert(1, "dbnum");
        }
        if self.aabb_has_bbox {
            aabb_schema.push("bbox");
        }

        let meta = serde_json::json!({
            "version": "1.0.0",
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "database": {
                "path": "model.duckdb",
                "size_bytes": file_size,
            },
            "tables": {
                "instance": { "count": instance_count },
                "geo": { "count": geo_count },
                "aabb": { "count": aabb_count },
            },
            "schema": {
                "instance": ["refno", "noun", "owner_refno", "color_index", "spec_value"],
                "geo": ["id", "refno", "geo_hash", "geo_transform"],
                "aabb": aabb_schema,
            }
        });

        let meta_path = self.output_path.parent().unwrap().join("duckdb_meta.json");
        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

        // === 拷贝到 web 目录并生成 latest.json ===
        self.publish_to_web_dir(instance_count, geo_count, aabb_count, file_size)?;

        println!(
            "✅ [DuckDB] 完成: instance={}, geo={}, aabb={}, size={}KB",
            instance_count, geo_count, aabb_count, file_size / 1024
        );
        println!("   📄 元数据: {:?}", meta_path);

        Ok(())
    }

    /// 拷贝 DuckDB 到 web 目录并生成 latest.json
    fn publish_to_web_dir(&self, instance_count: i64, geo_count: i64, aabb_count: i64, file_size: u64) -> Result<()> {
        // web 目录：assets/web_duckdb/
        let web_dir = Path::new("assets").join("web_duckdb");
        std::fs::create_dir_all(&web_dir)?;

        // 生成带时间戳的文件名
        let now = chrono::Local::now();
        let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
        let db_filename = format!("model_{}.duckdb", timestamp);
        let dest_path = web_dir.join(&db_filename);

        // 拷贝文件
        std::fs::copy(&self.output_path, &dest_path)?;

        // 生成 latest.json
        let updated_at = now.timestamp();
        let latest = serde_json::json!({
            "db_filename": db_filename,
            "updated_at": updated_at,
            "stats": {
                "instance_count": instance_count,
                "geo_count": geo_count,
                "aabb_count": aabb_count,
                "size_bytes": file_size,
            }
        });

        let latest_path = web_dir.join("latest.json");
        std::fs::write(&latest_path, serde_json::to_string_pretty(&latest)?)?;

        println!("📤 [DuckDB] 发布到 web 目录:");
        println!("   - 文件: {:?}", dest_path);
        println!("   - latest.json: {:?}", latest_path);

        // 清理旧文件（保留最近 3 个版本）
        self.cleanup_old_versions(&web_dir, 3)?;

        Ok(())
    }

    /// 清理旧版本文件
    fn cleanup_old_versions(&self, web_dir: &Path, keep_count: usize) -> Result<()> {
        let mut db_files: Vec<_> = std::fs::read_dir(web_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().map(|ext| ext == "duckdb").unwrap_or(false)
            })
            .collect();

        if db_files.len() <= keep_count {
            return Ok(());
        }

        // 按修改时间排序（最新在前）
        db_files.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        // 删除超出保留数量的旧文件
        for file in db_files.into_iter().skip(keep_count) {
            if let Err(e) = std::fs::remove_file(file.path()) {
                eprintln!("⚠️  清理旧版本失败: {:?} - {}", file.path(), e);
            } else {
                println!("🗑️  清理旧版本: {:?}", file.path());
            }
        }

        Ok(())
    }

    /// 获取输出路径
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }
}

#[cfg(test)]
#[cfg(feature = "duckdb-feature")]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_stream_writer_creation() {
        let temp_dir = std::env::temp_dir().join("duckdb_test");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let writer = DuckDBStreamWriter::new(&temp_dir, DuckDBWriteMode::Rebuild).unwrap();
        assert!(writer.output_path().exists());
        
        writer.finalize().unwrap();
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
