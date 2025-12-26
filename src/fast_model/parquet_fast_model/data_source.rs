//! Parquet 数据源 - 使用 DuckDB 查询 PE 和属性 parquet 文件

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Result, Context, anyhow};
use duckdb::{Connection, params};
use aios_core::RefnoEnum;

/// PE 简化数据
#[derive(Debug, Clone)]
pub struct PeDataRow {
    pub refno: RefnoEnum,
    pub noun: String,
    pub name: String,
    pub owner: RefnoEnum,
    pub dbnum: i32,
}

/// 属性值枚举
#[derive(Debug, Clone)]
pub enum AttributeValue {
    Int(i32),
    Float(f32),
    String(String),
    Bool(bool),
}

/// 属性数据
#[derive(Debug, Clone)]
pub struct AttrDataRow {
    pub refno: RefnoEnum,
    pub attributes: HashMap<String, AttributeValue>,
}

/// 获取指定 noun 类型需要的属性列表
pub fn get_required_attrs(noun: &str) -> &'static [&'static str] {
    match noun {
        "BOX" | "NBOX" => &["XLEN", "YLEN", "ZLEN"],
        "CYLI" | "SLCY" | "NCYL" => &["HEIG", "DIAM"],
        "SPHE" => &["RADI"],
        "CONE" | "NCON" | "SNOU" | "NSNO" => &["HEIG", "DTOP", "DBOT"],
        "DISH" | "NDIS" => &["HEIG", "DIAM", "RADI"],
        "CTOR" | "NCTO" => &["RINS", "ROUT", "ANGL"],
        "RTOR" | "NRTO" => &["RINS", "ROUT", "HEIG", "ANGL"],
        "PYRA" | "NPYR" => &["XBOT", "YBOT", "XTOP", "YTOP", "XOFF", "YOFF", "HEIG"],
        _ => &[], // 未知类型返回空，fallback 到 SELECT *
    }
}


/// DuckDB 数据源
pub struct DuckDbDataSource {
    conn: Connection,
    dbno: u32,
    registered_nouns: Vec<String>,
}

impl DuckDbDataSource {
    /// 创建数据源并注册 parquet 文件为视图
    pub fn new(dbno: u32, base_dir: &Path) -> Result<Self> {
        println!("📂 Initializing DuckDB data source for dbno {}...", dbno);
        
        // 创建内存数据库
        let conn = Connection::open_in_memory()
            .context("Failed to create DuckDB connection")?;
        
        let dbno_dir = base_dir.join(format!("database_models/{}", dbno));
        
        if !dbno_dir.exists() {
            return Err(anyhow!("Database directory not found: {}", dbno_dir.display()));
        }
        
        // 注册 PE parquet 视图
        let pe_path = dbno_dir.join("pe.parquet");
        if pe_path.exists() {
            let sql = format!(
                "CREATE VIEW pe AS SELECT * FROM read_parquet('{}')",
                pe_path.display()
            );
            conn.execute(&sql, [])?;
            println!("   ✅ Registered PE view: {}", pe_path.display());
        } else {
            return Err(anyhow!("PE parquet not found: {}", pe_path.display()));
        }
        
        // 注册所有 attr_{noun}.parquet 视图
        let mut registered_nouns = Vec::new();
        for entry in std::fs::read_dir(&dbno_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.starts_with("attr_") 
                    && filename.ends_with(".parquet") 
                    && !filename.chars().skip(5).any(|c| c == '_')  // 排除增量文件（attr_ 后的下划线）
                {
                    // 提取 noun：attr_EQUI.parquet -> EQUI
                    if let Some(noun) = filename
                        .strip_prefix("attr_")
                        .and_then(|s| s.strip_suffix(".parquet"))
                    {
                        let view_name = format!("attr_{}", noun);
                        let sql = format!(
                            "CREATE VIEW {} AS SELECT * FROM read_parquet('{}')",
                            view_name, path.display()
                        );
                        conn.execute(&sql, [])?;
                        println!("   ✅ Registered view: {}", view_name);
                        registered_nouns.push(noun.to_string());
                    }
                }
            }
        }
        
        println!("   📊 Total views registered: {} nouns", registered_nouns.len());
        
        Ok(Self { conn, dbno, registered_nouns })
    }
    
    /// 查询单个 PE 数据
    pub fn query_pe(&self, refno: RefnoEnum) -> Result<Option<PeDataRow>> {
        let refno_str = refno.to_string();
        
        let mut stmt = self.conn.prepare(
            "SELECT refno, noun, name, owner, dbnum FROM pe WHERE refno = ?"
        )?;
        
        let result = stmt.query_row(params![&refno_str], |row| {
            Ok(PeDataRow {
                refno,
                noun: row.get(1)?,
                name: row.get(2)?,
                owner: RefnoEnum::from(row.get::<_, String>(3)?.as_str()),
                dbnum: row.get(4)?,
            })
        });
        
        match result {
            Ok(pe) => Ok(Some(pe)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    /// 批量查询 PE 数据
    pub fn query_pe_batch(&self, refnos: &[RefnoEnum]) -> Result<Vec<PeDataRow>> {
        if refnos.is_empty() {
            return Ok(vec![]);
        }
        
        // 构建 IN 查询
        let placeholders = refnos.iter()
            .map(|r| format!("'{}'", r.to_string()))
            .collect::<Vec<_>>()
            .join(",");
        
        let sql = format!(
            "SELECT refno, noun, name, owner, dbnum FROM pe WHERE refno IN ({})",
            placeholders
        );
        
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(PeDataRow {
                refno: RefnoEnum::from(row.get::<_, String>(0)?.as_str()),
                noun: row.get(1)?,
                name: row.get(2)?,
                owner: RefnoEnum::from(row.get::<_, String>(3)?.as_str()),
                dbnum: row.get(4)?,
            })
        })?;
        
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
    
    /// 查询属性数据
    pub fn query_attr(&self, refno: RefnoEnum, noun: &str) -> Result<Option<AttrDataRow>> {
        let view_name = format!("attr_{}", noun);
        
        // 检查视图是否存在
        if !self.registered_nouns.contains(&noun.to_string()) {
            return Ok(None);
        }
        
        let refno_str = refno.to_string();
        
        // 根据 noun 类型获取所需的属性列表
        let attrs = get_required_attrs(noun);
        let sql = if attrs.is_empty() {
            // 未知类型，fallback 到 SELECT *
            format!("SELECT * FROM {} WHERE refno = ?", view_name)
        } else {
            // 只查询需要的属性
            let columns = std::iter::once("refno")
                .chain(attrs.iter().copied())
                .collect::<Vec<_>>()
                .join(", ");
            format!("SELECT {} FROM {} WHERE refno = ?", columns, view_name)
        };
        
        let mut stmt = self.conn.prepare(&sql)?;
        let column_count = stmt.column_count();
        let column_names: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).map_or(String::new(), |s| s.to_string()))
            .collect();
        
        let result = stmt.query_row(params![&refno_str], |row| {
            let mut attributes = HashMap::new();
            
            for (i, col_name) in column_names.iter().enumerate() {
                if col_name == "refno" {
                    continue;
                }
                
                // 尝试不同类型
                if let Ok(val) = row.get::<_, i32>(i) {
                    attributes.insert(col_name.clone(), AttributeValue::Int(val));
                } else if let Ok(val) = row.get::<_, f64>(i) {
                    attributes.insert(col_name.clone(), AttributeValue::Float(val as f32));
                } else if let Ok(val) = row.get::<_, String>(i) {
                    attributes.insert(col_name.clone(), AttributeValue::String(val));
                } else if let Ok(val) = row.get::<_, bool>(i) {
                    attributes.insert(col_name.clone(), AttributeValue::Bool(val));
                }
            }
            
            Ok(AttrDataRow { refno, attributes })
        });
        
        match result {
            Ok(attr) => Ok(Some(attr)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    /// 查询所有 refno（可选：按 noun 过滤）
    pub fn query_all_refnos(&self, noun_filter: Option<&str>) -> Result<Vec<RefnoEnum>> {
        let sql = if let Some(noun) = noun_filter {
            format!("SELECT refno FROM pe WHERE noun = '{}'", noun)
        } else {
            "SELECT refno FROM pe".to_string()
        };
        
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let refno_str: String = row.get(0)?;
            Ok(RefnoEnum::from(refno_str.as_str()))
        })?;
        
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
    
    /// 获取统计信息
    pub fn get_stats(&self) -> Result<()> {
        println!("\n📊 Database Statistics:");
        
        // PE 表统计
        let pe_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pe",
            [],
            |row| row.get(0)
        )?;
        println!("   PE records: {}", pe_count);
        
        // 按 noun 分组统计
        let mut noun_stmt = self.conn.prepare(
            "SELECT noun, COUNT(*) as cnt FROM pe GROUP BY noun ORDER BY cnt DESC LIMIT 10"
        )?;
        
        println!("   Top nouns:");
        let noun_rows = noun_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        
        for (noun, count) in noun_rows.flatten() {
            println!("      {}: {}", noun, count);
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_duckdb_connection() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();
        assert!(true);
    }
}
