//! Parquet Stream Writer - 按 dbnum 流式写入 Parquet
//!
//! 在模型生成过程中直接将 ShapeInstancesData 按 dbnum 写入 Parquet，
//! 支持增量写入和自动合并。

use anyhow::{Context, Result};
use chrono::Utc;
use polars::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aios_core::geometry::ShapeInstancesData;
use aios_core::prim_geo::basic::TUBI_GEO_HASH;
use aios_core::RefnoEnum;

use super::simple_color_palette::SimpleColorPalette;

/// Instance 行数据（用于构建 DataFrame）
#[derive(Debug, Clone)]
struct InstanceRow {
    refno: String,
    noun: String,
    owner_refno: Option<String>,
    color_index: i32,
    spec_value: Option<i64>,
    is_tubi: bool,
    inst_trans_id: String,
    geo_items: Vec<GeoItem>,
    min_x: Option<f64>,
    min_y: Option<f64>,
    min_z: Option<f64>,
    max_x: Option<f64>,
    max_y: Option<f64>,
    max_z: Option<f64>,
}

/// Transform 行数据（用于 transforms 表）
#[derive(Debug, Clone)]
struct TransformRow {
    trans_id: String,
    t_cols: [f32; 16],
}

/// 几何体与局部变换的绑定（用于 Instances 表）
#[derive(Debug, Clone)]
struct GeoItem {
    geo_hash: String,
    geo_trans_id: String,
    /// GLB 文件相对路径 (e.g. "lod_L1/{hash}_L1.glb")
    geo_url: String,
}

/// Parquet 流式写入器
/// 
/// 在模型生成过程中直接将数据按 dbnum 写入 Parquet，使用增量文件机制实现高性能批量插入。
pub struct ParquetStreamWriter {
    base_dir: PathBuf,
    /// 记录已处理的 dbnum 集合
    processed_dbnos: Mutex<std::collections::HashSet<u32>>,
}

impl ParquetStreamWriter {
    /// 创建新的流式写入器
    /// 
    /// # Arguments
    /// * `output_dir` - Parquet 输出目录（通常是 output）
    pub fn new(output_dir: impl AsRef<Path>) -> Result<Self> {
        let base_dir = output_dir.as_ref().join("database_models");
        
        // 确保目录存在
        std::fs::create_dir_all(&base_dir)?;

        println!("📦 [Parquet] 创建流式写入器: {:?}", base_dir);

        Ok(Self {
            base_dir,
            processed_dbnos: Mutex::new(std::collections::HashSet::new()),
        })
    }

    /// 写入一批 ShapeInstancesData
    /// 
    /// 直接写入增量文件，返回：(instance_count, geo_count, transform_count)
    pub fn write_batch(&self, data: &ShapeInstancesData) -> Result<(usize, usize, usize)> {
        // 从 batch 中任意一个 refno 通过 db_meta 映射得到 dbnum（ref0 != dbnum）
        use crate::data_interface::db_meta;
        let _ = db_meta().ensure_loaded();

        let sample_refno = if let Some(geos_data) = data.inst_geos_map.values().next() {
            geos_data.refno
        } else if let Some((refno, _)) = data.inst_info_map.iter().next() {
            *refno
        } else {
            // 没有数据，跳过
            return Ok((0, 0, 0));
        };

        let dbnum = db_meta().get_dbnum_by_refno(sample_refno)
            .ok_or_else(|| anyhow::anyhow!(
                "[ParquetStreamWriter] 缺少 ref0->dbnum 映射: refno={}",
                sample_refno
            ))?;
        
        // 记录已处理的 dbnum
        {
            let mut processed = self.processed_dbnos.lock().unwrap();
            processed.insert(dbnum);
        }
        
        // 提取行数据
        let (instance_rows, transform_rows) = self.extract_rows(data)?;
        
        if instance_rows.is_empty() {
            return Ok((0, 0, 0));
        }
        
        // 创建 dbnum 目录
        let dbno_dir = self.base_dir.join(dbnum.to_string());
        std::fs::create_dir_all(&dbno_dir)?;
        
        // 生成增量文件
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%6f").to_string();
        let inst_incr_path = dbno_dir.join(format!("instance_{}.parquet", timestamp));
        let trans_incr_path = dbno_dir.join(format!("transform_{}.parquet", timestamp));
        
        // 创建 DataFrame 并写入
        let instances_df = self.create_instances_dataframe(instance_rows)?;
        let transforms_df = self.create_transforms_dataframe(transform_rows)?;
        
        let instance_count = instances_df.height();
        let transform_count = transforms_df.height();
        
        {
            let file = std::fs::File::create(&inst_incr_path)?;
            ParquetWriter::new(file).finish(&mut instances_df.clone())?;
        }
        {
            let file = std::fs::File::create(&trans_incr_path)?;
            ParquetWriter::new(file).finish(&mut transforms_df.clone())?;
        }
        
        // 统计
        let geo_count = data.inst_geos_map.len();
        
        Ok((instance_count, geo_count, transform_count))
    }

    /// 完成写入并合并所有增量文件
    pub fn finalize(&self) -> Result<()> {
        let processed = self.processed_dbnos.lock().unwrap();
        
        if processed.is_empty() {
            println!("📦 [Parquet] 无数据需要合并");
            return Ok(());
        }
        
        println!("🔍 [Parquet] 开始合并 {} 个 dbnum 的数据...", processed.len());
        
        for dbnum in processed.iter() {
            self.compact_dbno(*dbnum)?;
        }
        
        println!("✅ [Parquet] 全部完成");
        Ok(())
    }

    /// 从单个 ShapeInstancesData 中提取行数据
    fn extract_rows(&self, data: &ShapeInstancesData) 
        -> Result<(Vec<InstanceRow>, Vec<TransformRow>)> {
        let mut instance_rows = Vec::new();
        let mut transform_rows = Vec::new();
        let mut palette = SimpleColorPalette::new();
        let mut added_trans_ids: HashSet<String> = HashSet::new();
        
        // 遍历 inst_info_map 获取实例信息
        for (refno, info) in &data.inst_info_map {
            let inst_key = info.get_inst_key();
            let noun_str = if info.owner_type.is_empty() {
                "UNKOWN"
            } else {
                info.owner_type.as_str()
            };
            let color_index = palette.index_for_noun(noun_str);
            let refno_str = refno.to_string();
            let world_trans_id = format!("{}_world", refno_str);

            if added_trans_ids.insert(world_trans_id.clone()) {
                let world_matrix = info.get_ele_world_transform().to_matrix();
                let t_cols = dmat4_to_f32_array(&world_matrix.as_dmat4());
                transform_rows.push(TransformRow { trans_id: world_trans_id.clone(), t_cols });
            }
            
            // 从 inst_geos_map 获取该 instance 的所有 geo
            let mut geo_items = Vec::new();
            
            if let Some(geos_data) = data.inst_geos_map.get(&inst_key) {
                for (idx, geo) in geos_data.insts.iter().enumerate() {
                    let trans_id = format!("{}_geo_{}", refno_str, idx);
                    let geo_hash_str = geo.geo_hash.to_string();
                    geo_items.push(GeoItem {
                        geo_url: format!("lod_L1/{}_L1.glb", &geo_hash_str),
                        geo_hash: geo_hash_str,
                        geo_trans_id: trans_id.clone(),
                    });
                    
                    // 生成 transform row
                    let transform_matrix = geo.geo_transform.to_matrix();
                    let t_cols = dmat4_to_f32_array(&transform_matrix.as_dmat4());
                    transform_rows.push(TransformRow { trans_id, t_cols });
                }
            }
            
            // 提取 AABB
            let (min_x, min_y, min_z, max_x, max_y, max_z) = if let Some(aabb) = &info.aabb {
                (
                    Some(aabb.mins.x as f64),
                    Some(aabb.mins.y as f64),
                    Some(aabb.mins.z as f64),
                    Some(aabb.maxs.x as f64),
                    Some(aabb.maxs.y as f64),
                    Some(aabb.maxs.z as f64),
                )
            } else {
                (None, None, None, None, None, None)
            };
            
            instance_rows.push(InstanceRow {
                refno: refno_str.clone(),
                noun: noun_str.to_string(),
                owner_refno: if info.owner_refno != *refno {
                    Some(info.owner_refno.to_string())
                } else {
                    None
                },
                color_index,
                spec_value: None, // ShapeInstancesData 中没有 spec_value
                is_tubi: false,
                inst_trans_id: world_trans_id,
                geo_items,
                min_x,
                min_y,
                min_z,
                max_x,
                max_y,
                max_z,
            });
        }
        
        // 处理 TUBI
        let tubi_color_index = palette.index_for_noun("TUBI");
        for (refno, tubi_info) in &data.inst_tubi_map {
            let refno_str = refno.to_string();
            let world_trans_id = format!("{}_world", refno_str);
            if added_trans_ids.insert(world_trans_id.clone()) {
                let world_matrix = tubi_info.get_ele_world_transform().to_matrix();
                let t_cols = dmat4_to_f32_array(&world_matrix.as_dmat4());
                transform_rows.push(TransformRow { trans_id: world_trans_id.clone(), t_cols });
            }

            let mut geo_items = Vec::new();
            let inst_key = tubi_info.get_inst_key();
            if let Some(geos_data) = data.inst_geos_map.get(&inst_key) {
                for (idx, geo) in geos_data.insts.iter().enumerate() {
                    let trans_id = format!("{}_geo_{}", refno_str, idx);
                    let geo_hash_str = geo.geo_hash.to_string();
                    geo_items.push(GeoItem {
                        geo_url: format!("lod_L1/{}_L1.glb", &geo_hash_str),
                        geo_hash: geo_hash_str,
                        geo_trans_id: trans_id.clone(),
                    });

                    let transform_matrix = geo.geo_transform.to_matrix();
                    let t_cols = dmat4_to_f32_array(&transform_matrix.as_dmat4());
                    transform_rows.push(TransformRow { trans_id, t_cols });
                }
            } else {
                let trans_id = format!("{}_geo_0", refno_str);
                let fallback_geo_hash = tubi_info
                    .cata_hash
                    .clone()
                    .unwrap_or_else(|| TUBI_GEO_HASH.to_string());
                geo_items.push(GeoItem {
                    geo_url: format!("lod_L1/{}_L1.glb", &fallback_geo_hash),
                    geo_hash: fallback_geo_hash,
                    geo_trans_id: trans_id.clone(),
                });
                let identity_cols: [f32; 16] = [
                    1.0, 0.0, 0.0, 0.0,
                    0.0, 1.0, 0.0, 0.0,
                    0.0, 0.0, 1.0, 0.0,
                    0.0, 0.0, 0.0, 1.0,
                ];
                transform_rows.push(TransformRow { trans_id, t_cols: identity_cols });
            }

            let (min_x, min_y, min_z, max_x, max_y, max_z) = if let Some(aabb) = &tubi_info.aabb {
                (
                    Some(aabb.mins.x as f64),
                    Some(aabb.mins.y as f64),
                    Some(aabb.mins.z as f64),
                    Some(aabb.maxs.x as f64),
                    Some(aabb.maxs.y as f64),
                    Some(aabb.maxs.z as f64),
                )
            } else {
                (None, None, None, None, None, None)
            };

            instance_rows.push(InstanceRow {
                refno: refno_str,
                noun: "TUBI".to_string(),
                owner_refno: if tubi_info.owner_refno != *refno {
                    Some(tubi_info.owner_refno.to_string())
                } else {
                    None
                },
                color_index: tubi_color_index,
                spec_value: None,
                is_tubi: true,
                inst_trans_id: world_trans_id,
                geo_items,
                min_x,
                min_y,
                min_z,
                max_x,
                max_y,
                max_z,
            });
        }
        
        Ok((instance_rows, transform_rows))
    }

    /// 创建 instances DataFrame
    fn create_instances_dataframe(&self, rows: Vec<InstanceRow>) -> Result<DataFrame> {
        if rows.is_empty() {
            return Err(anyhow::anyhow!("No instance rows to create DataFrame"));
        }
        
        let refnos: Vec<String> = rows.iter().map(|r| r.refno.clone()).collect();
        let nouns: Vec<String> = rows.iter().map(|r| r.noun.clone()).collect();
        let owners: Vec<Option<String>> = rows.iter().map(|r| r.owner_refno.clone()).collect();
        let colors: Vec<i32> = rows.iter().map(|r| r.color_index).collect();
        let specs: Vec<Option<i64>> = rows.iter().map(|r| r.spec_value).collect();
        let is_tubis: Vec<bool> = rows.iter().map(|r| r.is_tubi).collect();
        let inst_trans_ids: Vec<String> = rows.iter().map(|r| r.inst_trans_id.clone()).collect();
        
        // AABB 列
        let min_xs: Vec<Option<f64>> = rows.iter().map(|r| r.min_x).collect();
        let min_ys: Vec<Option<f64>> = rows.iter().map(|r| r.min_y).collect();
        let min_zs: Vec<Option<f64>> = rows.iter().map(|r| r.min_z).collect();
        let max_xs: Vec<Option<f64>> = rows.iter().map(|r| r.max_x).collect();
        let max_ys: Vec<Option<f64>> = rows.iter().map(|r| r.max_y).collect();
        let max_zs: Vec<Option<f64>> = rows.iter().map(|r| r.max_z).collect();
        
        let geo_items_series = self.build_geo_items_series(&rows)?
            .cast(&Self::geo_items_dtype())?;
        
        let df = DataFrame::new(vec![
            Column::from(Series::new("refno".into(), refnos)),
            Column::from(Series::new("noun".into(), nouns)),
            Column::from(Series::new("owner_refno".into(), owners)),
            Column::from(Series::new("color_index".into(), colors)),
            Column::from(Series::new("spec_value".into(), specs)),
            Column::from(Series::new("is_tubi".into(), is_tubis)),
            Column::from(Series::new("inst_trans_id".into(), inst_trans_ids)),
            Column::from(geo_items_series),
            Column::from(Series::new("min_x".into(), min_xs)),
            Column::from(Series::new("min_y".into(), min_ys)),
            Column::from(Series::new("min_z".into(), min_zs)),
            Column::from(Series::new("max_x".into(), max_xs)),
            Column::from(Series::new("max_y".into(), max_ys)),
            Column::from(Series::new("max_z".into(), max_zs)),
        ])?;
        
        Ok(df)
    }

    fn geo_items_dtype() -> DataType {
        DataType::List(Box::new(DataType::Struct(vec![
            Field::new("geo_hash".into(), DataType::String),
            Field::new("geo_trans_id".into(), DataType::String),
            Field::new("geo_url".into(), DataType::String),
        ])))
    }

    fn build_geo_items_series(&self, rows: &[InstanceRow]) -> Result<Series> {
        let mut lists: Vec<Series> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut hashes = Vec::with_capacity(row.geo_items.len());
            let mut trans_ids = Vec::with_capacity(row.geo_items.len());
            let mut urls = Vec::with_capacity(row.geo_items.len());
            for item in &row.geo_items {
                hashes.push(item.geo_hash.clone());
                trans_ids.push(item.geo_trans_id.clone());
                urls.push(item.geo_url.clone());
            }

            let hash_series = Series::new("geo_hash".into(), hashes);
            let trans_series = Series::new("geo_trans_id".into(), trans_ids);
            let url_series = Series::new("geo_url".into(), urls);
            let struct_len = hash_series.len();
            let struct_fields = [hash_series, trans_series, url_series];
            let struct_series =
                StructChunked::from_series("geo_item".into(), struct_len, struct_fields.iter())?
                    .into_series();
            lists.push(struct_series);
        }

        Ok(Series::new("geo_items".into(), lists))
    }

    fn ensure_geo_items_format(&self, df: DataFrame) -> Result<DataFrame> {
        if df.column("geo_items").is_ok() {
            return Ok(df);
        }

        if df.column("geo_hashes").is_ok() && df.column("geo_trans_ids").is_ok() {
            return self.convert_lists_to_geo_items(df);
        }

        Ok(df)
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

    /// 创建 transforms DataFrame
    fn create_transforms_dataframe(&self, rows: Vec<TransformRow>) -> Result<DataFrame> {
        if rows.is_empty() {
            return Err(anyhow::anyhow!("No transform rows to create DataFrame"));
        }
        
        let trans_ids: Vec<String> = rows.iter().map(|r| r.trans_id.clone()).collect();
        
        // 展平 16 个分量
        let mut t_cols: Vec<Vec<f32>> = vec![Vec::new(); 16];
        for row in &rows {
            for i in 0..16 {
                t_cols[i].push(row.t_cols[i]);
            }
        }
        
        let mut cols: Vec<Column> = vec![Column::from(Series::new("trans_id".into(), trans_ids))];
        for i in 0..16 {
            cols.push(Column::from(Series::new(format!("t{}", i).into(), t_cols[i].clone())));
        }
        
        DataFrame::new(cols).map_err(Into::into)
    }

    /// 合并指定 dbnum 的增量文件到主文件
    fn compact_dbno(&self, dbnum: u32) -> Result<()> {
        self.compact_table(dbnum, "instance", "refno")?;
        self.compact_table(dbnum, "transform", "trans_id")?;
        Ok(())
    }

    /// 通用单表合并逻辑
    fn compact_table(&self, dbnum: u32, prefix: &str, key_col: &str) -> Result<()> {
        let dbno_dir = self.base_dir.join(dbnum.to_string());
        let main_file = dbno_dir.join(format!("{}.parquet", prefix));
        
        // 列出所有增量文件
        let incremental_files = self.list_incremental_files(dbnum, prefix)?;
        if incremental_files.is_empty() {
            return Ok(());
        }
        
        println!("🔄 [Parquet] dbnum={} {} 合并 {} 个增量文件...", dbnum, prefix, incremental_files.len());
        
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
            return Ok(());
        }
        
        // 合并（先统一 schema）
        let mut uniform_frames = Vec::new();
        for df in frames {
            uniform_frames.push(self.ensure_geo_items_format(df)?);
        }

        let mut merged_df = uniform_frames[0].clone();
        for df in uniform_frames.iter().skip(1) {
            merged_df = merged_df.vstack(df)?;
        }
        
        // 去重（保留最新）
        let unique_df = merged_df.unique::<&[String], &String>(
            Some(&[key_col.to_string()]),
            UniqueKeepStrategy::Last,
            None
        )?;
        
        // 写入临时文件
        let temp_file = dbno_dir.join(format!("{}.parquet.tmp", prefix));
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
        
        println!("✅ [Parquet] dbnum={} {} 合并完成: {} 条记录", dbnum, prefix, unique_df.height());
        Ok(())
    }

    /// 列出指定 dbnum 和类型的增量文件
    fn list_incremental_files(&self, dbnum: u32, prefix: &str) -> Result<Vec<PathBuf>> {
        let dbno_dir = self.base_dir.join(dbnum.to_string());
        if !dbno_dir.exists() {
            return Ok(Vec::new());
        }
        
        let pattern_prefix = format!("{}_{}", prefix, "");
        let main_filename = format!("{}.parquet", prefix);
        
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dbno_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
                continue;
            }
            
            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                if filename.starts_with(&pattern_prefix) && filename != main_filename {
                    files.push(path);
                }
            }
        }
        
        files.sort();
        Ok(files)
    }

    /// 获取输出路径
    pub fn output_path(&self) -> &Path {
        &self.base_dir
    }
}

/// 将 DMat4 转换为 f32 数组
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
    fn test_stream_writer_creation() {
        let temp_dir = std::env::temp_dir().join("parquet_stream_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        
        let writer = ParquetStreamWriter::new(&temp_dir).unwrap();
        assert!(writer.output_path().exists());
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
