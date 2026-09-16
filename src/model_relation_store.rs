use aios_core::RefnoEnum;
/// 模型关系数据的 SQLite 集中存储
///
/// 替代分散在 SurrealDB 多表中的关系数据，简化 regen 清理逻辑
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

/// 按 dbnum 分片的模型关系存储
pub struct ModelRelationStore {
    base_path: PathBuf,
}

impl ModelRelationStore {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    pub fn db_dir(&self, dbnum: u32) -> PathBuf {
        self.base_path.join(format!("{dbnum}"))
    }

    /// 获取指定 dbnum 的连接（懒加载）
    fn get_conn(&self, dbnum: u32) -> Result<Connection> {
        let db_path = self.db_dir(dbnum).join("relations.db");
        std::fs::create_dir_all(db_path.parent().unwrap())?;

        let conn =
            Connection::open(&db_path).with_context(|| format!("打开 SQLite: {:?}", db_path))?;

        // 性能优化配置
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000;
             PRAGMA temp_store = MEMORY;",
        )?;

        self.init_schema(&conn)?;
        Ok(conn)
    }

    /// 初始化表结构
    fn init_schema(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS inst_relate (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dbnum INTEGER NOT NULL,
                refno INTEGER NOT NULL,
                inst_id INTEGER NOT NULL,
                parent_refno INTEGER,
                world_matrix BLOB,
                name TEXT,
                noun TEXT,
                identity_source TEXT,
                resolved INTEGER DEFAULT 0,
                attrs_json TEXT,
                created_at INTEGER DEFAULT (unixepoch())
            );
            CREATE INDEX IF NOT EXISTS idx_refno ON inst_relate(refno);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_inst_id_unique ON inst_relate(inst_id);

            CREATE TABLE IF NOT EXISTS geo_relate (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                inst_id INTEGER NOT NULL,
                geo_hash INTEGER NOT NULL,
                FOREIGN KEY (inst_id) REFERENCES inst_relate(inst_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_geo_inst ON geo_relate(inst_id);
            CREATE INDEX IF NOT EXISTS idx_geo_hash ON geo_relate(geo_hash);

            CREATE TABLE IF NOT EXISTS inst_geo (
                hash INTEGER PRIMARY KEY,
                geometry BLOB NOT NULL,
                aabb_min_x REAL, aabb_min_y REAL, aabb_min_z REAL,
                aabb_max_x REAL, aabb_max_y REAL, aabb_max_z REAL,
                meshed INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS tubi_relate (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                branch_refno INTEGER NOT NULL,
                segment_data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tubi_branch ON tubi_relate(branch_refno);

            CREATE TABLE IF NOT EXISTS inst_relate_bool (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                carrier_refno INTEGER NOT NULL,
                bool_result BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_bool_carrier ON inst_relate_bool(carrier_refno);",
        )?;
        Self::ensure_inst_relate_columns(conn)?;
        Ok(())
    }

    /// spec 009:旧库幂等迁移——为 inst_relate 补 name/noun/identity_source/resolved 列。
    /// 新建库由 CREATE TABLE 直接包含;已存在的旧库按 PRAGMA table_info 检测缺失列补齐。
    fn ensure_inst_relate_columns(conn: &Connection) -> Result<()> {
        let existing: Vec<String> = conn
            .prepare("PRAGMA table_info(inst_relate)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<_, _>>()?;
        let add = |col: &str, ddl: &str| -> Result<()> {
            if !existing.iter().any(|c| c == col) {
                conn.execute(&format!("ALTER TABLE inst_relate ADD COLUMN {ddl}"), [])?;
            }
            Ok(())
        };
        add("name", "name TEXT")?;
        add("noun", "noun TEXT")?;
        add("identity_source", "identity_source TEXT")?;
        add("resolved", "resolved INTEGER DEFAULT 0")?;
        add("attrs_json", "attrs_json TEXT")?;
        Ok(())
    }

    /// 清理指定 refnos 的所有关联数据（核心简化逻辑）
    pub fn cleanup_by_refnos(&self, dbnum: u32, refnos: &[RefnoEnum]) -> Result<usize> {
        if refnos.is_empty() {
            return Ok(0);
        }

        let conn = self.get_conn(dbnum)?;
        let refno_list = refnos
            .iter()
            .map(|r| r.refno().0.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let mut total_deleted = 0;

        // 利用 FOREIGN KEY CASCADE 自动删除 geo_relate
        let deleted = conn.execute(
            &format!("DELETE FROM inst_relate WHERE refno IN ({})", refno_list),
            [],
        )?;
        total_deleted += deleted;

        // 删除 tubi_relate
        let deleted = conn.execute(
            &format!(
                "DELETE FROM tubi_relate WHERE branch_refno IN ({})",
                refno_list
            ),
            [],
        )?;
        total_deleted += deleted;

        // 删除 inst_relate_bool
        let deleted = conn.execute(
            &format!(
                "DELETE FROM inst_relate_bool WHERE carrier_refno IN ({})",
                refno_list
            ),
            [],
        )?;
        total_deleted += deleted;

        Ok(total_deleted)
    }

    /// 清理指定名称的旧 inst_relate 记录。
    ///
    /// RVM 导入可能在解析规则修复后把同一个节点映射到不同 refno；仅按新 refno
    /// 清理会留下旧错误身份记录，因此导入前也按本轮节点名做一次幂等清理。
    pub fn cleanup_inst_relates_by_names(&self, dbnum: u32, names: &[String]) -> Result<usize> {
        if names.is_empty() {
            return Ok(0);
        }

        let conn = self.get_conn(dbnum)?;
        let mut stmt =
            conn.prepare_cached("DELETE FROM inst_relate WHERE dbnum = ?1 AND name = ?2")?;
        let mut total_deleted = 0usize;
        for name in names {
            total_deleted += stmt.execute(params![dbnum, name])?;
        }
        Ok(total_deleted)
    }

    /// 批量插入 inst_relate
    pub fn insert_inst_relates(&self, dbnum: u32, records: &[InstRelateRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let conn = self.get_conn(dbnum)?;
        let tx = conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO inst_relate
                 (dbnum, refno, inst_id, parent_refno, world_matrix,
                  name, noun, identity_source, resolved, attrs_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;

            for rec in records {
                stmt.execute(params![
                    dbnum,
                    rec.refno.refno().0,
                    rec.inst_id,
                    rec.parent_refno.map(|r| r.refno().0),
                    rec.world_matrix.as_ref(),
                    rec.name.as_deref(),
                    rec.noun.as_deref(),
                    rec.identity_source.as_deref(),
                    rec.resolved as i64,
                    rec.attrs_json.as_deref()
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// 批量插入 geo_relate
    pub fn insert_geo_relates(&self, dbnum: u32, records: &[(u64, u64)]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let conn = self.get_conn(dbnum)?;
        let tx = conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO geo_relate (inst_id, geo_hash) VALUES (?1, ?2)",
            )?;

            for (inst_id, geo_hash) in records {
                stmt.execute(params![inst_id, geo_hash])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// 批量插入 inst_geo
    pub fn insert_inst_geos(&self, dbnum: u32, records: &[InstGeoRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let conn = self.get_conn(dbnum)?;
        let tx = conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO inst_geo
                 (hash, geometry, aabb_min_x, aabb_min_y, aabb_min_z, aabb_max_x, aabb_max_y, aabb_max_z, meshed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            )?;

            for rec in records {
                stmt.execute(params![
                    rec.hash,
                    rec.geometry.as_slice(),
                    rec.aabb_min_x,
                    rec.aabb_min_y,
                    rec.aabb_min_z,
                    rec.aabb_max_x,
                    rec.aabb_max_y,
                    rec.aabb_max_z,
                    if rec.meshed { 1 } else { 0 }
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// 查询 refno 关联的所有 inst_id
    pub fn query_inst_ids_by_refnos(&self, dbnum: u32, refnos: &[RefnoEnum]) -> Result<Vec<u64>> {
        if refnos.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.get_conn(dbnum)?;
        let refno_list = refnos
            .iter()
            .map(|r| r.refno().0.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let mut stmt = conn.prepare(&format!(
            "SELECT inst_id FROM inst_relate WHERE refno IN ({})",
            refno_list
        ))?;

        let inst_ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<u64>, _>>()?;

        Ok(inst_ids)
    }

    /// 查询 RVM/ATT 导入时保存在关系库中的属性。
    pub fn query_attrs_by_refno(
        &self,
        dbnum: u32,
        refno: RefnoEnum,
    ) -> Result<Option<serde_json::Value>> {
        let conn = self.get_conn(dbnum)?;
        let attrs_json: Option<String> = conn
            .query_row(
                "SELECT attrs_json FROM inst_relate WHERE dbnum = ?1 AND refno = ?2 AND attrs_json IS NOT NULL LIMIT 1",
                params![dbnum, refno.refno().0],
                |row| row.get(0),
            )
            .optional()?;

        let Some(attrs_json) = attrs_json else {
            return Ok(None);
        };
        let attrs =
            serde_json::from_str(&attrs_json).context("解析 inst_relate.attrs_json 失败")?;
        Ok(Some(attrs))
    }

    /// 获取统计信息
    pub fn get_stats(&self, dbnum: u32) -> Result<StoreStats> {
        let conn = self.get_conn(dbnum)?;

        let inst_relate_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM inst_relate", [], |row| row.get(0))?;

        let geo_relate_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM geo_relate", [], |row| row.get(0))?;

        let inst_geo_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM inst_geo", [], |row| row.get(0))?;

        Ok(StoreStats {
            inst_relate_count: inst_relate_count as usize,
            geo_relate_count: geo_relate_count as usize,
            inst_geo_count: inst_geo_count as usize,
        })
    }

    pub fn query_rvm_root_name(&self, dbnum: u32) -> Result<Option<String>> {
        let conn = self.get_conn(dbnum)?;
        let geometry_blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT geometry FROM inst_geo ORDER BY hash LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;

        let Some(geometry_blob) = geometry_blob else {
            return Ok(None);
        };

        let payload: serde_json::Value =
            serde_json::from_slice(&geometry_blob).context("解析 inst_geo.geometry 失败")?;
        let root_name = payload
            .get("group_path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.split('/').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        Ok(root_name)
    }
}

#[derive(Debug, Clone)]
pub struct InstRelateRecord {
    pub refno: RefnoEnum,
    pub inst_id: u64,
    pub parent_refno: Option<RefnoEnum>,
    pub world_matrix: Option<Vec<u8>>,
    /// spec 009:来源名称(RVM 组名末段 / E3D 元素名),用于身份解析与对拍报告。
    pub name: Option<String>,
    /// spec 009:PDMS 短名词(BRAN/FLAN/...);解析成功时回填。
    pub noun: Option<String>,
    /// spec 009:身份来源——`surreal_name` / `default_name_rule` / `stable_hash`。
    pub identity_source: Option<String>,
    /// spec 009:refno 是否为真实 PDMS refno(身份解析成功)。
    pub resolved: bool,
    /// RVM ATT 文件中的原始属性键值，JSON object 字符串。
    pub attrs_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstGeoRecord {
    pub hash: u64,
    pub geometry: Vec<u8>,
    pub aabb_min_x: Option<f64>,
    pub aabb_min_y: Option<f64>,
    pub aabb_min_z: Option<f64>,
    pub aabb_max_x: Option<f64>,
    pub aabb_max_y: Option<f64>,
    pub aabb_max_z: Option<f64>,
    pub meshed: bool,
}

#[derive(Debug)]
pub struct StoreStats {
    pub inst_relate_count: usize,
    pub geo_relate_count: usize,
    pub inst_geo_count: usize,
}

/// 全局单例
static GLOBAL_STORE: once_cell::sync::Lazy<ModelRelationStore> = once_cell::sync::Lazy::new(|| {
    let base_path = std::env::var("MODEL_RELATION_STORE_PATH")
        .unwrap_or_else(|_| "output/model_relations".to_string());
    ModelRelationStore::new(base_path)
});

pub fn global_store() -> &'static ModelRelationStore {
    &GLOBAL_STORE
}
