//! Parquet Writer 模块
//!
//! 将模型实例数据写入 Parquet 格式，支持增量生成和去重。

use anyhow::Result;
use chrono::Utc;
use polars::prelude::*;
use std::path::{Path, PathBuf};

use super::export_common::ExportData;

/// 实例行数据（用于 Parquet 存储）
#[derive(Debug, Clone)]
pub struct InstanceRow {
    pub refno: String,
    pub noun: String,
    pub geo_hash: String,
    pub transform: [f32; 16],
    pub aabb: Option<[f32; 6]>,
    pub is_tubi: bool,
    pub owner_refno: Option<String>,
}

/// Parquet 存储管理器
pub struct ParquetManager {
    base_dir: PathBuf,
}

impl ParquetManager {
    /// 创建新的管理器实例
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
        }
    }

    /// 获取 Parquet 文件的基础路径： output/database_models
    fn get_base_dir(&self) -> PathBuf {
        self.base_dir.join("database_models")
    }

    /// 获取主 Parquet 文件路径： output/database_models/{dbno}.parquet
    fn get_main_parquet_path(&self, dbno: u32) -> PathBuf {
        self.get_base_dir().join(format!("{}.parquet", dbno))
    }

    /// 生成增量 Parquet 文件路径： output/database_models/{dbno}_{timestamp}.parquet
    fn get_incremental_parquet_path(&self, dbno: u32) -> PathBuf {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        self.get_base_dir().join(format!("{}_{}.parquet", dbno, timestamp))
    }

    /// 列出所有 Parquet 文件（主文件 + 增量文件），按时间排序
    pub fn list_all_files(&self, dbno: u32) -> Result<Vec<PathBuf>> {
        let base_dir = self.get_base_dir();
        if !base_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let main_file = self.get_main_parquet_path(dbno);
        let prefix = format!("{}_", dbno);

        for entry in std::fs::read_dir(base_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            // 检查是否是 Parquet 文件
            if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
                continue;
            }

            // 检查文件名
            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                // 如果是主文件
                if path == main_file {
                    files.push(path);
                    continue;
                }
                
                // 如果是增量文件
                if filename.starts_with(&prefix) {
                    files.push(path);
                }
            }
        }

        // 排序：主文件通常在最前（因为没有时间戳后缀），增量文件按时间戳排序
        files.sort();
        
        Ok(files)
    }

    /// 检查指定的 refnos 是否已存在于任何 Parquet 文件中
    /// 返回已存在的 refnos 列表
    pub fn check_existence(&self, dbno: u32, refnos: &[String]) -> Result<Vec<String>> {
        let files = self.list_all_files(dbno)?;
        if files.is_empty() {
            return Ok(Vec::new());
        }

        // 收集所有已存在的 refnos
        let mut existing_set = std::collections::HashSet::new();
        
        for file_path in &files {
            if let Ok(file) = std::fs::File::open(file_path) {
                if let Ok(df) = ParquetReader::new(file).finish() {
                    if let Ok(refno_col) = df.column("refno") {
                        if let Ok(str_col) = refno_col.str() {
                            for opt_val in str_col.into_iter() {
                                if let Some(val) = opt_val {
                                    existing_set.insert(val.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // 返回输入中已存在的 refnos
        Ok(refnos.iter()
            .filter(|r| existing_set.contains(*r))
            .cloned()
            .collect())
    }

    /// 增量写入：创建新的增量文件，不读取旧数据
    pub fn write_incremental(&self, data: &ExportData, dbno: u32) -> Result<PathBuf> {
        let rows = self.export_data_to_rows(data);
        if rows.is_empty() {
            return Ok(PathBuf::new());
        }

        let df = self.rows_to_dataframe(&rows)?;
        let output_path = self.get_incremental_parquet_path(dbno);

        // 确保目录存在
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::File::create(&output_path)?;
        ParquetWriter::new(file).finish(&mut df.clone())?;

        println!(
            "✅ 增量 Parquet 写入完成: {} ({} 条记录)",
            output_path.display(),
            rows.len()
        );
        
        Ok(output_path)
    }

    /// 将 ExportData 转换为 InstanceRow 列表
    fn export_data_to_rows(&self, data: &ExportData) -> Vec<InstanceRow> {
        let mut rows = Vec::new();

        // 处理元件记录
        for comp in &data.components {
            for geo in &comp.geometries {
                let transform = dmat4_to_f32_array(&comp.world_transform);
                rows.push(InstanceRow {
                    refno: comp.refno.to_string(),
                    noun: comp.noun.clone(),
                    geo_hash: geo.geo_hash.clone(),
                    transform,
                    aabb: None,
                    is_tubi: false,
                    owner_refno: comp.owner_refno.map(|r| r.to_string()),
                });
            }
        }

        // 处理 TUBI 记录
        for tubi in &data.tubings {
            let transform = dmat4_to_f32_array(&tubi.transform);
            rows.push(InstanceRow {
                refno: tubi.refno.to_string(),
                noun: "TUBI".to_string(),
                geo_hash: tubi.geo_hash.clone(),
                transform,
                aabb: None,
                is_tubi: true,
                owner_refno: Some(tubi.owner_refno.to_string()),
            });
        }

        rows
    }

    /// 将 InstanceRow 列表转换为 DataFrame
    fn rows_to_dataframe(&self, rows: &[InstanceRow]) -> Result<DataFrame> {
        let refnos: Vec<&str> = rows.iter().map(|r| r.refno.as_str()).collect();
        let nouns: Vec<&str> = rows.iter().map(|r| r.noun.as_str()).collect();
        let geo_hashes: Vec<&str> = rows.iter().map(|r| r.geo_hash.as_str()).collect();
        let is_tubis: Vec<bool> = rows.iter().map(|r| r.is_tubi).collect();
        let owner_refnos: Vec<Option<&str>> = rows
            .iter()
            .map(|r| r.owner_refno.as_deref())
            .collect();

        // 将 transform 展平为 16 个独立的 f32 列
        let t0: Vec<f32> = rows.iter().map(|r| r.transform[0]).collect();
        let t1: Vec<f32> = rows.iter().map(|r| r.transform[1]).collect();
        let t2: Vec<f32> = rows.iter().map(|r| r.transform[2]).collect();
        let t3: Vec<f32> = rows.iter().map(|r| r.transform[3]).collect();
        let t4: Vec<f32> = rows.iter().map(|r| r.transform[4]).collect();
        let t5: Vec<f32> = rows.iter().map(|r| r.transform[5]).collect();
        let t6: Vec<f32> = rows.iter().map(|r| r.transform[6]).collect();
        let t7: Vec<f32> = rows.iter().map(|r| r.transform[7]).collect();
        let t8: Vec<f32> = rows.iter().map(|r| r.transform[8]).collect();
        let t9: Vec<f32> = rows.iter().map(|r| r.transform[9]).collect();
        let t10: Vec<f32> = rows.iter().map(|r| r.transform[10]).collect();
        let t11: Vec<f32> = rows.iter().map(|r| r.transform[11]).collect();
        let t12: Vec<f32> = rows.iter().map(|r| r.transform[12]).collect();
        let t13: Vec<f32> = rows.iter().map(|r| r.transform[13]).collect();
        let t14: Vec<f32> = rows.iter().map(|r| r.transform[14]).collect();
        let t15: Vec<f32> = rows.iter().map(|r| r.transform[15]).collect();

        let df = df! {
            "refno" => refnos,
            "noun" => nouns,
            "geo_hash" => geo_hashes,
            "is_tubi" => is_tubis,
            "owner_refno" => owner_refnos,
            "t0" => t0, "t1" => t1, "t2" => t2, "t3" => t3,
            "t4" => t4, "t5" => t5, "t6" => t6, "t7" => t7,
            "t8" => t8, "t9" => t9, "t10" => t10, "t11" => t11,
            "t12" => t12, "t13" => t13, "t14" => t14, "t15" => t15,
        }?;

        Ok(df)
    }

    /// 列出指定 dbno 的所有 Parquet 文件名称（仅文件名）
    pub fn list_parquet_files(&self, dbno: u32) -> Result<Vec<String>> {
        let files = self.list_all_files(dbno)?;
        let mut file_names = Vec::new();
        for path in files {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                file_names.push(name.to_string());
            }
        }
        Ok(file_names)
    }
}

// 辅助函数：将 DMat4 转换为 [f32; 16]
fn dmat4_to_f32_array(mat: &glam::DMat4) -> [f32; 16] {
    let cols = mat.to_cols_array();
    [
        cols[0] as f32, cols[1] as f32, cols[2] as f32, cols[3] as f32,
        cols[4] as f32, cols[5] as f32, cols[6] as f32, cols[7] as f32,
        cols[8] as f32, cols[9] as f32, cols[10] as f32, cols[11] as f32,
        cols[12] as f32, cols[13] as f32, cols[14] as f32, cols[15] as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parquet_manager_creation() {
        let manager = ParquetManager::new("test_output");
        let dbno = 12345;
        let main_path = manager.get_main_parquet_path(dbno);
        assert!(main_path.to_string_lossy().ends_with("12345.parquet"));
        
        // 测试增量路径
        let inc_path = manager.get_incremental_parquet_path(dbno);
        assert!(inc_path.to_string_lossy().contains("12345_"));
        assert!(inc_path.extension().unwrap() == "parquet");
    }
}
