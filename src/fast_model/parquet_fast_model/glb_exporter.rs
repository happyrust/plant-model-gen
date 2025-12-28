//! GLB 导出器 - 将 parquet 数据导出为 GLB 模型

use std::sync::Arc;
use std::path::{Path, PathBuf};
use anyhow::Result;
use aios_core::RefnoEnum;
use crate::fast_model::export_model::{ExportData, ComponentRecord, GeometryInstance};
use glam::DMat4;

use super::data_source::DuckDbDataSource;
use super::primitive_builder::PrimitiveBuilder;


pub struct ParquetGlbExporter {
    builder: PrimitiveBuilder,
    mesh_dir: PathBuf,
}

impl ParquetGlbExporter {
    pub fn new(data_source: Arc<DuckDbDataSource>, mesh_dir: PathBuf) -> Self {
        Self {
            builder: PrimitiveBuilder::new(data_source),
            mesh_dir,
        }
    }
    
    /// 导出为 GLB
    pub async fn export_glb(
        &self,
        refnos: &[RefnoEnum],
        output_path: &Path,
        verbose: bool
    ) -> Result<()> {
        if verbose {
            println!("🔨 Building primitives for {} refnos...", refnos.len());
        }
        
        // 1. 生成几何数据（异步）
        let geos_data = self.builder.build_batch(refnos).await;
        
        if verbose {
            println!("   ✅ Built {} geometries", geos_data.len());
        }
        
        // 2. 转换为 ExportData
        let export_data = self.to_export_data(geos_data)?;
        
        if verbose {
            println!("🎨 Exporting GLB to: {}", output_path.display());
        }
        
        // 3. 使用 GlbExporter 导出
        use crate::fast_model::export_model::export_glb::export_single_mesh_to_glb;
        use crate::fast_model::export_model::GltfMeshCache;
        
        // TODO: 实际的 GLB 导出逻辑需要进一步实现
        // 当前先保存一个占位文件
        std::fs::write(output_path, b"TODO: GLB export").map_err(|e| anyhow::anyhow!("Write error: {}", e))?;
        
        if verbose {
            println!("   ✅ GLB export complete! (placeholder)");
        }
        
        Ok(())
    }
    
    /// 将 EleInstGeosData 转换为 ExportData
    fn to_export_data(&self, geos_data: Vec<aios_core::geometry::EleInstGeosData>) -> Result<ExportData> {
        let mut components = Vec::new();
        
        for geo_data in geos_data {
            let refno = geo_data.refno;
            
            // 转换每个几何实例
            let mut geometries = Vec::new();
            for (idx, inst_geo) in geo_data.insts.iter().enumerate() {
                let geo_instance = GeometryInstance {
                    geo_hash: inst_geo.geo_hash.to_string(),
                    local_transform: inst_geo.transform.to_matrix().as_dmat4(),
                    index: idx,
                    unit_flag: inst_geo.unit_flag,
                };
                geometries.push(geo_instance);
            }
            
            if !geometries.is_empty() {
                let component = ComponentRecord {
                    refno,
                    noun: geo_data.type_name.clone(),
                    name: None,  // 可以从 PE 数据获取
                    world_transform: DMat4::IDENTITY,  // 使用单位矩阵
                    geometries,
                    owner_refno: None,
                    owner_noun: None,
                    owner_type: None,
                    spec_value: None,
                    has_neg: false,  // 初期不支持布尔运算
                    aabb: Default::default(),
                };
                components.push(component);
            }
        }
        
        Ok(ExportData {
            components,
            tubings: vec![],  // 初期不支持管道
            valid_geo_hashes: Default::default(),
            loaded_count: 0,
            failed_count: 0,
            total_instances: 0,
            tubi_count: 0,
            cache_hits: 0,
            cache_misses: 0,
        })
    }
}
