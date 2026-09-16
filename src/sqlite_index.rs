use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "sqlite-index")]
use rusqlite::{Connection, Result, params};

/// 索引内容版本号。
///
/// 上层（如空间查询的分页快照缓存）会把结果缓存起来复用，但没有别的办法察觉
/// 索引在背后被改写。任何写入路径 bump 一次，缓存把它放进键里就能自然失效。
static INDEX_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 当前索引内容版本号。
pub fn index_generation() -> u64 {
    INDEX_GENERATION.load(Ordering::Relaxed)
}

/// 声明索引内容已被改写。
pub fn bump_index_generation() {
    INDEX_GENERATION.fetch_add(1, Ordering::Relaxed);
}

// Minimal SQLite-based AABB index using the SQLite RTree virtual table.
// This module is feature-gated behind `sqlite-index` and can be integrated
// incrementally without impacting existing backends.
pub struct SqliteAabbIndex {
    path: PathBuf,
}

#[cfg(feature = "sqlite-index")]
impl SqliteAabbIndex {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let this = SqliteAabbIndex {
            path: path.as_ref().to_path_buf(),
        };
        let conn = Connection::open(&this.path)?;
        Self::configure(&conn)?;
        drop(conn);
        Ok(this)
    }

    fn configure(conn: &Connection) -> Result<()> {
        // WAL for multi-reader concurrency; NORMAL sync for performance.
        conn.pragma_update(None, "journal_mode", &"WAL")?;
        conn.pragma_update(None, "synchronous", &"NORMAL")?;
        Ok(())
    }

    pub fn init_schema(&self) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY,
                noun TEXT,
                spec_value INTEGER NOT NULL DEFAULT 0,
                dbnum INTEGER,
                name TEXT
            );
            -- 3D AABB RTree: id, [min_x, max_x], [min_y, max_y], [min_z, max_z]
            CREATE VIRTUAL TABLE IF NOT EXISTS aabb_index USING rtree(
                id, min_x, max_x, min_y, max_y, min_z, max_z
            );
            "#,
        )?;
        // 兼容旧数据库文件：如果 items 只有 id 列，这条语句会失败；忽略即可。
        let _ = conn.execute("ALTER TABLE items ADD COLUMN noun TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE items ADD COLUMN spec_value INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE items ADD COLUMN dbnum INTEGER", []);
        let _ = conn.execute("ALTER TABLE items ADD COLUMN name TEXT", []);
        Ok(())
    }

    /// 批量写入 items.name（构件名称），用于名称回填任务。
    ///
    /// AABB 刷新链路只产出几何与 noun/spec_value，名称来自模型库，
    /// 因此名称通过独立的回填步骤写入，不影响既有的索引重建流程。
    pub fn update_item_names<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare("UPDATE items SET name = ?2 WHERE id = ?1")?;
            for (id, name) in iter {
                count += stmt.execute(params![id, name])?;
            }
        }
        tx.commit()?;
        if count > 0 {
            bump_index_generation();
        }
        Ok(count)
    }

    /// 取出尚未回填名称的 item id（用于增量回填）。
    pub fn ids_missing_names(&self, limit: usize) -> Result<Vec<i64>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM items WHERE name IS NULL OR name = '' ORDER BY id LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| row.get::<_, i64>(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    /// 统计名称回填进度：(已命名数量, 总数量)。
    pub fn name_coverage(&self) -> Result<(usize, usize)> {
        let conn = Connection::open(&self.path)?;
        let named: i64 = conn.query_row(
            "SELECT COUNT(1) FROM items WHERE name IS NOT NULL AND name != ''",
            [],
            |row| row.get(0),
        )?;
        let total: i64 = conn.query_row("SELECT COUNT(1) FROM items", [], |row| row.get(0))?;
        Ok((named as usize, total as usize))
    }

    // Batch insert/replace AABBs: (id, min_x, max_x, min_y, max_y, min_z, max_z)
    pub fn insert_many<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, f64, f64, f64, f64, f64, f64)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO aabb_index \
                 (id, min_x, max_x, min_y, max_y, min_z, max_z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (id, minx, maxx, miny, maxy, minz, maxz) in iter {
                stmt.execute(params![id, minx, maxx, miny, maxy, minz, maxz])?;
            }
        } // stmt 在这里被销毁，释放对 tx 的借用
        tx.commit()?;
        Ok(1)
    }

    // AABB intersection query: returns matching ids.
    pub fn query_intersect(
        &self,
        minx: f64,
        maxx: f64,
        miny: f64,
        maxy: f64,
        minz: f64,
        maxz: f64,
    ) -> Result<Vec<i64>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM aabb_index \
             WHERE min_x <= ?2 AND max_x >= ?1 \
               AND min_y <= ?4 AND max_y >= ?3 \
               AND min_z <= ?6 AND max_z >= ?5",
        )?;
        let rows = stmt.query_map((minx, maxx, miny, maxy, minz, maxz), |row| {
            row.get::<_, i64>(0)
        })?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    // Optional: range query on X for scanning.
    pub fn query_range_x(&self, minx: f64, maxx: f64) -> Result<Vec<i64>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM aabb_index \
             WHERE min_x <= ?2 AND max_x >= ?1",
        )?;
        let rows = stmt.query_map((minx, maxx), |row| row.get::<_, i64>(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    // Query all AABBs: returns all (id, min_x, max_x, min_y, max_y, min_z, max_z) tuples
    pub fn query_all_aabbs(&self) -> Result<Vec<(i64, f64, f64, f64, f64, f64, f64)>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt =
            conn.prepare("SELECT id, min_x, max_x, min_y, max_y, min_z, max_z FROM aabb_index")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })?;
        let mut aabbs = Vec::new();
        for r in rows {
            aabbs.push(r?);
        }
        Ok(aabbs)
    }

    /// 批量插入 items 表（id, noun）
    pub fn insert_items<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO items (id, noun, spec_value) VALUES (?1, ?2, 0)",
            )?;
            for (id, noun) in iter {
                stmt.execute(params![id, noun])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// 批量插入 AABB 和 items（合并事务）
    pub fn insert_aabbs_with_items<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String, f64, f64, f64, f64, f64, f64)>,
    {
        self.insert_aabbs_with_items_and_spec_values(iter.into_iter().map(
            |(id, noun, minx, maxx, miny, maxy, minz, maxz)| {
                (id, noun, 0_i64, minx, maxx, miny, maxy, minz, maxz)
            },
        ))
    }

    pub fn insert_aabbs_with_items_and_spec_values<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String, i64, f64, f64, f64, f64, f64, f64)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        let mut count = 0;
        {
            let mut aabb_stmt = tx.prepare(
                "INSERT OR REPLACE INTO aabb_index \
                 (id, min_x, max_x, min_y, max_y, min_z, max_z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut item_stmt = tx.prepare(
                "INSERT OR REPLACE INTO items (id, noun, spec_value) VALUES (?1, ?2, ?3)",
            )?;
            for (id, noun, spec_value, minx, maxx, miny, maxy, minz, maxz) in iter {
                aabb_stmt.execute(params![id, minx, maxx, miny, maxy, minz, maxz])?;
                item_stmt.execute(params![id, noun, spec_value])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn replace_dbnum_aabbs_with_items_and_spec_values<I>(
        &self,
        dbnum: u32,
        iter: I,
    ) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String, i64, f64, f64, f64, f64, f64, f64)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        self.init_schema()?;

        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM aabb_index WHERE id IN (SELECT id FROM items WHERE dbnum = ?1)",
            params![dbnum],
        )?;
        tx.execute("DELETE FROM items WHERE dbnum = ?1", params![dbnum])?;

        // Clean up legacy Parquet imports that encoded the export dbnum into the
        // RTree id instead of preserving the real refno_u64.
        let start = ((dbnum as u64) << 32) as i64;
        let end = ((((dbnum as u64) + 1) << 32) - 1) as i64;
        tx.execute(
            "DELETE FROM aabb_index WHERE id BETWEEN ?1 AND ?2",
            params![start, end],
        )?;
        tx.execute(
            "DELETE FROM items WHERE id BETWEEN ?1 AND ?2",
            params![start, end],
        )?;

        let mut count = 0;
        {
            let mut aabb_stmt = tx.prepare(
                "INSERT OR REPLACE INTO aabb_index \
                 (id, min_x, max_x, min_y, max_y, min_z, max_z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut item_stmt = tx.prepare(
                "INSERT OR REPLACE INTO items (id, noun, spec_value, dbnum) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (id, noun, spec_value, minx, maxx, miny, maxy, minz, maxz) in iter {
                aabb_stmt.execute(params![id, minx, maxx, miny, maxy, minz, maxz])?;
                item_stmt.execute(params![id, noun, spec_value, dbnum])?;
                count += 1;
            }
        }

        tx.commit()?;
        bump_index_generation();
        Ok(count)
    }
}

// ============================================================================
// instances.json 导入功能
// ============================================================================

/// 从 instances.json 导入空间索引的配置
#[derive(Debug, Clone)]
pub struct ImportConfig {
    /// EQUI 使用粗粒度（Owner AABB）
    pub equi_coarse: bool,
    /// BRAN/HANG 使用细粒度（Children + Tubings AABB）
    pub bran_fine: bool,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            equi_coarse: true,
            bran_fine: true,
        }
    }
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
#[derive(Debug, Clone, Copy)]
struct ParquetAabbBounds {
    minx: f64,
    maxx: f64,
    miny: f64,
    maxy: f64,
    minz: f64,
    maxz: f64,
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
impl ParquetAabbBounds {
    fn merge(&mut self, other: Self) {
        self.minx = self.minx.min(other.minx);
        self.maxx = self.maxx.max(other.maxx);
        self.miny = self.miny.min(other.miny);
        self.maxy = self.maxy.max(other.maxy);
        self.minz = self.minz.min(other.minz);
        self.maxz = self.maxz.max(other.maxz);
    }

    fn as_sqlite_parts(self) -> (f64, f64, f64, f64, f64, f64) {
        (
            self.minx, self.maxx, self.miny, self.maxy, self.minz, self.maxz,
        )
    }
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
struct OwnerAabbAggregate {
    noun: String,
    bounds: ParquetAabbBounds,
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
type SqliteAabbRow = (i64, String, i64, f64, f64, f64, f64, f64, f64);

/// 将 refno 字符串（如 "17496_170764"）转换为 i64
/// 格式：(dbnum << 32) + refno
pub fn refno_str_to_i64(refno: &str) -> Option<i64> {
    // 兼容 "dbnum_refno" 与 "dbnum/refno" 两种格式
    let sep = if refno.contains('_') {
        '_'
    } else if refno.contains('/') {
        '/'
    } else {
        return None;
    };
    let parts: Vec<&str> = refno.split(sep).collect();
    if parts.len() != 2 {
        return None;
    }
    let dbnum: u32 = parts[0].parse().ok()?;
    let refno: u32 = parts[1].parse().ok()?;
    Some(((dbnum as u64) << 32 | refno as u64) as i64)
}

/// 将 i64 转换回 refno 字符串
pub fn i64_to_refno_str(id: i64) -> String {
    let id = id as u64;
    let dbnum = (id >> 32) as u32;
    let refno = (id & 0xFFFFFFFF) as u32;
    format!("{}_{}", dbnum, refno)
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_string_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
    path: &Path,
) -> anyhow::Result<&'a arrow_array::StringArray> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::StringArray>())
        .ok_or_else(|| anyhow::anyhow!("{} 缺少 Utf8 列 `{}`", path.display(), name))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_f64_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
    path: &Path,
) -> anyhow::Result<&'a arrow_array::Float64Array> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::Float64Array>())
        .ok_or_else(|| anyhow::anyhow!("{} 缺少 Float64 列 `{}`", path.display(), name))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_u64_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
    path: &Path,
) -> anyhow::Result<&'a arrow_array::UInt64Array> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::UInt64Array>())
        .ok_or_else(|| anyhow::anyhow!("{} 缺少 UInt64 列 `{}`", path.display(), name))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_optional_u64_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
    path: &Path,
) -> anyhow::Result<Option<&'a arrow_array::UInt64Array>> {
    match batch.column_by_name(name) {
        Some(column) => column
            .as_any()
            .downcast_ref::<arrow_array::UInt64Array>()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{} 列 `{}` 不是 UInt64", path.display(), name)),
        None => Ok(None),
    }
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_u32_column<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
    path: &Path,
) -> anyhow::Result<&'a arrow_array::UInt32Array> {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<arrow_array::UInt32Array>())
        .ok_or_else(|| anyhow::anyhow!("{} 缺少 UInt32 列 `{}`", path.display(), name))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_required_string<'a>(
    column: &'a arrow_array::StringArray,
    row: usize,
    column_name: &str,
    path: &Path,
) -> anyhow::Result<&'a str> {
    use arrow_array::Array;

    if column.is_null(row) {
        anyhow::bail!("{} 第 {} 行 `{}` 为空", path.display(), row, column_name);
    }
    Ok(column.value(row))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_optional_string<'a>(
    column: &'a arrow_array::StringArray,
    row: usize,
) -> Option<&'a str> {
    use arrow_array::Array;

    if column.is_null(row) {
        None
    } else {
        Some(column.value(row))
    }
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_required_f64(
    column: &arrow_array::Float64Array,
    row: usize,
    column_name: &str,
    path: &Path,
) -> anyhow::Result<f64> {
    use arrow_array::Array;

    if column.is_null(row) {
        anyhow::bail!("{} 第 {} 行 `{}` 为空", path.display(), row, column_name);
    }
    Ok(column.value(row))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_required_u64(
    column: &arrow_array::UInt64Array,
    row: usize,
    column_name: &str,
    path: &Path,
) -> anyhow::Result<u64> {
    use arrow_array::Array;

    if column.is_null(row) {
        anyhow::bail!("{} 第 {} 行 `{}` 为空", path.display(), row, column_name);
    }
    Ok(column.value(row))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_optional_u64(column: &arrow_array::UInt64Array, row: usize) -> Option<u64> {
    use arrow_array::Array;

    if column.is_null(row) {
        None
    } else {
        Some(column.value(row))
    }
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
fn parquet_required_u32(
    column: &arrow_array::UInt32Array,
    row: usize,
    column_name: &str,
    path: &Path,
) -> anyhow::Result<u32> {
    use arrow_array::Array;

    if column.is_null(row) {
        anyhow::bail!("{} 第 {} 行 `{}` 为空", path.display(), row, column_name);
    }
    Ok(column.value(row))
}

#[cfg(all(feature = "sqlite-index", feature = "parquet-export"))]
impl SqliteAabbIndex {
    pub fn refresh_dbnum_from_parquet_dir<P: AsRef<Path>>(
        &self,
        dbnum: u32,
        parquet_dir: P,
    ) -> anyhow::Result<ImportStats> {
        use anyhow::Context;
        use std::collections::HashMap;

        let parquet_dir = parquet_dir.as_ref();
        let aabb_path = parquet_dir.join("aabb.parquet");
        let instances_path = parquet_dir.join("instances.parquet");
        let tubings_path = parquet_dir.join("tubings.parquet");

        if !aabb_path.exists() {
            anyhow::bail!("aabb.parquet 不存在: {}", aabb_path.display());
        }
        if !instances_path.exists() {
            anyhow::bail!("instances.parquet 不存在: {}", instances_path.display());
        }
        if !tubings_path.exists() {
            anyhow::bail!("tubings.parquet 不存在: {}", tubings_path.display());
        }

        let aabb_by_hash = Self::read_parquet_aabb_table(&aabb_path)?;
        let mut rows: HashMap<i64, SqliteAabbRow> = HashMap::new();
        let mut owner_aggs: HashMap<i64, OwnerAabbAggregate> = HashMap::new();
        let mut owner_specs: HashMap<i64, i64> = HashMap::new();
        let mut nouns_by_refno: HashMap<String, String> = HashMap::new();
        let mut stats = ImportStats::default();

        Self::read_parquet_instances(
            dbnum,
            &instances_path,
            &aabb_by_hash,
            &mut rows,
            &mut owner_aggs,
            &mut owner_specs,
            &mut nouns_by_refno,
            &mut stats,
        )?;
        Self::read_parquet_tubings(
            dbnum,
            &tubings_path,
            &aabb_by_hash,
            &nouns_by_refno,
            &mut rows,
            &mut owner_aggs,
            &mut stats,
        )?;

        for (owner_id, agg) in owner_aggs {
            let spec_value = owner_specs.get(&owner_id).copied().unwrap_or(0);
            let should_insert = matches!(agg.noun.as_str(), "BRAN" | "HANG")
                || (agg.noun == "EQUI" && !rows.contains_key(&owner_id));
            if should_insert {
                let (minx, maxx, miny, maxy, minz, maxz) = agg.bounds.as_sqlite_parts();
                if agg.noun == "EQUI" {
                    stats.equi_count += 1;
                }
                rows.insert(
                    owner_id,
                    (
                        owner_id, agg.noun, spec_value, minx, maxx, miny, maxy, minz, maxz,
                    ),
                );
            }
        }

        let mut items: Vec<_> = rows.into_values().collect();
        items.sort_by_key(|row| row.0);
        let inserted = self
            .replace_dbnum_aabbs_with_items_and_spec_values(dbnum, items)
            .with_context(|| {
                format!(
                    "从 Parquet 刷新 SQLite spatial index 失败: dbnum={}, dir={}",
                    dbnum,
                    parquet_dir.display()
                )
            })?;
        stats.unique_count = inserted;
        stats.total_inserted = inserted;
        Ok(stats)
    }

    fn read_parquet_aabb_table(
        path: &Path,
    ) -> anyhow::Result<std::collections::HashMap<String, ParquetAabbBounds>> {
        use anyhow::Context;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::collections::HashMap;
        use std::fs::File;

        let file = File::open(path)
            .with_context(|| format!("打开 aabb.parquet 失败: {}", path.display()))?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .with_context(|| format!("读取 aabb.parquet metadata 失败: {}", path.display()))?
            .build()
            .with_context(|| format!("创建 aabb.parquet reader 失败: {}", path.display()))?;

        let mut map = HashMap::new();
        for batch in reader {
            let batch = batch
                .with_context(|| format!("读取 aabb.parquet batch 失败: {}", path.display()))?;
            let hash_col = parquet_string_column(&batch, "aabb_hash", path)?;
            let min_x_col = parquet_f64_column(&batch, "min_x", path)?;
            let min_y_col = parquet_f64_column(&batch, "min_y", path)?;
            let min_z_col = parquet_f64_column(&batch, "min_z", path)?;
            let max_x_col = parquet_f64_column(&batch, "max_x", path)?;
            let max_y_col = parquet_f64_column(&batch, "max_y", path)?;
            let max_z_col = parquet_f64_column(&batch, "max_z", path)?;

            for row in 0..batch.num_rows() {
                let hash = parquet_required_string(hash_col, row, "aabb_hash", path)?;
                let bounds = ParquetAabbBounds {
                    minx: parquet_required_f64(min_x_col, row, "min_x", path)?,
                    maxx: parquet_required_f64(max_x_col, row, "max_x", path)?,
                    miny: parquet_required_f64(min_y_col, row, "min_y", path)?,
                    maxy: parquet_required_f64(max_y_col, row, "max_y", path)?,
                    minz: parquet_required_f64(min_z_col, row, "min_z", path)?,
                    maxz: parquet_required_f64(max_z_col, row, "max_z", path)?,
                };
                Self::validate_bounds(hash, bounds, path)?;
                map.insert(hash.to_string(), bounds);
            }
        }

        Ok(map)
    }

    fn read_parquet_instances(
        dbnum: u32,
        path: &Path,
        aabb_by_hash: &std::collections::HashMap<String, ParquetAabbBounds>,
        rows: &mut std::collections::HashMap<i64, SqliteAabbRow>,
        owner_aggs: &mut std::collections::HashMap<i64, OwnerAabbAggregate>,
        owner_specs: &mut std::collections::HashMap<i64, i64>,
        nouns_by_refno: &mut std::collections::HashMap<String, String>,
        stats: &mut ImportStats,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::fs::File;

        let file = File::open(path)
            .with_context(|| format!("打开 instances.parquet 失败: {}", path.display()))?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .with_context(|| format!("读取 instances.parquet metadata 失败: {}", path.display()))?
            .build()
            .with_context(|| format!("创建 instances.parquet reader 失败: {}", path.display()))?;

        for batch in reader {
            let batch = batch.with_context(|| {
                format!("读取 instances.parquet batch 失败: {}", path.display())
            })?;
            let refno_col = parquet_string_column(&batch, "refno_str", path)?;
            let noun_col = parquet_string_column(&batch, "noun", path)?;
            let owner_refno_col = parquet_string_column(&batch, "owner_refno_str", path)?;
            let owner_noun_col = parquet_string_column(&batch, "owner_noun", path)?;
            let aabb_hash_col = parquet_string_column(&batch, "aabb_hash", path)?;
            let spec_value_col = parquet_u64_column(&batch, "spec_value", path)?;
            let dbnum_col = parquet_u32_column(&batch, "dbnum", path)?;
            let refno_u64_col = parquet_optional_u64_column(&batch, "refno_u64", path)?;
            let owner_refno_u64_col = parquet_optional_u64_column(&batch, "owner_refno_u64", path)?;

            for row in 0..batch.num_rows() {
                let row_dbnum = parquet_required_u32(dbnum_col, row, "dbnum", path)?;
                if row_dbnum != dbnum {
                    anyhow::bail!(
                        "{} 第 {} 行 dbnum={}，期望 dbnum={}",
                        path.display(),
                        row,
                        row_dbnum,
                        dbnum
                    );
                }

                let refno = parquet_required_string(refno_col, row, "refno_str", path)?;
                let noun = parquet_required_string(noun_col, row, "noun", path)?;
                let refno_u64 = refno_u64_col.and_then(|column| parquet_optional_u64(column, row));
                let id = Self::required_refno_id(refno, refno_u64, dbnum, path, row)?;
                let spec_value =
                    Self::required_spec_value(spec_value_col, row, "spec_value", path)?;
                let aabb_hash = parquet_required_string(aabb_hash_col, row, "aabb_hash", path)?;
                if aabb_hash.trim().is_empty() {
                    stats.skipped_empty_aabb_count += 1;
                    continue;
                }
                let bounds = *aabb_by_hash.get(aabb_hash).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{} 第 {} 行 aabb_hash={} 未在 aabb.parquet 中找到",
                        path.display(),
                        row,
                        aabb_hash
                    )
                })?;

                let (minx, maxx, miny, maxy, minz, maxz) = bounds.as_sqlite_parts();
                rows.insert(
                    id,
                    (
                        id,
                        noun.to_string(),
                        spec_value,
                        minx,
                        maxx,
                        miny,
                        maxy,
                        minz,
                        maxz,
                    ),
                );
                stats.children_count += 1;
                owner_specs.insert(id, spec_value);
                nouns_by_refno.insert(refno.to_string(), noun.to_string());

                if let Some(owner_refno) = parquet_optional_string(owner_refno_col, row)
                    .filter(|value| !value.trim().is_empty())
                {
                    let owner_refno_u64 =
                        owner_refno_u64_col.and_then(|column| parquet_optional_u64(column, row));
                    let owner_noun =
                        parquet_required_string(owner_noun_col, row, "owner_noun", path)?;
                    Self::merge_owner_aggregate(
                        dbnum,
                        owner_refno,
                        owner_refno_u64,
                        owner_noun,
                        bounds,
                        owner_aggs,
                        path,
                        row,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn read_parquet_tubings(
        dbnum: u32,
        path: &Path,
        aabb_by_hash: &std::collections::HashMap<String, ParquetAabbBounds>,
        nouns_by_refno: &std::collections::HashMap<String, String>,
        rows: &mut std::collections::HashMap<i64, SqliteAabbRow>,
        owner_aggs: &mut std::collections::HashMap<i64, OwnerAabbAggregate>,
        stats: &mut ImportStats,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::fs::File;

        let file = File::open(path)
            .with_context(|| format!("打开 tubings.parquet 失败: {}", path.display()))?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .with_context(|| format!("读取 tubings.parquet metadata 失败: {}", path.display()))?
            .build()
            .with_context(|| format!("创建 tubings.parquet reader 失败: {}", path.display()))?;

        for batch in reader {
            let batch = batch
                .with_context(|| format!("读取 tubings.parquet batch 失败: {}", path.display()))?;
            let refno_col = parquet_string_column(&batch, "tubi_refno_str", path)?;
            let owner_refno_col = parquet_string_column(&batch, "owner_refno_str", path)?;
            let aabb_hash_col = parquet_string_column(&batch, "aabb_hash", path)?;
            let spec_value_col = parquet_u64_column(&batch, "spec_value", path)?;
            let dbnum_col = parquet_u32_column(&batch, "dbnum", path)?;
            let refno_u64_col = parquet_optional_u64_column(&batch, "tubi_refno_u64", path)?;
            let owner_refno_u64_col = parquet_optional_u64_column(&batch, "owner_refno_u64", path)?;

            for row in 0..batch.num_rows() {
                let row_dbnum = parquet_required_u32(dbnum_col, row, "dbnum", path)?;
                if row_dbnum != dbnum {
                    anyhow::bail!(
                        "{} 第 {} 行 dbnum={}，期望 dbnum={}",
                        path.display(),
                        row,
                        row_dbnum,
                        dbnum
                    );
                }

                let refno = parquet_required_string(refno_col, row, "tubi_refno_str", path)?;
                let refno_u64 = refno_u64_col.and_then(|column| parquet_optional_u64(column, row));
                let id = Self::required_refno_id(refno, refno_u64, dbnum, path, row)?;
                let spec_value =
                    Self::required_spec_value(spec_value_col, row, "spec_value", path)?;
                let aabb_hash = parquet_required_string(aabb_hash_col, row, "aabb_hash", path)?;
                if aabb_hash.trim().is_empty() {
                    stats.skipped_empty_aabb_count += 1;
                    continue;
                }
                let bounds = *aabb_by_hash.get(aabb_hash).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{} 第 {} 行 aabb_hash={} 未在 aabb.parquet 中找到",
                        path.display(),
                        row,
                        aabb_hash
                    )
                })?;

                let (minx, maxx, miny, maxy, minz, maxz) = bounds.as_sqlite_parts();
                rows.insert(
                    id,
                    (
                        id,
                        "TUBI".to_string(),
                        spec_value,
                        minx,
                        maxx,
                        miny,
                        maxy,
                        minz,
                        maxz,
                    ),
                );
                stats.tubings_count += 1;

                let owner_refno =
                    parquet_required_string(owner_refno_col, row, "owner_refno_str", path)?;
                if let Some(owner_noun) = nouns_by_refno.get(owner_refno) {
                    let owner_refno_u64 =
                        owner_refno_u64_col.and_then(|column| parquet_optional_u64(column, row));
                    Self::merge_owner_aggregate(
                        dbnum,
                        owner_refno,
                        owner_refno_u64,
                        owner_noun,
                        bounds,
                        owner_aggs,
                        path,
                        row,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn validate_bounds(hash: &str, bounds: ParquetAabbBounds, path: &Path) -> anyhow::Result<()> {
        if !bounds.minx.is_finite()
            || !bounds.maxx.is_finite()
            || !bounds.miny.is_finite()
            || !bounds.maxy.is_finite()
            || !bounds.minz.is_finite()
            || !bounds.maxz.is_finite()
        {
            anyhow::bail!("{} aabb_hash={} 包含非有限 bounds", path.display(), hash);
        }
        if bounds.minx > bounds.maxx || bounds.miny > bounds.maxy || bounds.minz > bounds.maxz {
            anyhow::bail!("{} aabb_hash={} bounds min/max 非法", path.display(), hash);
        }
        Ok(())
    }

    fn sqlite_id_from_refno_u64(refno_u64: u64, path: &Path, row: usize) -> anyhow::Result<i64> {
        i64::try_from(refno_u64).map_err(|_| {
            anyhow::anyhow!(
                "{} 第 {} 行 refno_u64={} 超过 SQLite RTree id 范围",
                path.display(),
                row,
                refno_u64
            )
        })
    }

    fn required_refno_id(
        refno: &str,
        refno_u64: Option<u64>,
        _dbnum: u32,
        path: &Path,
        row: usize,
    ) -> anyhow::Result<i64> {
        if let Some(refno_u64) = refno_u64 {
            return Self::sqlite_id_from_refno_u64(refno_u64, path, row);
        }

        let id = refno_str_to_i64(refno).ok_or_else(|| {
            anyhow::anyhow!("{} 第 {} 行 refno 格式非法: {}", path.display(), row, refno)
        })?;
        Ok(id)
    }

    fn required_spec_value(
        column: &arrow_array::UInt64Array,
        row: usize,
        column_name: &str,
        path: &Path,
    ) -> anyhow::Result<i64> {
        let value = parquet_required_u64(column, row, column_name, path)?;
        i64::try_from(value).map_err(|_| {
            anyhow::anyhow!(
                "{} 第 {} 行 `{}`={} 超过 i64::MAX",
                path.display(),
                row,
                column_name,
                value
            )
        })
    }

    fn merge_owner_aggregate(
        dbnum: u32,
        owner_refno: &str,
        owner_refno_u64: Option<u64>,
        owner_noun: &str,
        bounds: ParquetAabbBounds,
        owner_aggs: &mut std::collections::HashMap<i64, OwnerAabbAggregate>,
        path: &Path,
        row: usize,
    ) -> anyhow::Result<()> {
        if !matches!(owner_noun, "BRAN" | "HANG" | "EQUI") {
            return Ok(());
        }
        let owner_id = Self::required_refno_id(owner_refno, owner_refno_u64, dbnum, path, row)?;
        owner_aggs
            .entry(owner_id)
            .and_modify(|agg| agg.bounds.merge(bounds))
            .or_insert_with(|| OwnerAabbAggregate {
                noun: owner_noun.to_string(),
                bounds,
            });
        Ok(())
    }
}

#[cfg(feature = "sqlite-index")]
impl SqliteAabbIndex {
    /// 从 instances.json 文件导入空间索引
    pub fn import_from_instances_json(
        &self,
        json_path: &Path,
        config: &ImportConfig,
    ) -> anyhow::Result<ImportStats> {
        use std::fs::File;
        use std::io::BufReader;

        let file = File::open(json_path)
            .map_err(|e| anyhow::anyhow!("打开文件失败: {}: {}", json_path.display(), e))?;
        let reader = BufReader::new(file);
        let json: serde_json::Value = serde_json::from_reader(reader)
            .map_err(|e| anyhow::anyhow!("解析 JSON 失败: {}", e))?;

        self.import_from_json_value_with_path(&json, config, Some(json_path))
    }

    /// 从 JSON Value 导入空间索引
    pub fn import_from_json_value(
        &self,
        json: &serde_json::Value,
        config: &ImportConfig,
    ) -> anyhow::Result<ImportStats> {
        self.import_from_json_value_with_path(json, config, None)
    }

    fn import_from_json_value_with_path(
        &self,
        json: &serde_json::Value,
        config: &ImportConfig,
        json_path: Option<&Path>,
    ) -> anyhow::Result<ImportStats> {
        use std::collections::{HashMap, HashSet};

        let mut stats = ImportStats::default();

        let groups = json["groups"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("JSON 缺少 groups 数组"))?;

        // 旧格式：group/child 上直接携带 aabb: {min,max}
        let looks_like_inline_aabb = groups.iter().any(|g| g.get("owner_aabb").is_some())
            || groups
                .iter()
                .flat_map(|g| {
                    g.get("children")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                })
                .any(|c| c.get("aabb").is_some());

        if looks_like_inline_aabb {
            // ========================
            // 旧格式导入逻辑（min/max 直写）
            // ========================
            let mut aabb_map: HashMap<i64, (String, i64, f64, f64, f64, f64, f64, f64)> =
                HashMap::new();

            for group in groups {
                let owner_noun = group["owner_noun"].as_str().unwrap_or("");

                match owner_noun {
                    "EQUI" if config.equi_coarse => {
                        if let Some(item) = Self::extract_owner_aabb(group) {
                            Self::merge_aabb(&mut aabb_map, item);
                            stats.equi_count += 1;
                        }
                    }
                    "BRAN" | "HANG" if config.bran_fine => {
                        Self::extract_children_aabbs_merged(group, &mut aabb_map, &mut stats);
                        Self::extract_tubings_aabbs_merged(group, &mut aabb_map, &mut stats);
                        Self::extract_group_parts_owner_aabb_merged(group, &mut aabb_map);
                    }
                    _ => {}
                }
            }

            let aabb_items: Vec<_> = aabb_map
                .into_iter()
                .map(
                    |(id, (noun, spec_value, minx, maxx, miny, maxy, minz, maxz))| {
                        (id, noun, spec_value, minx, maxx, miny, maxy, minz, maxz)
                    },
                )
                .collect();
            stats.unique_count = aabb_items.len();
            if !aabb_items.is_empty() {
                self.insert_aabbs_with_items_and_spec_values(aabb_items)?;
            }
            stats.total_inserted = stats.unique_count;
            return Ok(stats);
        }

        // ========================
        // 新格式导入逻辑：AABB 去重表 aabb.json + aabb_hash 引用
        // ========================
        let aabb_table = {
            let Some(json_path) = json_path else {
                return Err(anyhow::anyhow!(
                    "instances.json 使用 aabb_hash 格式，但未提供 json_path 上下文，无法定位 aabb.json"
                ));
            };
            let base_dir = json_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("无法获取 instances.json 所在目录"))?;
            let aabb_path = base_dir.join("aabb.json");
            if !aabb_path.exists() {
                return Err(anyhow::anyhow!(
                    "instances.json 使用 aabb_hash 格式，但未找到配套 aabb.json: {}",
                    aabb_path.display()
                ));
            }
            let bytes = std::fs::read(&aabb_path).map_err(|e| {
                anyhow::anyhow!("读取 aabb.json 失败: {}: {}", aabb_path.display(), e)
            })?;
            let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("解析 aabb.json 失败: {}: {}", aabb_path.display(), e)
            })?;
            v
        };

        fn aabb_hash_key(v: &serde_json::Value) -> Option<String> {
            if let Some(s) = v.as_str() {
                return Some(s.to_string());
            }
            if let Some(n) = v.as_u64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_i64() {
                return Some(n.to_string());
            }
            None
        }

        fn aabb_from_table(
            aabb_table: &serde_json::Value,
            hash_value: &serde_json::Value,
        ) -> Option<(f64, f64, f64, f64, f64, f64)> {
            let key = aabb_hash_key(hash_value)?;
            let entry = aabb_table.get(&key)?;
            let min = entry.get("min")?.as_array()?;
            let max = entry.get("max")?.as_array()?;
            if min.len() < 3 || max.len() < 3 {
                return None;
            }
            Some((
                min[0].as_f64()?,
                max[0].as_f64()?,
                min[1].as_f64()?,
                max[1].as_f64()?,
                min[2].as_f64()?,
                max[2].as_f64()?,
            ))
        }

        fn merge_bounds(
            acc: &mut Option<(f64, f64, f64, f64, f64, f64)>,
            b: (f64, f64, f64, f64, f64, f64),
        ) {
            *acc = Some(match acc.take() {
                None => b,
                Some((minx, maxx, miny, maxy, minz, maxz)) => (
                    minx.min(b.0),
                    maxx.max(b.1),
                    miny.min(b.2),
                    maxy.max(b.3),
                    minz.min(b.4),
                    maxz.max(b.5),
                ),
            });
        }

        // 批量插入（避免一次性 Vec 过大）
        const CHUNK: usize = 50_000;
        let mut buf: Vec<(i64, String, i64, f64, f64, f64, f64, f64, f64)> =
            Vec::with_capacity(CHUNK);
        let mut seen: HashSet<i64> = HashSet::new();

        let mut flush = |this: &SqliteAabbIndex,
                         buf: &mut Vec<(i64, String, i64, f64, f64, f64, f64, f64, f64)>|
         -> anyhow::Result<()> {
            if buf.is_empty() {
                return Ok(());
            }
            let items = std::mem::take(buf);
            this.insert_aabbs_with_items_and_spec_values(items)?;
            Ok(())
        };

        // 1) groups：按配置导入
        for group in groups {
            let owner_noun = group
                .get("owner_noun")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let owner_refno = group
                .get("owner_refno")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // EQUI coarse：尝试用 children/tubings 的 AABB 合并近似 owner AABB
            if owner_noun == "EQUI" && config.equi_coarse {
                if let Some(id) = refno_str_to_i64(owner_refno) {
                    let mut merged: Option<(f64, f64, f64, f64, f64, f64)> = None;
                    if let Some(children) = group.get("children").and_then(|v| v.as_array()) {
                        for child in children {
                            if let Some(b) = child
                                .get("aabb_hash")
                                .and_then(|h| aabb_from_table(&aabb_table, h))
                            {
                                merge_bounds(&mut merged, b);
                            }
                        }
                    }
                    if let Some(tubings) = group.get("tubings").and_then(|v| v.as_array()) {
                        for t in tubings {
                            if let Some(b) = t
                                .get("aabb_hash")
                                .and_then(|h| aabb_from_table(&aabb_table, h))
                            {
                                merge_bounds(&mut merged, b);
                            }
                        }
                    }
                    if let Some((minx, maxx, miny, maxy, minz, maxz)) = merged {
                        if seen.insert(id) {
                            stats.unique_count += 1;
                        }
                        stats.equi_count += 1;
                        let spec_value = group
                            .get("spec_value")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        buf.push((
                            id,
                            owner_noun.to_string(),
                            spec_value,
                            minx,
                            maxx,
                            miny,
                            maxy,
                            minz,
                            maxz,
                        ));
                        if buf.len() >= CHUNK {
                            flush(self, &mut buf)?;
                        }
                    }
                }
            }

            // BRAN/HANG fine：children + tubings
            if matches!(owner_noun, "BRAN" | "HANG") && config.bran_fine {
                let mut owner_merged: Option<(f64, f64, f64, f64, f64, f64)> = None;
                if let Some(children) = group.get("children").and_then(|v| v.as_array()) {
                    for child in children {
                        let r = child.get("refno").and_then(|v| v.as_str()).unwrap_or("");
                        let id = match refno_str_to_i64(r) {
                            Some(v) => v,
                            None => continue,
                        };
                        let Some((minx, maxx, miny, maxy, minz, maxz)) = child
                            .get("aabb_hash")
                            .and_then(|h| aabb_from_table(&aabb_table, h))
                        else {
                            continue;
                        };
                        merge_bounds(&mut owner_merged, (minx, maxx, miny, maxy, minz, maxz));
                        if seen.insert(id) {
                            stats.unique_count += 1;
                        }
                        stats.children_count += 1;
                        let spec_value = child
                            .get("spec_value")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        buf.push((
                            id,
                            owner_noun.to_string(),
                            spec_value,
                            minx,
                            maxx,
                            miny,
                            maxy,
                            minz,
                            maxz,
                        ));
                        if buf.len() >= CHUNK {
                            flush(self, &mut buf)?;
                        }
                    }
                }
                if let Some(tubings) = group.get("tubings").and_then(|v| v.as_array()) {
                    for t in tubings {
                        let r = t.get("refno").and_then(|v| v.as_str()).unwrap_or("");
                        let id = match refno_str_to_i64(r) {
                            Some(v) => v,
                            None => continue,
                        };
                        let Some((minx, maxx, miny, maxy, minz, maxz)) = t
                            .get("aabb_hash")
                            .and_then(|h| aabb_from_table(&aabb_table, h))
                        else {
                            continue;
                        };
                        merge_bounds(&mut owner_merged, (minx, maxx, miny, maxy, minz, maxz));
                        if seen.insert(id) {
                            stats.unique_count += 1;
                        }
                        stats.tubings_count += 1;
                        let spec_value = t.get("spec_value").and_then(|v| v.as_i64()).unwrap_or(0);
                        buf.push((
                            id,
                            "TUBI".to_string(),
                            spec_value,
                            minx,
                            maxx,
                            miny,
                            maxy,
                            minz,
                            maxz,
                        ));
                        if buf.len() >= CHUNK {
                            flush(self, &mut buf)?;
                        }
                    }
                }

                if let (Some(owner_id), Some((minx, maxx, miny, maxy, minz, maxz))) =
                    (refno_str_to_i64(owner_refno), owner_merged)
                {
                    let spec_value = group
                        .get("spec_value")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if seen.insert(owner_id) {
                        stats.unique_count += 1;
                    }
                    buf.push((
                        owner_id,
                        owner_noun.to_string(),
                        spec_value,
                        minx,
                        maxx,
                        miny,
                        maxy,
                        minz,
                        maxz,
                    ));
                    if buf.len() >= CHUNK {
                        flush(self, &mut buf)?;
                    }
                }
            }
        }

        // 2) instances：尽量补全（避免漏掉不在 BRAN/HANG/EQUI 分组内的构件）
        if let Some(instances) = json.get("instances").and_then(|v| v.as_array()) {
            for inst in instances {
                let r = inst.get("refno").and_then(|v| v.as_str()).unwrap_or("");
                let id = match refno_str_to_i64(r) {
                    Some(v) => v,
                    None => continue,
                };
                let Some((minx, maxx, miny, maxy, minz, maxz)) = inst
                    .get("aabb_hash")
                    .and_then(|h| aabb_from_table(&aabb_table, h))
                else {
                    continue;
                };
                let noun = inst
                    .get("noun")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("")
                    .to_string();
                if seen.insert(id) {
                    stats.unique_count += 1;
                }
                // instances 不计入 total_inserted 的原三类统计，但对 room 计算很关键
                let spec_value = inst.get("spec_value").and_then(|v| v.as_i64()).unwrap_or(0);
                buf.push((id, noun, spec_value, minx, maxx, miny, maxy, minz, maxz));
                if buf.len() >= CHUNK {
                    flush(self, &mut buf)?;
                }
            }
        }

        flush(self, &mut buf)?;

        stats.total_inserted = stats.unique_count;
        Ok(stats)
    }

    /// 合并 AABB 到 map（取并集）
    fn merge_aabb(
        map: &mut std::collections::HashMap<i64, (String, i64, f64, f64, f64, f64, f64, f64)>,
        item: (i64, String, i64, f64, f64, f64, f64, f64, f64),
    ) {
        let (id, noun, spec_value, minx, maxx, miny, maxy, minz, maxz) = item;
        map.entry(id)
            .and_modify(|e| {
                if e.1 == 0 && spec_value != 0 {
                    e.1 = spec_value;
                }
                e.2 = e.2.min(minx); // min_x
                e.3 = e.3.max(maxx); // max_x
                e.4 = e.4.min(miny); // min_y
                e.5 = e.5.max(maxy); // max_y
                e.6 = e.6.min(minz); // min_z
                e.7 = e.7.max(maxz); // max_z
            })
            .or_insert((noun, spec_value, minx, maxx, miny, maxy, minz, maxz));
    }

    fn extract_owner_aabb(
        group: &serde_json::Value,
    ) -> Option<(i64, String, i64, f64, f64, f64, f64, f64, f64)> {
        let refno = group["owner_refno"].as_str()?;
        let id = refno_str_to_i64(refno)?;
        let noun = group["owner_noun"].as_str().unwrap_or("").to_string();
        let spec_value = group["spec_value"].as_i64().unwrap_or(0);
        let aabb = &group["owner_aabb"];

        if aabb.is_null() {
            return None;
        }

        let min = aabb["min"].as_array()?;
        let max = aabb["max"].as_array()?;

        Some((
            id,
            noun,
            spec_value,
            min[0].as_f64()?,
            max[0].as_f64()?,
            min[1].as_f64()?,
            max[1].as_f64()?,
            min[2].as_f64()?,
            max[2].as_f64()?,
        ))
    }

    fn extract_children_aabbs_merged(
        group: &serde_json::Value,
        map: &mut std::collections::HashMap<i64, (String, i64, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(children) = group["children"].as_array() {
            for child in children {
                if let Some(item) = Self::extract_element_aabb(child) {
                    Self::merge_aabb(map, item);
                    stats.children_count += 1;
                }
            }
        }
    }

    fn extract_tubings_aabbs_merged(
        group: &serde_json::Value,
        map: &mut std::collections::HashMap<i64, (String, i64, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(tubings) = group["tubings"].as_array() {
            for tubi in tubings {
                if let Some(item) = Self::extract_element_aabb(tubi) {
                    Self::merge_aabb(map, item);
                    stats.tubings_count += 1;
                }
            }
        }
    }

    fn extract_group_parts_owner_aabb_merged(
        group: &serde_json::Value,
        map: &mut std::collections::HashMap<i64, (String, i64, f64, f64, f64, f64, f64, f64)>,
    ) {
        let Some(owner_refno) = group["owner_refno"].as_str() else {
            return;
        };
        let Some(owner_id) = refno_str_to_i64(owner_refno) else {
            return;
        };
        let owner_noun = group["owner_noun"].as_str().unwrap_or("").to_string();
        let spec_value = group["spec_value"].as_i64().unwrap_or(0);
        let mut merged: Option<(f64, f64, f64, f64, f64, f64)> = None;

        let mut merge_part = |item: (i64, String, i64, f64, f64, f64, f64, f64, f64)| {
            let (_, _, _, minx, maxx, miny, maxy, minz, maxz) = item;
            match &mut merged {
                Some(acc) => {
                    acc.0 = acc.0.min(minx);
                    acc.1 = acc.1.max(maxx);
                    acc.2 = acc.2.min(miny);
                    acc.3 = acc.3.max(maxy);
                    acc.4 = acc.4.min(minz);
                    acc.5 = acc.5.max(maxz);
                }
                None => merged = Some((minx, maxx, miny, maxy, minz, maxz)),
            }
        };

        if let Some(children) = group["children"].as_array() {
            for child in children {
                if let Some(item) = Self::extract_element_aabb(child) {
                    merge_part(item);
                }
            }
        }
        if let Some(tubings) = group["tubings"].as_array() {
            for tubi in tubings {
                if let Some(item) = Self::extract_element_aabb(tubi) {
                    merge_part(item);
                }
            }
        }

        if let Some((minx, maxx, miny, maxy, minz, maxz)) = merged {
            Self::merge_aabb(
                map,
                (
                    owner_id, owner_noun, spec_value, minx, maxx, miny, maxy, minz, maxz,
                ),
            );
        }
    }

    fn extract_children_aabbs(
        group: &serde_json::Value,
        items: &mut Vec<(i64, String, i64, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(children) = group["children"].as_array() {
            for child in children {
                if let Some(item) = Self::extract_element_aabb(child) {
                    items.push(item);
                    stats.children_count += 1;
                }
            }
        }
    }

    fn extract_tubings_aabbs(
        group: &serde_json::Value,
        items: &mut Vec<(i64, String, i64, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(tubings) = group["tubings"].as_array() {
            for tubi in tubings {
                if let Some(item) = Self::extract_element_aabb(tubi) {
                    items.push(item);
                    stats.tubings_count += 1;
                }
            }
        }
    }

    fn extract_element_aabb(
        elem: &serde_json::Value,
    ) -> Option<(i64, String, i64, f64, f64, f64, f64, f64, f64)> {
        let refno = elem["refno"].as_str()?;
        let id = refno_str_to_i64(refno)?;
        let noun = elem["noun"].as_str().unwrap_or("").to_string();
        let spec_value = elem["spec_value"].as_i64().unwrap_or(0);
        let aabb = &elem["aabb"];

        if aabb.is_null() {
            return None;
        }

        let min = aabb["min"].as_array()?;
        let max = aabb["max"].as_array()?;

        Some((
            id,
            noun,
            spec_value,
            min[0].as_f64()?,
            max[0].as_f64()?,
            min[1].as_f64()?,
            max[1].as_f64()?,
            min[2].as_f64()?,
            max[2].as_f64()?,
        ))
    }
}

/// 导入统计
#[derive(Debug, Default)]
pub struct ImportStats {
    pub equi_count: usize,
    pub children_count: usize,
    pub tubings_count: usize,
    pub skipped_empty_aabb_count: usize,
    pub total_inserted: usize,
    /// 去重后的唯一记录数
    pub unique_count: usize,
}

#[cfg(all(test, feature = "sqlite-index"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn basic_intersect() {
        let path = "test_aabb.sqlite";
        let _ = fs::remove_file(path);
        let idx = SqliteAabbIndex::open(path).unwrap();
        idx.init_schema().unwrap();

        let data = vec![
            (1, 0.0, 10.0, 0.0, 5.0, -5.0, 5.0),
            (2, 5.0, 15.0, 2.0, 8.0, -2.0, 2.0),
            (3, 20.0, 30.0, -1.0, 1.0, 0.0, 1.0),
        ];
        idx.insert_many(data).unwrap();

        let ids = idx.query_intersect(4.0, 6.0, 1.0, 3.0, -1.0, 1.0).unwrap();
        assert!(ids.contains(&1) && ids.contains(&2) && !ids.contains(&3));

        let _ = fs::remove_file(path);
    }

    #[cfg(feature = "parquet-export")]
    #[test]
    fn refresh_dbnum_from_parquet_dir_replaces_only_target_dbnum_and_merges_owner() {
        use arrow_array::{
            ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array, UInt64Array,
        };
        use arrow_schema::{DataType, Field, Schema};
        use parquet::arrow::ArrowWriter;
        use rusqlite::Connection;
        use std::sync::Arc;

        fn write_batch(path: &Path, batch: RecordBatch) {
            let file = fs::File::create(path).unwrap();
            let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
            writer.write(&batch).unwrap();
            writer.close().unwrap();
        }

        let temp = tempfile::tempdir().unwrap();
        let parquet_dir = temp.path().join("parquet").join("24383");
        fs::create_dir_all(&parquet_dir).unwrap();

        let aabb_schema = Arc::new(Schema::new(vec![
            Field::new("aabb_hash", DataType::Utf8, false),
            Field::new("min_x", DataType::Float64, false),
            Field::new("min_y", DataType::Float64, false),
            Field::new("min_z", DataType::Float64, false),
            Field::new("max_x", DataType::Float64, false),
            Field::new("max_y", DataType::Float64, false),
            Field::new("max_z", DataType::Float64, false),
        ]));
        write_batch(
            &parquet_dir.join("aabb.parquet"),
            RecordBatch::try_new(
                aabb_schema,
                vec![
                    Arc::new(StringArray::from(vec!["owner_direct", "child", "tubi"])) as ArrayRef,
                    Arc::new(Float64Array::from(vec![100.0, 0.0, 2.0])) as ArrayRef,
                    Arc::new(Float64Array::from(vec![100.0, 0.0, 2.0])) as ArrayRef,
                    Arc::new(Float64Array::from(vec![100.0, 0.0, 2.0])) as ArrayRef,
                    Arc::new(Float64Array::from(vec![101.0, 1.0, 3.0])) as ArrayRef,
                    Arc::new(Float64Array::from(vec![101.0, 1.0, 3.0])) as ArrayRef,
                    Arc::new(Float64Array::from(vec![101.0, 1.0, 3.0])) as ArrayRef,
                ],
            )
            .unwrap(),
        );

        let instances_schema = Arc::new(Schema::new(vec![
            Field::new("refno_str", DataType::Utf8, false),
            Field::new("noun", DataType::Utf8, false),
            Field::new("owner_refno_str", DataType::Utf8, true),
            Field::new("owner_noun", DataType::Utf8, false),
            Field::new("aabb_hash", DataType::Utf8, false),
            Field::new("spec_value", DataType::UInt64, false),
            Field::new("dbnum", DataType::UInt32, false),
        ]));
        write_batch(
            &parquet_dir.join("instances.parquet"),
            RecordBatch::try_new(
                instances_schema,
                vec![
                    Arc::new(StringArray::from(vec!["24383_100", "24383_101"])) as ArrayRef,
                    Arc::new(StringArray::from(vec!["BRAN", "PIPE"])) as ArrayRef,
                    Arc::new(StringArray::from(vec![None, Some("24383_100")])) as ArrayRef,
                    Arc::new(StringArray::from(vec!["", "BRAN"])) as ArrayRef,
                    Arc::new(StringArray::from(vec!["owner_direct", "child"])) as ArrayRef,
                    Arc::new(UInt64Array::from(vec![42_u64, 7_u64])) as ArrayRef,
                    Arc::new(UInt32Array::from(vec![24383_u32, 24383_u32])) as ArrayRef,
                ],
            )
            .unwrap(),
        );

        let tubings_schema = Arc::new(Schema::new(vec![
            Field::new("tubi_refno_str", DataType::Utf8, false),
            Field::new("owner_refno_str", DataType::Utf8, false),
            Field::new("aabb_hash", DataType::Utf8, false),
            Field::new("spec_value", DataType::UInt64, false),
            Field::new("dbnum", DataType::UInt32, false),
        ]));
        write_batch(
            &parquet_dir.join("tubings.parquet"),
            RecordBatch::try_new(
                tubings_schema,
                vec![
                    Arc::new(StringArray::from(vec!["24383_201"])) as ArrayRef,
                    Arc::new(StringArray::from(vec!["24383_100"])) as ArrayRef,
                    Arc::new(StringArray::from(vec!["tubi"])) as ArrayRef,
                    Arc::new(UInt64Array::from(vec![8_u64])) as ArrayRef,
                    Arc::new(UInt32Array::from(vec![24383_u32])) as ArrayRef,
                ],
            )
            .unwrap(),
        );

        let sqlite_path = temp.path().join("spatial_index.sqlite");
        let idx = SqliteAabbIndex::open(&sqlite_path).unwrap();
        idx.init_schema().unwrap();

        let foreign_id = refno_str_to_i64("24384_1").unwrap();
        idx.insert_aabbs_with_items_and_spec_values(vec![(
            foreign_id,
            "FOREIGN".to_string(),
            1,
            -1.0,
            1.0,
            -1.0,
            1.0,
            -1.0,
            1.0,
        )])
        .unwrap();

        let stats = idx
            .refresh_dbnum_from_parquet_dir(24383, &parquet_dir)
            .unwrap();
        assert_eq!(stats.total_inserted, 3);

        let conn = Connection::open(&sqlite_path).unwrap();
        let owner_id = refno_str_to_i64("24383_100").unwrap();
        let owner: (String, i64, f64, f64, f64, f64, f64, f64) = conn
            .query_row(
                "SELECT items.noun, items.spec_value, min_x, max_x, min_y, max_y, min_z, max_z \
                 FROM aabb_index JOIN items USING(id) WHERE id = ?1",
                [owner_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(owner.0, "BRAN");
        assert_eq!(owner.1, 42);
        assert_eq!(
            (owner.2, owner.3, owner.4, owner.5, owner.6, owner.7),
            (0.0, 3.0, 0.0, 3.0, 0.0, 3.0)
        );

        let foreign_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM aabb_index WHERE id = ?1",
                [foreign_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(foreign_count, 1);
    }
}
