//! DuckDB Writer for Model Export
//!
//! This module provides functionality to write model data to DuckDB files,
//! supporting HTTP Range Requests for efficient frontend loading via DuckDB-WASM.
//!
//! Uses DuckDB's `Appender` for high-performance bulk inserts instead of individual SQL statements.

#[cfg(feature = "duckdb-feature")]
use anyhow::{Context, Result};
#[cfg(feature = "duckdb-feature")]
use duckdb::{params, Appender, Connection};
#[cfg(feature = "duckdb-feature")]
use std::path::Path;

/// Model data writer using DuckDB storage format.
/// 
/// Schema:
/// - trans: Transformation matrices (hash -> d[16])
/// - aabb: Bounding boxes (hash -> bbox GEOMETRY, min_z, max_z)
/// - instances: Model instances (refno -> trans_hash, aabb_hash, ...)
/// - geos: Geometry references (refno -> geo_hash, geo_transform)
#[cfg(feature = "duckdb-feature")]
pub struct DuckDBWriter {
    conn: Connection,
}

#[cfg(feature = "duckdb-feature")]
impl DuckDBWriter {
    /// Create a new DuckDB writer, creating or opening the database file.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .context("Failed to open DuckDB database")?;
        
        let writer = Self { conn };
        writer.init_schema()?;
        
        Ok(writer)
    }

    /// Initialize the database schema with all required tables and indexes.
    fn init_schema(&self) -> Result<()> {
        // Load spatial extension for R-Tree support
        self.conn.execute_batch(
            r#"
            INSTALL spatial;
            LOAD spatial;
            "#,
        ).ok(); // Ignore if already installed

        // Create trans table - 与 SurrealDB 定义保持一致: rotation[4], scale[3], translation[3]
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS trans (
                hash        VARCHAR,
                rotation    VARCHAR,    -- [x, y, z, w] quaternion (JSON)
                scale       VARCHAR,    -- [x, y, z] (JSON)
                translation VARCHAR     -- [x, y, z] (JSON)
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_trans_hash ON trans(hash);
            "#,
        )?;

        // Create aabb table - 保留原始坐标，不使用 GEOMETRY（Appender 不支持 ST_MakeEnvelope）
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS aabb (
                hash    VARCHAR,
                min_x   DOUBLE,
                min_y   DOUBLE,
                min_z   DOUBLE,
                max_x   DOUBLE,
                max_y   DOUBLE,
                max_z   DOUBLE
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_aabb_hash ON aabb(hash);
            CREATE INDEX IF NOT EXISTS idx_aabb_z ON aabb(min_z, max_z);
            "#,
        )?;

        // Create instance table
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS instance (
                refno           VARCHAR,
                noun            VARCHAR,
                name            VARCHAR,
                trans_hash      VARCHAR,
                aabb_hash       VARCHAR,
                color_index     INTEGER,
                lod_mask        INTEGER,
                spec_value      INTEGER,
                properties      VARCHAR
            );
            
            CREATE UNIQUE INDEX IF NOT EXISTS idx_instance_refno ON instance(refno);
            CREATE INDEX IF NOT EXISTS idx_noun ON instance(noun);
            CREATE INDEX IF NOT EXISTS idx_trans_hash ON instance(trans_hash);
            CREATE INDEX IF NOT EXISTS idx_aabb_hash ON instance(aabb_hash);
            "#,
        )?;

        // Create geo table - id 为 refno_number 格式，geo_transform 存储为 JSON 字符串
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS geo (
                id            VARCHAR PRIMARY KEY,
                refno         VARCHAR NOT NULL,
                geo_hash      VARCHAR NOT NULL,
                geo_transform VARCHAR
            );
            
            CREATE INDEX IF NOT EXISTS idx_geo_refno ON geo(refno);
            CREATE INDEX IF NOT EXISTS idx_geo_geo_hash ON geo(geo_hash);
            "#,
        )?;

        Ok(())
    }

    /// Get connection reference for creating appenders
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // ==================== Bulk Insert with Appender ====================

    /// Create a bulk appender for trans table
    pub fn trans_appender(&self) -> Result<Appender<'_>> {
        self.conn.appender("trans").context("Failed to create trans appender")
    }

    /// Create a bulk appender for aabb table
    pub fn aabb_appender(&self) -> Result<Appender<'_>> {
        self.conn.appender("aabb").context("Failed to create aabb appender")
    }

    /// Create a bulk appender for instance table
    pub fn instance_appender(&self) -> Result<Appender<'_>> {
        self.conn.appender("instance").context("Failed to create instance appender")
    }

    /// Create a bulk appender for geo table
    pub fn geo_appender(&self) -> Result<Appender<'_>> {
        self.conn.appender("geo").context("Failed to create geo appender")
    }

    /// Bulk insert transformation matrices
    /// TransRow: hash, rotation[4], scale[3], translation[3]
    pub fn bulk_insert_trans<'a, I>(&self, items: I) -> Result<usize>
    where
        I: IntoIterator<Item = TransRow<'a>>,
    {
        let mut appender = self.trans_appender()?;
        let mut count = 0;
        for row in items {
            // 序列化为 JSON 字符串
            let rotation_json = serde_json::to_string(row.rotation).unwrap_or_default();
            let scale_json = serde_json::to_string(row.scale).unwrap_or_default();
            let translation_json = serde_json::to_string(row.translation).unwrap_or_default();
            appender.append_row(params![row.hash, rotation_json, scale_json, translation_json])?;
            count += 1;
        }
        appender.flush()?;
        Ok(count)
    }

    /// Bulk insert bounding boxes
    pub fn bulk_insert_aabb<'a, I>(&self, items: I) -> Result<usize>
    where
        I: IntoIterator<Item = (&'a str, f64, f64, f64, f64, f64, f64)>,
    {
        let mut appender = self.aabb_appender()?;
        let mut count = 0;
        for (hash, min_x, min_y, min_z, max_x, max_y, max_z) in items {
            appender.append_row(params![hash, min_x, min_y, min_z, max_x, max_y, max_z])?;
            count += 1;
        }
        appender.flush()?;
        Ok(count)
    }

    /// Bulk insert instances
    pub fn bulk_insert_instances<'a, I>(&self, items: I) -> Result<usize>
    where
        I: IntoIterator<Item = InstanceRow<'a>>,
    {
        let mut appender = self.instance_appender()?;
        let mut count = 0;
        for row in items {
            appender.append_row(params![
                row.refno,
                row.noun,
                row.name,
                row.trans_hash,
                row.aabb_hash,
                row.color_index,
                row.lod_mask,
                row.spec_value,
                row.properties
            ])?;
            count += 1;
        }
        appender.flush()?;
        Ok(count)
    }

    /// Bulk insert geometry references
    /// GeoRow.number 用于生成 id = refno_number
    pub fn bulk_insert_geos<'a, I>(&self, items: I) -> Result<usize>
    where
        I: IntoIterator<Item = GeoRow<'a>>,
    {
        let mut appender = self.geo_appender()?;
        let mut count = 0;
        for row in items {
            // id = refno_number
            let id = format!("{}_{}", row.refno, row.number);
            // geo_transform 序列化为 JSON 字符串
            let transform_json = serde_json::to_string(row.geo_transform).unwrap_or_default();
            appender.append_row(params![
                id,
                row.refno,
                row.geo_hash,
                transform_json
            ])?;
            count += 1;
        }
        appender.flush()?;
        Ok(count)
    }

    // ==================== Query Methods ====================

    /// Check which refnos already exist in the database.
    pub fn get_existing_refnos(&self, refnos: &[&str]) -> Result<Vec<String>> {
        if refnos.is_empty() {
            return Ok(vec![]);
        }

        let placeholders: Vec<&str> = refnos.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT refno FROM instance WHERE refno IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn duckdb::ToSql> = refnos.iter().map(|s| s as &dyn duckdb::ToSql).collect();
        
        let rows = stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0))?;
        
        let mut existing = Vec::new();
        for row in rows {
            existing.push(row?);
        }
        
        Ok(existing)
    }

    /// Get total instance count.
    pub fn get_instance_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM instance",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Close the connection (drop will also close it).
    pub fn close(self) -> Result<()> {
        // Connection is closed on drop
        Ok(())
    }
}

// ==================== Row Types for Bulk Insert ====================

/// Row data for trans table (与 SurrealDB trans 表保持一致)
#[cfg(feature = "duckdb-feature")]
pub struct TransRow<'a> {
    pub hash: &'a str,
    pub rotation: &'a [f32; 4],     // quaternion [x, y, z, w]
    pub scale: &'a [f32; 3],        // [x, y, z]
    pub translation: &'a [f32; 3], // [x, y, z]
}

/// Row data for instance table
#[cfg(feature = "duckdb-feature")]
pub struct InstanceRow<'a> {
    pub refno: &'a str,
    pub noun: Option<&'a str>,
    pub name: Option<&'a str>,
    pub trans_hash: Option<&'a str>,
    pub aabb_hash: Option<&'a str>,
    pub color_index: Option<i32>,
    pub lod_mask: Option<i32>,
    pub spec_value: Option<i32>,
    pub properties: Option<&'a str>,
}

/// Row data for geos table
/// id = refno_number (e.g., "12345_67890_0")
#[cfg(feature = "duckdb-feature")]
pub struct GeoRow<'a> {
    pub refno: &'a str,
    pub number: usize,              // 该实例下的几何体序号 (0, 1, 2...)
    pub geo_hash: &'a str,
    pub geo_transform: &'a [f64; 16],
}

#[cfg(test)]
#[cfg(feature = "duckdb-feature")]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_duckdb_writer_bulk_insert() {
        let path = "test_model_bulk.duckdb";
        let _ = fs::remove_file(path);

        let writer = DuckDBWriter::new(path).unwrap();

        // Bulk insert trans
        let rot = [0.0f32, 0.0, 0.0, 1.0];
        let scale = [1.0f32, 1.0, 1.0];
        let trans1 = [100.0f32, 200.0, 300.0];
        let trans2 = [400.0f32, 500.0, 600.0];
        let trans_data = vec![
            TransRow { hash: "hash_trans_1", rotation: &rot, scale: &scale, translation: &trans1 },
            TransRow { hash: "hash_trans_2", rotation: &rot, scale: &scale, translation: &trans2 },
        ];
        let trans_count = writer.bulk_insert_trans(trans_data).unwrap();
        assert_eq!(trans_count, 2);

        // Bulk insert aabb
        let aabb_data = vec![
            ("hash_aabb_1", 0.0, 0.0, 0.0, 10.0, 10.0, 5.0),
            ("hash_aabb_2", 5.0, 5.0, 1.0, 15.0, 15.0, 8.0),
        ];
        let aabb_count = writer.bulk_insert_aabb(aabb_data).unwrap();
        assert_eq!(aabb_count, 2);

        // Bulk insert instances
        let identity = [1.0f64; 16];
        let instances = vec![
            InstanceRow {
                refno: "12345_67890",
                noun: Some("EQUI"),
                name: Some("Equipment1"),
                trans_hash: Some("hash_trans_1"),
                aabb_hash: Some("hash_aabb_1"),
                color_index: Some(1),
                lod_mask: Some(7),
                spec_value: Some(100),
                properties: None,
            },
            InstanceRow {
                refno: "12345_67891",
                noun: Some("PIPE"),
                name: Some("Pipe1"),
                trans_hash: Some("hash_trans_2"),
                aabb_hash: Some("hash_aabb_2"),
                color_index: Some(2),
                lod_mask: Some(3),
                spec_value: Some(50),
                properties: None,
            },
        ];
        let inst_count = writer.bulk_insert_instances(instances).unwrap();
        assert_eq!(inst_count, 2);

        // Bulk insert geos
        let geos = vec![
            GeoRow {
                refno: "12345_67890",
                number: 0,
                geo_hash: "geo_abc",
                geo_transform: &identity,
            },
            GeoRow {
                refno: "12345_67890",
                number: 1,
                geo_hash: "geo_def",
                geo_transform: &identity,
            },
        ];
        let geo_count = writer.bulk_insert_geos(geos).unwrap();
        assert_eq!(geo_count, 2);

        // Verify count
        assert_eq!(writer.get_instance_count().unwrap(), 2);

        // Check existing refnos
        let existing = writer.get_existing_refnos(&["12345_67890", "99999_99999"]).unwrap();
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0], "12345_67890");

        writer.close().unwrap();
        let _ = fs::remove_file(path);
    }
}
