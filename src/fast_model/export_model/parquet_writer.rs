//! Parquet Writer 模块
//!
//! 将模型实例数据写入 Parquet 格式，支持增量生成和去重。
//! 包含两个表：
//! - `instances`: 存储业务属性和几何引用（嵌套列表）
//! - `transforms`: 存储唯一的几何变换矩阵（Trans ID）

use anyhow::Result;
use chrono::Utc;
use polars::prelude::*;
use std::path::{Path, PathBuf};

use super::export_common::ExportData;
use crate::fast_model::export_model::simple_color_palette::SimpleColorPalette;

/// 变换行数据（用于 Transforms 表）
#[derive(Debug, Clone)]
pub struct TransformRow {
    pub trans_id: String,
    pub t_cols: [f32; 16],
}

/// 几何体与局部变换的绑定（用于 Instances 表）
#[derive(Debug, Clone)]
struct GeoItem {
    pub geo_hash: String,
    pub geo_trans_id: String,
}

/// 实例行数据（按 refno 聚合，用于构建 DataFrame）
#[derive(Debug, Clone)]
struct InstanceRow {
    pub refno: String,
    pub noun: String,
    pub spec_value: Option<i64>,
    pub color_index: i32,
    pub is_tubi: bool,
    pub owner_refno: Option<String>,
    pub geo_items: Vec<GeoItem>,
    pub inst_trans_id: String,          // 实例世界变换 ID (e.g. {refno}_world)
    pub aabb: [f32; 6],                 // 包围盒 [min_x, min_y, min_z, max_x, max_y, max_z]
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

    /// 获取 dbno 目录路径
    fn get_dbno_dir(&self, dbno: u32) -> PathBuf {
        self.get_base_dir().join(dbno.to_string())
    }

    /// 获取 Instances 主文件路径
    fn get_instances_main_path(&self, dbno: u32) -> PathBuf {
        self.get_dbno_dir(dbno).join("instances.parquet")
    }

    /// 获取 Transforms 主文件路径
    fn get_transforms_main_path(&self, dbno: u32) -> PathBuf {
        self.get_dbno_dir(dbno).join("transforms.parquet")
    }

    /// 获取 Instances 增量文件路径
    fn get_instances_incremental_path(&self, dbno: u32, timestamp: &str) -> PathBuf {
        self.get_dbno_dir(dbno).join(format!("instances_{}.parquet", timestamp))
    }

    /// 获取 Transforms 增量文件路径
    fn get_transforms_incremental_path(&self, dbno: u32, timestamp: &str) -> PathBuf {
        self.get_dbno_dir(dbno).join(format!("transforms_{}.parquet", timestamp))
    }

    /// 生成当前时间戳
    fn get_timestamp(&self) -> String {
        Utc::now().format("%Y%m%d_%H%M%S").to_string()
    }

    /// 列出所有指定类型的文件（Instances 或 Transforms）
    /// prefix_type: "instances" 或 "transforms"
    pub fn list_files(&self, dbno: u32, prefix_type: &str) -> Result<Vec<PathBuf>> {
        let dbno_dir = self.get_dbno_dir(dbno);
        if !dbno_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let main_file = dbno_dir.join(format!("{}.parquet", prefix_type));
        let prefix = format!("{}_", prefix_type);

        for entry in std::fs::read_dir(dbno_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            // 检查扩展名
            if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
                continue;
            }

            // 检查文件名
            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                if path == main_file {
                    files.push(path);
                    continue;
                }
                
                if filename.starts_with(&prefix) {
                    files.push(path);
                }
            }
        }

        files.sort();
        Ok(files)
    }

    /// 检查指定的 refnos 是否存在
    pub fn check_existence(&self, dbno: u32, refnos: &[String]) -> Result<Vec<String>> {
        // 只需检查 instances 表
        let files = self.list_files(dbno, "instances")?;
        if files.is_empty() {
            return Ok(Vec::new());
        }

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

        Ok(refnos.iter()
            .filter(|r| existing_set.contains(*r))
            .cloned()
            .collect())
    }

    /// 增量写入：同时生成 instances 和 transforms 文件
    pub fn write_incremental(&self, data: &ExportData, dbno: u32) -> Result<(PathBuf, PathBuf)> {
        let (flat_rows, transform_rows) = self.export_to_rows(data);
        
        if flat_rows.is_empty() {
            return Ok((PathBuf::new(), PathBuf::new()));
        }

        // 1. 构建 Instances DataFrame (聚合)
        let instances_df = self.create_instances_dataframe(flat_rows)?;
        
        // 2. 构建 Transforms DataFrame
        let transforms_df = self.create_transforms_dataframe(transform_rows)?;

        // 3. 写入文件
        let timestamp = self.get_timestamp();
        let inst_path = self.get_instances_incremental_path(dbno, &timestamp);
        let trans_path = self.get_transforms_incremental_path(dbno, &timestamp);

        // 确保目录存在
        if let Some(parent) = inst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 写入 Instances
        let file = std::fs::File::create(&inst_path)?;
        ParquetWriter::new(file).finish(&mut instances_df.clone())?;

        // 写入 Transforms
        let file = std::fs::File::create(&trans_path)?;
        ParquetWriter::new(file).finish(&mut transforms_df.clone())?;

        println!(
            "✅ 增量 Parquet 写入完成: Instances({}, {}条), Transforms({}, {}条)",
            inst_path.display(), instances_df.height(),
            trans_path.display(), transforms_df.height()
        );
        
        Ok((inst_path, trans_path))
    }

    /// 数据转换逻辑（按 refno 聚合，分离世界变换和局部变换）
    fn export_to_rows(&self, data: &ExportData) -> (Vec<InstanceRow>, Vec<TransformRow>) {
        use std::collections::HashMap;
        
        let mut instance_map: HashMap<String, InstanceRow> = HashMap::new();
        let mut transform_rows = Vec::new();
        let mut palette = SimpleColorPalette::new();
        // 用于记录已添加的世界变换，避免重复
        let mut added_world_trans: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. 处理 Components
        for comp in &data.components {
            let refno_str = comp.refno.to_string();
            let color_index = palette.index_for_noun(&comp.noun);
            
            // 生成世界变换 ID
            let world_trans_id = format!("{}_world", comp.refno);
            
            // 如果这个 refno 的世界变换还没添加，则添加
            if !added_world_trans.contains(&world_trans_id) {
                let world_t_cols = dmat4_to_f32_array(&comp.world_transform);
                transform_rows.push(TransformRow {
                    trans_id: world_trans_id.clone(),
                    t_cols: world_t_cols,
                });
                added_world_trans.insert(world_trans_id.clone());
            }
            
            // 获取或创建实例行
            let instance = instance_map.entry(refno_str.clone()).or_insert_with(|| InstanceRow {
                refno: refno_str.clone(),
                noun: comp.noun.clone(),
                spec_value: comp.spec_value,
                color_index,
                is_tubi: false,
                owner_refno: comp.owner_refno.map(|r| r.to_string()),
                geo_items: Vec::new(),
                inst_trans_id: world_trans_id.clone(),
                aabb: comp.aabb.as_ref().map(|a| [
                    a.mins().x as f32, a.mins().y as f32, a.mins().z as f32,
                    a.maxs().x as f32, a.maxs().y as f32, a.maxs().z as f32,
                ]).unwrap_or([0.0; 6]),
            });
            
            // 添加所有几何体（分离存储局部变换）
            for (geo_index, geo) in comp.geometries.iter().enumerate() {
                // 生成几何体局部变换 ID
                let geo_trans_id = format!("{}_geo_{}", comp.refno, geo_index);
                
                // 存储局部变换（不再组合）
                let geo_t_cols = dmat4_to_f32_array(&geo.local_transform);

                instance.geo_items.push(GeoItem {
                    geo_hash: geo.geo_hash.clone(),
                    geo_trans_id: geo_trans_id.clone(),
                });

                transform_rows.push(TransformRow {
                    trans_id: geo_trans_id,
                    t_cols: geo_t_cols,
                });
            }
        }

        // 2. 处理 TUBI
        let tubi_color_index = palette.index_for_noun("TUBI");
        for tubi in &data.tubings {
            let refno_str = tubi.refno.to_string();
            
            // TUBI 的世界变换 ID
            let world_trans_id = format!("{}_world", tubi.refno);
            
            // 如果这个 refno 的世界变换还没添加，则添加
            if !added_world_trans.contains(&world_trans_id) {
                let world_t_cols = dmat4_to_f32_array(&tubi.transform);
                transform_rows.push(TransformRow {
                    trans_id: world_trans_id.clone(),
                    t_cols: world_t_cols,
                });
                added_world_trans.insert(world_trans_id.clone());
            }
            
            // 获取或创建实例行
            let instance = instance_map.entry(refno_str.clone()).or_insert_with(|| InstanceRow {
                refno: refno_str.clone(),
                noun: "TUBI".to_string(),
                spec_value: tubi.spec_value,
                color_index: tubi_color_index,
                is_tubi: true,
                owner_refno: Some(tubi.owner_refno.to_string()),
                geo_items: Vec::new(),
                inst_trans_id: world_trans_id.clone(),
                aabb: tubi.aabb.as_ref().map(|a| [
                    a.mins().x as f32, a.mins().y as f32, a.mins().z as f32,
                    a.maxs().x as f32, a.maxs().y as f32, a.maxs().z as f32,
                ]).unwrap_or([0.0; 6]),
            });
            
            // TUBI 的几何体局部变换 ID（使用单位矩阵）
            let geo_trans_id = format!("{}_geo_{}", tubi.refno, tubi.index);
            
            // TUBI 局部变换为单位矩阵
            let identity_cols: [f32; 16] = [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ];

            instance.geo_items.push(GeoItem {
                geo_hash: tubi.geo_hash.clone(),
                geo_trans_id: geo_trans_id.clone(),
            });

            transform_rows.push(TransformRow {
                trans_id: geo_trans_id,
                t_cols: identity_cols,
            });
        }

        let instance_rows: Vec<InstanceRow> = instance_map.into_values().collect();
        (instance_rows, transform_rows)
    }

    fn create_instances_dataframe(&self, rows: Vec<InstanceRow>) -> Result<DataFrame> {
        if rows.is_empty() {
            return Err(anyhow::anyhow!("No instance rows to create DataFrame"));
        }
        
        // 创建基础列
        let refnos: Vec<String> = rows.iter().map(|r| r.refno.clone()).collect();
        let nouns: Vec<String> = rows.iter().map(|r| r.noun.clone()).collect();
        let specs: Vec<Option<i64>> = rows.iter().map(|r| r.spec_value).collect();
        let colors: Vec<i32> = rows.iter().map(|r| r.color_index).collect();
        let is_tubis: Vec<bool> = rows.iter().map(|r| r.is_tubi).collect();
        let owners: Vec<Option<String>> = rows.iter().map(|r| r.owner_refno.clone()).collect();
        let inst_trans_ids: Vec<String> = rows.iter().map(|r| r.inst_trans_id.clone()).collect();
        
        let min_xs: Vec<f32> = rows.iter().map(|r| r.aabb[0]).collect();
        let min_ys: Vec<f32> = rows.iter().map(|r| r.aabb[1]).collect();
        let min_zs: Vec<f32> = rows.iter().map(|r| r.aabb[2]).collect();
        let max_xs: Vec<f32> = rows.iter().map(|r| r.aabb[3]).collect();
        let max_ys: Vec<f32> = rows.iter().map(|r| r.aabb[4]).collect();
        let max_zs: Vec<f32> = rows.iter().map(|r| r.aabb[5]).collect();

        let geo_items_series = self.build_geo_items_series(&rows)?
            .cast(&Self::geo_items_dtype())?;
        
        let df = DataFrame::new(vec![
            Column::from(Series::new("refno".into(), refnos)),
            Column::from(Series::new("noun".into(), nouns)),
            Column::from(Series::new("spec_value".into(), specs)),
            Column::from(Series::new("color_index".into(), colors)),
            Column::from(Series::new("is_tubi".into(), is_tubis)),
            Column::from(Series::new("owner_refno".into(), owners)),
            Column::from(Series::new("inst_trans_id".into(), inst_trans_ids)),
            Column::from(Series::new("min_x".into(), min_xs)),
            Column::from(Series::new("min_y".into(), min_ys)),
            Column::from(Series::new("min_z".into(), min_zs)),
            Column::from(Series::new("max_x".into(), max_xs)),
            Column::from(Series::new("max_y".into(), max_ys)),
            Column::from(Series::new("max_z".into(), max_zs)),
            Column::from(geo_items_series),
        ])?;

        Ok(df)
    }

    fn create_transforms_dataframe(&self, rows: Vec<TransformRow>) -> Result<DataFrame> {
        let trans_ids: Vec<String> = rows.iter().map(|r| r.trans_id.clone()).collect();
        
        // 展平 16 个分量
        // 为了方便，这里简单循环
        let mut t_cols: Vec<Vec<f32>> = vec![Vec::new(); 16];
        for row in &rows {
            for i in 0..16 {
                t_cols[i].push(row.t_cols[i]);
            }
        }

        let mut cols: Vec<Column> = vec![Column::from(Series::new("trans_id".into(), trans_ids))];
        for i in 0..16 {
            cols.push(Column::from(Series::new((&format!("t{}", i)).into(), t_cols[i].clone())));
        }

        DataFrame::new(cols).map_err(anyhow::Error::from)
    }

    /// 双路合并
    pub fn compact(&self, dbno: u32) -> Result<Option<(PathBuf, PathBuf)>> {
        let res_instances = self.compact_table(dbno, "instances", "refno")?;
        let res_transforms = self.compact_table(dbno, "transforms", "trans_id")?;
        
        if res_instances.is_some() || res_transforms.is_some() {
             // 简单返回主文件路径，即使其中一个可能没变
             Ok(Some((
                 self.get_instances_main_path(dbno),
                 self.get_transforms_main_path(dbno)
             )))
        } else {
            Ok(None)
        }
    }

    /// 通用单表合并逻辑
    fn compact_table(&self, dbno: u32, prefix: &str, key_col: &str) -> Result<Option<PathBuf>> {
        let incremental_files = self.get_incremental_files_only(dbno, prefix)?;
        if incremental_files.is_empty() {
            return Ok(None);
        }

        println!("🔄 [{}] 开始合并 {} 个增量文件...", prefix, incremental_files.len());

        // 读取主文件
        // 修改为: output/database_models/{dbno}/{prefix}.parquet
        let main_file = self.get_dbno_dir(dbno).join(format!("{}.parquet", prefix));
        let mut frames = Vec::new();
        
        if main_file.exists() {
            let file = std::fs::File::open(&main_file)?;
            frames.push(ParquetReader::new(file).finish()?);
        }

        // 读取增量文件
        for path in &incremental_files {
            let file = std::fs::File::open(path)?;
            frames.push(ParquetReader::new(file).finish()?);
        }

        // 合并并去重
        if frames.is_empty() { return Ok(None); }
        
        // 确保所有 DataFrame 格式一致（处理旧格式/列表格式）
        let mut uniform_frames = Vec::new();
        for df in frames {
             let df = self.ensure_geo_items_format(df)?;
             uniform_frames.push(df);
        }

        let mut merged_df = uniform_frames[0].clone();
        for df in uniform_frames.iter().skip(1) {
            merged_df = merged_df.vstack(df)?;
        }

        // Polars 去重: keep='last' (保留最新)
        let subset_vec = vec![key_col.to_string()];
        let unique_df = merged_df.unique::<&[String], &String>(Some(subset_vec.as_slice()), UniqueKeepStrategy::Last, None)?;

        // 写入临时文件
        // 临时文件也放在同一目录下，避免跨设备移动
        let temp_file = self.get_dbno_dir(dbno).join(format!("{}.parquet.tmp", prefix));
        {
            let file = std::fs::File::create(&temp_file)?;
            ParquetWriter::new(file).finish(&mut unique_df.clone())?;
        }

        // 替换
        std::fs::rename(&temp_file, &main_file)?;

        // 清理
        for path in &incremental_files {
             let _ = std::fs::remove_file(path);
        }
        
        println!("✅ [{}] 合并完成: {} 条记录 -> {}", prefix, unique_df.height(), main_file.display());
        Ok(Some(main_file))
    }

    /// 确保 DataFrame 是 List 格式
    /// 如果发现旧的扁平格式（包含 geo_hash 列），则聚合为 List 格式
    fn ensure_geo_items_format(&self, df: DataFrame) -> Result<DataFrame> {
        if df.column("geo_items").is_ok() {
            return Ok(df);
        }

        let has_geo_hash = df.column("geo_hash").is_ok();
        let has_geo_hashes = df.column("geo_hashes").is_ok() && df.column("geo_trans_ids").is_ok();

        let df = if has_geo_hash && !has_geo_hashes {
            println!("⚠️ 检测到旧格式(Flat)数据，正在转换为 List<Struct>...");

            let original_height = df.height();
            let trans_col = if df.column("trans_id").is_ok() {
                "trans_id"
            } else {
                "geo_trans_id"
            };

            let group_by = df.lazy().group_by([col("refno")]);
            let agg_df = group_by
                .agg([
                    col("noun").first(),
                    col("spec_value").first(),
                    col("color_index").first(),
                    col("is_tubi").first(),
                    col("owner_refno").first(),
                    col("inst_trans_id").first(),
                    col("geo_hash").alias("geo_hashes"),
                    col(trans_col).alias("geo_trans_ids"),
                ])
                .collect()?;

            println!("   转化完成: {} -> {} 条记录", original_height, agg_df.height());
            agg_df
        } else {
            df
        };

        if df.column("geo_hashes").is_ok() && df.column("geo_trans_ids").is_ok() {
            self.convert_lists_to_geo_items(df)
        } else {
            Ok(df)
        }
    }

    fn geo_items_dtype() -> DataType {
        DataType::List(Box::new(DataType::Struct(vec![
            Field::new("geo_hash".into(), DataType::String),
            Field::new("geo_trans_id".into(), DataType::String),
        ])))
    }

    fn build_geo_items_series(&self, rows: &[InstanceRow]) -> Result<Series> {
        let mut lists: Vec<Series> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut hashes = Vec::with_capacity(row.geo_items.len());
            let mut trans_ids = Vec::with_capacity(row.geo_items.len());
            for item in &row.geo_items {
                hashes.push(item.geo_hash.clone());
                trans_ids.push(item.geo_trans_id.clone());
            }

            let hash_series = Series::new("geo_hash".into(), hashes);
            let trans_series = Series::new("geo_trans_id".into(), trans_ids);
            let struct_len = hash_series.len();
            let struct_fields = [hash_series, trans_series];
            let struct_series =
                StructChunked::from_series("geo_item".into(), struct_len, struct_fields.iter())?
                    .into_series();
            lists.push(struct_series);
        }

        Ok(Series::new("geo_items".into(), lists))
    }

    fn convert_lists_to_geo_items(&self, mut df: DataFrame) -> Result<DataFrame> {
        let geo_items = self.build_geo_items_series_from_lists(&df)?
            .cast(&Self::geo_items_dtype())?;
        df.with_column(geo_items)?;
        let _ = df.drop_in_place("geo_hashes");
        let _ = df.drop_in_place("geo_trans_ids");
        Ok(df)
    }

    fn build_geo_items_series_from_lists(&self, df: &DataFrame) -> Result<Series> {
        let geo_hashes = df.column("geo_hashes")?.list()?;
        let geo_trans_ids = df.column("geo_trans_ids")?.list()?;

        let mut lists: Vec<Series> = Vec::with_capacity(df.height());
        for idx in 0..df.height() {
            let hashes = if let Some(series) = geo_hashes.get_as_series(idx) {
                if let Ok(chunked) = series.str() {
                    chunked
                        .into_iter()
                        .map(|v| v.unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            let trans_ids = if let Some(series) = geo_trans_ids.get_as_series(idx) {
                if let Ok(chunked) = series.str() {
                    chunked
                        .into_iter()
                        .map(|v| v.unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            let mut zipped_hashes = Vec::new();
            let mut zipped_trans_ids = Vec::new();
            for (hash, trans_id) in hashes.into_iter().zip(trans_ids.into_iter()) {
                zipped_hashes.push(hash);
                zipped_trans_ids.push(trans_id);
            }

            let hash_series = Series::new("geo_hash".into(), zipped_hashes);
            let trans_series = Series::new("geo_trans_id".into(), zipped_trans_ids);
            let struct_len = hash_series.len();
            let struct_fields = [hash_series, trans_series];
            let struct_series =
                StructChunked::from_series("geo_item".into(), struct_len, struct_fields.iter())?
                    .into_series();
            lists.push(struct_series);
        }

        Ok(Series::new("geo_items".into(), lists))
    }

    fn get_incremental_files_only(&self, dbno: u32, prefix_type: &str) -> Result<Vec<PathBuf>> {
        let files = self.list_files(dbno, prefix_type)?;
        let main_file = self.get_dbno_dir(dbno).join(format!("{}.parquet", prefix_type));
        
        Ok(files.into_iter().filter(|p| *p != main_file).collect())
    }

    pub fn scan_dbnos_with_incremental(&self) -> Result<Vec<u32>> {
        // 扫描所有 dbno 子目录，检查是否有增量文件
        let base_dir = self.get_base_dir();
        if !base_dir.exists() { return Ok(Vec::new()); }
        
        let mut dbnos = std::collections::HashSet::new();
        for entry in std::fs::read_dir(base_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            // 检查是否是目录且可解析为 dbno
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    if let Ok(dbno) = dir_name.parse::<u32>() {
                        // 检查是否有增量文件
                        let has_incremental = std::fs::read_dir(&path)?
                            .filter_map(|e| e.ok())
                            .any(|e| {
                                let name = e.file_name().to_string_lossy().to_string();
                                (name.starts_with("instances_") || name.starts_with("transforms_"))
                                    && name.ends_with(".parquet")
                            });
                        
                        if has_incremental {
                            dbnos.insert(dbno);
                        }
                    }
                }
            }
        }
        Ok(dbnos.into_iter().collect())
    }
    
    // 兼容接口：根据类型返回文件名列表
    pub fn list_parquet_files(&self, dbno: u32, prefix_type: Option<&str>) -> Result<Vec<String>> {
        let type_key = prefix_type.unwrap_or("instances");
        let files = self.list_files(dbno, type_key)?;
        Ok(files.iter().filter_map(|p| p.file_name().map(|s| s.to_string_lossy().to_string())).collect())
    }
}

pub fn dmat4_to_f32_array(mat: &glam::DMat4) -> [f32; 16] {
    let cols = mat.to_cols_array();
    [
        cols[0] as f32, cols[1] as f32, cols[2] as f32, cols[3] as f32,
        cols[4] as f32, cols[5] as f32, cols[6] as f32, cols[7] as f32,
        cols[8] as f32, cols[9] as f32, cols[10] as f32, cols[11] as f32,
        cols[12] as f32, cols[13] as f32, cols[14] as f32, cols[15] as f32,
    ]
}
