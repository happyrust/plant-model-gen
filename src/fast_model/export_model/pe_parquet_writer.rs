//! PE 数据 Parquet Writer
//!
//! 将 SPdmsElement 数据写入 Parquet 格式
//! 文件组织结构：output/database_models/{dbnum}/pe.parquet

use anyhow::Result;
use chrono::Utc;
use polars::prelude::*;
use std::path::{Path, PathBuf};
use aios_core::types::SPdmsElement;
use crate::fast_model::export_model::parquet_writer::dmat4_to_f32_array;

/// PE 行数据
#[derive(Debug, Clone)]
pub struct PeRow {
    pub refno: String,
    pub owner: String,
    pub name: String,
    pub noun: String,
    pub dbnum: i32,
    pub sesno: i32,
    pub status_code: String,
    pub cata_hash: String,
    pub lock: bool,
    pub deleted: bool,
    pub typex: Option<i32>,
}

impl PeRow {
    /// 从 SPdmsElement 创建 PeRow
    pub fn from_pe(pe: &SPdmsElement) -> Self {
        Self {
            refno: pe.refno.to_string(),
            owner: pe.owner.to_string(),
            name: pe.name.clone(),
            noun: pe.noun.clone(),
            dbnum: pe.dbnum,
            sesno: pe.sesno,
            status_code: pe.status_code.clone(),
            cata_hash: pe.cata_hash.clone(),
            lock: pe.lock,
            deleted: pe.deleted,
            typex: pe.typex,
        }
    }
}

/// PE Parquet 管理器
pub struct PeParquetManager {
    base_dir: PathBuf,
}

impl PeParquetManager {
    /// 创建新的管理器实例
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
        }
    }

    /// 获取 dbnum 的文件夹路径
    fn get_dbno_dir(&self, dbnum: u32) -> PathBuf {
        self.base_dir.join("database_models").join(dbnum.to_string())
    }

    /// 获取 PE 主文件路径
    fn get_pe_main_path(&self, dbnum: u32) -> PathBuf {
        self.get_dbno_dir(dbnum).join("pe.parquet")
    }

    /// 获取 PE 增量文件路径
    fn get_pe_incremental_path(&self, dbnum: u32, timestamp: &str) -> PathBuf {
        self.get_dbno_dir(dbnum).join(format!("pe_{}.parquet", timestamp))
    }

    /// 生成当前时间戳
    fn get_timestamp(&self) -> String {
        Utc::now().format("%Y%m%d_%H%M%S").to_string()
    }

    /// 写入增量 PE Parquet 文件
    pub fn write_incremental(&self, pes: &[SPdmsElement], dbnum: u32) -> Result<PathBuf> {
        if pes.is_empty() {
            return Ok(PathBuf::new());
        }

        let rows: Vec<PeRow> = pes.iter().map(PeRow::from_pe).collect();
        let df = self.create_pe_dataframe(rows)?;

        let timestamp = self.get_timestamp();
        let path = self.get_pe_incremental_path(dbnum, &timestamp);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 写入文件
        let file = std::fs::File::create(&path)?;
        ParquetWriter::new(file).finish(&mut df.clone())?;

        println!("✅ PE 增量 Parquet 写入完成: {} ({} 条记录)", path.display(), df.height());
        
        Ok(path)
    }

    /// 创建 PE DataFrame
    fn create_pe_dataframe(&self, rows: Vec<PeRow>) -> Result<DataFrame> {
        let refnos: Vec<String> = rows.iter().map(|r| r.refno.clone()).collect();
        let owners: Vec<String> = rows.iter().map(|r| r.owner.clone()).collect();
        let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
        let nouns: Vec<String> = rows.iter().map(|r| r.noun.clone()).collect();
        let dbnums: Vec<i32> = rows.iter().map(|r| r.dbnum).collect();
        let sesnos: Vec<i32> = rows.iter().map(|r| r.sesno).collect();
        let status_codes: Vec<String> = rows.iter().map(|r| r.status_code.clone()).collect();
        let cata_hashes: Vec<String> = rows.iter().map(|r| r.cata_hash.clone()).collect();
        let locks: Vec<bool> = rows.iter().map(|r| r.lock).collect();
        let deleteds: Vec<bool> = rows.iter().map(|r| r.deleted).collect();
        let typexs: Vec<Option<i32>> = rows.iter().map(|r| r.typex).collect();

        let df = df! {
            "refno" => refnos,
            "owner" => owners,
            "name" => names,
            "noun" => nouns,
            "dbnum" => dbnums,
            "sesno" => sesnos,
            "status_code" => status_codes,
            "cata_hash" => cata_hashes,
            "lock" => locks,
            "deleted" => deleteds,
            "typex" => typexs,
        }?;

        Ok(df)
    }

    /// 合并增量文件到主文件
    pub fn compact(&self, dbnum: u32) -> Result<Option<PathBuf>> {
        let dbno_dir = self.get_dbno_dir(dbnum);
        if !dbno_dir.exists() {
            return Ok(None);
        }

        // 列出所有增量文件
        let incremental_files = self.list_incremental_files(dbnum)?;
        if incremental_files.is_empty() {
            return Ok(None);
        }

        println!("🔄 [PE] 开始合并 {} 个增量文件...", incremental_files.len());

        let main_file = self.get_pe_main_path(dbnum);
        let mut frames = Vec::new();

        // 读取主文件（如果存在）
        if main_file.exists() {
            let file = std::fs::File::open(&main_file)?;
            frames.push(ParquetReader::new(file).finish()?);
        }

        // 读取增量文件
        for path in &incremental_files {
            let file = std::fs::File::open(path)?;
            frames.push(ParquetReader::new(file).finish()?);
        }

        if frames.is_empty() {
            return Ok(None);
        }

        // 合并并去重
        let mut merged_df = frames[0].clone();
        for df in frames.iter().skip(1) {
            merged_df = merged_df.vstack(df)?;
        }

        // 按 refno 去重，保留最新记录
        let unique_df = merged_df.unique::<&[String], &String>(
            Some(&["refno".to_string()]),
            UniqueKeepStrategy::Last,
            None
        )?;

        // 写入临时文件
        let temp_file = dbno_dir.join("pe.parquet.tmp");
        {
            let file = std::fs::File::create(&temp_file)?;
            ParquetWriter::new(file).finish(&mut unique_df.clone())?;
        }

        // 原子替换
        std::fs::rename(&temp_file, &main_file)?;

        // 清理增量文件
        for path in &incremental_files {
            let _ = std::fs::remove_file(path);
        }

        println!("✅ [PE] 合并完成: {} 条记录", unique_df.height());
        Ok(Some(main_file))
    }

    /// 列出增量文件（不包括主文件）
    fn list_incremental_files(&self, dbnum: u32) -> Result<Vec<PathBuf>> {
        let dbno_dir = self.get_dbno_dir(dbnum);
        if !dbno_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        for entry in std::fs::read_dir(dbno_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
                continue;
            }

            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                // pe_{timestamp}.parquet 格式
                if filename.starts_with("pe_") && filename != "pe.parquet" {
                    files.push(path);
                }
            }
        }

        files.sort();
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_core::RefnoEnum;

    #[test]
    fn test_pe_parquet_write_and_read() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manager = PeParquetManager::new(temp_dir.path());

        // 创建测试数据
        let pes = vec![
            SPdmsElement {
                refno: RefnoEnum::from("1112_123456"),
                owner: RefnoEnum::from("1112_100"),
                name: "Test PE 1".to_string(),
                noun: "EQUI".to_string(),
                dbnum: 1112,
                sesno: 1,
                status_code: "OK".to_string(),
                cata_hash: "abc123".to_string(),
                lock: false,
                deleted: false,
                typex: Some(10),
                ..Default::default()
            },
        ];

        // 写入
        let path = manager.write_incremental(&pes, 1112).unwrap();
        assert!(path.exists());

        // 读取验证
        let file = std::fs::File::open(&path).unwrap();
        let df = ParquetReader::new(file).finish().unwrap();
        assert_eq!(df.height(), 1);
        
        let refno_col = df.column("refno").unwrap();
        assert_eq!(refno_col.str().unwrap().get(0).unwrap(), "1112_123456");
    }
}
