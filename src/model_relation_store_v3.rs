/// 极简版：直接存储 refno 关联的所有 ID 集合
use anyhow::{Context, Result};
use aios_core::RefnoEnum;
use dashmap::DashMap;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// refno 关联的所有数据（扁平化存储）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RefnoRelations {
    pub refno: u64,
    pub inst_ids: Vec<u64>,
    pub geo_hashes: Vec<u64>,
    pub tubi_segments: Vec<Vec<u8>>,
    pub bool_results: Vec<Vec<u8>>,
    pub world_matrices: Vec<Vec<u8>>,
}

pub struct ModelRelationStoreV3 {
    base_path: PathBuf,
    connections: DashMap<u32, Arc<Connection>>,
}

impl ModelRelationStoreV3 {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            connections: DashMap::new(),
        }
    }

    fn get_conn(&self, dbnum: u32) -> Result<Arc<Connection>> {
        if let Some(conn) = self.connections.get(&dbnum) {
            return Ok(conn.clone());
        }

        let db_path = self.base_path.join(format!("{}/relations.db", dbnum));
        std::fs::create_dir_all(db_path.parent().unwrap())?;

        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;"
        )?;

        self.init_schema(&conn)?;

        let conn = Arc::new(conn);
        self.connections.insert(dbnum, conn.clone());
        Ok(conn)
    }

    fn init_schema(&self, conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS refno_relations (
                refno INTEGER PRIMARY KEY,
                data BLOB NOT NULL,
                updated_at INTEGER DEFAULT (unixepoch())
            )",
            [],
        )?;
        Ok(())
    }

    /// 核心简化：单条 DELETE 清理所有关联数据
    pub fn cleanup_by_refnos(&self, dbnum: u32, refnos: &[RefnoEnum]) -> Result<usize> {
        if refnos.is_empty() {
            return Ok(0);
        }

        let conn = self.get_conn(dbnum)?;
        let refno_list = refnos.iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");

        let deleted = conn.execute(
            &format!("DELETE FROM refno_relations WHERE refno IN ({})", refno_list),
            [],
        )?;

        Ok(deleted)
    }

    /// 批量保存（使用 bincode 序列化）
    pub fn save_relations(&self, dbnum: u32, relations: &[RefnoRelations]) -> Result<()> {
        if relations.is_empty() {
            return Ok(());
        }

        let conn = self.get_conn(dbnum)?;
        let tx = conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO refno_relations (refno, data) VALUES (?1, ?2)"
            )?;

            for rel in relations {
                let data = bincode::serialize(rel)?;
                stmt.execute(params![rel.refno, data])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// 批量读取
    pub fn load_relations(&self, dbnum: u32, refnos: &[RefnoEnum]) -> Result<Vec<RefnoRelations>> {
        if refnos.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.get_conn(dbnum)?;
        let refno_list = refnos.iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");

        let mut stmt = conn.prepare(&format!(
            "SELECT data FROM refno_relations WHERE refno IN ({})",
            refno_list
        ))?;

        let results = stmt.query_map([], |row| {
            let data: Vec<u8> = row.get(0)?;
            Ok(data)
        })?
        .filter_map(|r| r.ok())
        .filter_map(|data| bincode::deserialize(&data).ok())
        .collect();

        Ok(results)
    }

    pub fn get_stats(&self, dbnum: u32) -> Result<usize> {
        let conn = self.get_conn(dbnum)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM refno_relations", [], |row| row.get(0)
        )?;
        Ok(count as usize)
    }
}

static GLOBAL_STORE_V3: once_cell::sync::Lazy<ModelRelationStoreV3> =
    once_cell::sync::Lazy::new(|| {
        let base_path = std::env::var("MODEL_RELATION_STORE_PATH")
            .unwrap_or_else(|_| "output/model_relations_v3".to_string());
        ModelRelationStoreV3::new(base_path)
    });

pub fn global_store_v3() -> &'static ModelRelationStoreV3 {
    &GLOBAL_STORE_V3
}
