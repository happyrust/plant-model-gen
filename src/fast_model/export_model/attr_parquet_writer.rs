//! 属性数据 Parquet Writer
//!
//! 将 NamedAttrMap 数据按 noun 分表写入 Parquet 格式
//! 文件组织结构：output/database_models/{dbno}/attr_{noun}.parquet
//!
//! 设计：
//! - 常见 noun（EQUI、PIPE、ELBOW 等）有预定义 schema
//! - 不常见的 noun 使用通用 schema（key-value 对）

use anyhow::{Result, anyhow};
use chrono::Utc;
use polars::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use aios_core::types::{NamedAttrMap, RefnoEnum};
use aios_core::types::named_attvalue::NamedAttrValue;
use serde_json;

/// 通用属性行（用于不常见的 noun）
#[derive(Debug, Clone)]
pub struct GenericAttrRow {
    pub refno: String,
    pub attr_key: String,
    pub attr_type: String,
    pub value_int: Option<i32>,
    pub value_str: Option<String>,
    pub value_float: Option<f32>,
    pub value_bool: Option<bool>,
    pub value_vec: Option<String>,  // JSON 字符串，用于数组和复杂类型
}

/// 属性 Parquet 管理器
pub struct AttrParquetManager {
    base_dir: PathBuf,
    /// 缓存：noun -> 最新的属性 key 列表
    noun_attrs_cache: HashMap<String, Vec<String>>,
}

impl AttrParquetManager {
    /// 创建新的管理器实例
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            noun_attrs_cache: HashMap::new(),
        }
    }

    /// 获取 dbno 的文件夹路径
    fn get_dbno_dir(&self, dbno: u32) -> PathBuf {
        self.base_dir.join("database_models").join(dbno.to_string())
    }

    /// 获取指定 noun 的属性文件路径
    fn get_attr_file_path(&self, dbno: u32, noun: &str) -> PathBuf {
        self.get_dbno_dir(dbno).join(format!("attr_{}.parquet", noun))
    }

    /// 获取指定 noun 的增量文件路径
    fn get_attr_incremental_path(&self, dbno: u32, noun: &str, timestamp: &str) -> PathBuf {
        self.get_dbno_dir(dbno).join(format!("attr_{}_{}.parquet", noun, timestamp))
    }

    /// 生成当前时间戳
    fn get_timestamp(&self) -> String {
        Utc::now().format("%Y%m%d_%H%M%S").to_string()
    }

    /// 写入增量属性 Parquet 文件（支持多个 noun）
    /// 
    /// 参数：
    /// - data: refno -> (noun, NamedAttrMap) 的映射
    /// - dbno: 数据库编号
    pub fn write_incremental(
        &self,
        data: &HashMap<RefnoEnum, (String, NamedAttrMap)>,
        dbno: u32
    ) -> Result<Vec<PathBuf>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // 按 noun 分组
        let mut grouped: HashMap<String, Vec<(RefnoEnum, &NamedAttrMap)>> = HashMap::new();
        for (refno, (noun, attmap)) in data {
            grouped.entry(noun.clone())
                .or_insert_with(Vec::new)
                .push((*refno, attmap));
        }

        let mut written_files = Vec::new();
        let timestamp = self.get_timestamp();

        // 为每个 noun 生成对应的 parquet 文件
        for (noun, entries) in grouped {
            let path = self.write_noun_incremental(&noun, &entries, dbno, &timestamp)?;
            written_files.push(path);
        }

        Ok(written_files)
    }

    /// 为单个 noun 写入增量文件
    fn write_noun_incremental(
        &self,
        noun: &str,
        entries: &[(RefnoEnum, &NamedAttrMap)],
        dbno: u32,
        timestamp: &str,
    ) -> Result<PathBuf> {
        // 收集所有属性 key
        let mut all_keys = std::collections::HashSet::new();
        for (_, attmap) in entries {
            for key in attmap.map.keys() {
                all_keys.insert(key.clone());
            }
        }

        let keys: Vec<String> = all_keys.into_iter().collect();
        let df = self.create_attr_dataframe(noun, entries, &keys)?;

        let path = self.get_attr_incremental_path(dbno, noun, timestamp);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 写入文件
        let file = std::fs::File::create(&path)?;
        ParquetWriter::new(file).finish(&mut df.clone())?;

        println!(
            "✅ 属性增量 Parquet 写入完成: {} ({} 条记录)",
            path.display(),
            df.height()
        );

        Ok(path)
    }

    /// 创建属性 DataFrame（动态列）
    fn create_attr_dataframe(
        &self,
        noun: &str,
        entries: &[(RefnoEnum, &NamedAttrMap)],
        keys: &[String],
    ) -> Result<DataFrame> {
        let row_count = entries.len();

        // refno 列
        let refnos: Vec<String> = entries.iter().map(|(r, _)| r.to_string()).collect();

        // 为每个属性 key 创建一列
        let mut columns = vec![Column::from(Series::new("refno".into(), refnos))];

        for key in keys {
            let values = self.extract_column_values(entries, key)?;
            columns.push(values);
        }

        DataFrame::new(columns).map_err(Into::into)
    }

    /// 提取指定 key 的列值
    fn extract_column_values(
        &self,
        entries: &[(RefnoEnum, &NamedAttrMap)],
        key: &str,
    ) -> Result<Column> {
        // 先检测列的数据类型（取第一个非 None 值）
        let mut inferred_type: Option<&NamedAttrValue> = None;
        for (_, attmap) in entries {
            if let Some(val) = attmap.map.get(key) {
                inferred_type = Some(val);
                break;
            }
        }

        let series = match inferred_type {
            Some(NamedAttrValue::IntegerType(_)) => {
                let vals: Vec<Option<i32>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| match v {
                            NamedAttrValue::IntegerType(i) => Some(*i),
                            _ => None,
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            Some(NamedAttrValue::StringType(_)) => {
                let vals: Vec<Option<String>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| match v {
                            NamedAttrValue::StringType(s) => Some(s.clone()),
                            _ => None,
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            Some(NamedAttrValue::F32Type(_)) => {
                let vals: Vec<Option<f32>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| match v {
                            NamedAttrValue::F32Type(f) => Some(*f),
                            _ => None,
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            Some(NamedAttrValue::BoolType(_)) => {
                let vals: Vec<Option<bool>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| match v {
                            NamedAttrValue::BoolType(b) => Some(*b),
                            _ => None,
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            // Vec3、数组等复杂类型序列化为 JSON 字符串
            Some(NamedAttrValue::Vec3Type(_)) 
            | Some(NamedAttrValue::F32VecType(_))
            | Some(NamedAttrValue::IntArrayType(_))
            | Some(NamedAttrValue::StringArrayType(_)) 
            | Some(NamedAttrValue::RefU64Array(_)) => {
                let vals: Vec<Option<String>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| {
                            serde_json::to_string(v).ok()
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            // RefU64Type 转为字符串
            Some(NamedAttrValue::RefU64Type(_)) | Some(NamedAttrValue::RefnoEnumType(_)) => {
                let vals: Vec<Option<String>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| match v {
                            NamedAttrValue::RefU64Type(r) => Some(r.to_string()),
                            NamedAttrValue::RefnoEnumType(r) => Some(r.to_string()),
                            _ => None,
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            Some(NamedAttrValue::WordType(_)) => {
                let vals: Vec<Option<String>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).and_then(|v| match v {
                            NamedAttrValue::WordType(w) => Some(w.clone()),
                            _ => None,
                        })
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
            _ => {
                // 默认当作字符串
                let vals: Vec<Option<String>> = entries
                    .iter()
                    .map(|(_, attmap)| {
                        attmap.map.get(key).map(|v| format!("{:?}", v))
                    })
                    .collect();
                Series::new(key.into(), vals)
            }
        };

        Ok(Column::from(series))
    }

    /// 合并增量文件到主文件（按 noun）
    pub fn compact(&self, dbno: u32, noun: &str) -> Result<Option<PathBuf>> {
        let dbno_dir = self.get_dbno_dir(dbno);
        if !dbno_dir.exists() {
            return Ok(None);
        }

        let incremental_files = self.list_incremental_files(dbno, noun)?;
        if incremental_files.is_empty() {
            return Ok(None);
        }

        println!("🔄 [attr_{}] 开始合并 {} 个增量文件...", noun, incremental_files.len());

        let main_file = self.get_attr_file_path(dbno, noun);
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
        let temp_file = dbno_dir.join(format!("attr_{}.parquet.tmp", noun));
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

        println!("✅ [attr_{}] 合并完成: {} 条记录", noun, unique_df.height());
        Ok(Some(main_file))
    }

    /// 列出指定 noun 的增量文件
    fn list_incremental_files(&self, dbno: u32, noun: &str) -> Result<Vec<PathBuf>> {
        let dbno_dir = self.get_dbno_dir(dbno);
        if !dbno_dir.exists() {
            return Ok(Vec::new());
        }

        let prefix = format!("attr_{}_", noun);
        let main_filename = format!("attr_{}.parquet", noun);

        let mut files = Vec::new();
        for entry in std::fs::read_dir(dbno_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
                continue;
            }

            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                if filename.starts_with(&prefix) && filename != main_filename {
                    files.push(path);
                }
            }
        }

        files.sort();
        Ok(files)
    }

    /// 列出所有 noun（用于批量合并）
    pub fn list_all_nouns(&self, dbno: u32) -> Result<Vec<String>> {
        let dbno_dir = self.get_dbno_dir(dbno);
        if !dbno_dir.exists() {
            return Ok(Vec::new());
        }

        let mut nouns = std::collections::HashSet::new();
        for entry in std::fs::read_dir(dbno_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
                continue;
            }

            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                if filename.starts_with("attr_") {
                    // attr_EQUI_20251223_013045.parquet -> EQUI
                    // attr_EQUI.parquet -> EQUI
                    let parts: Vec<&str> = filename.strip_prefix("attr_")
                        .and_then(|s| s.strip_suffix(".parquet"))
                        .map(|s| s.split('_').collect())
                        .unwrap_or_default();
                    
                    if let Some(noun) = parts.get(0) {
                        nouns.insert(noun.to_string());
                    }
                }
            }
        }

        Ok(nouns.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_attr_parquet_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manager = AttrParquetManager::new(temp_dir.path());

        // 创建测试数据
        let mut data = HashMap::new();
        let mut attmap = NamedAttrMap::default();
        attmap.map.insert("NAME".to_string(), NamedAttrValue::StringType("Test".to_string()));
        attmap.map.insert("POS".to_string(), NamedAttrValue::Vec3Type(Vec3::new(1.0, 2.0, 3.0)));
        
        data.insert(
            RefnoEnum::from("1112_123456"),
            ("EQUI".to_string(), attmap)
        );

        // 写入
        let paths = manager.write_incremental(&data, 1112).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
    }
}
