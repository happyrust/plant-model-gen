//! 基本体几何构建器 - 从 parquet 数据构建基本体几何

use std::sync::Arc;
use std::collections::BTreeMap;
use anyhow::{Result, anyhow};
use aios_core::RefnoEnum;
use aios_core::types::{NamedAttrMap, NamedAttrValue};
use aios_core::geometry::{EleInstGeo, EleInstGeosData, GeoBasicType};
use aios_core::shape::pdms_shape::BrepShapeTrait;
use glam::Vec3;

use super::data_source::{DuckDbDataSource, AttributeValue};


pub struct PrimitiveBuilder {
    data_source: Arc<DuckDbDataSource>,
}

impl PrimitiveBuilder {
    pub fn new(data_source: Arc<DuckDbDataSource>) -> Self {
        Self { data_source }
    }
    
    /// 生成单个基本体的几何数据
    pub async fn build_primitive(&self, refno: RefnoEnum) -> Result<Option<EleInstGeosData>> {
        // 1. 获取 PE 数据
        let pe = self.data_source.query_pe(refno)?
            .ok_or_else(|| anyhow!("PE not found: {}", refno))?;
        
        // 2. 获取 world_transform（使用 aios_core 提供的方法）
        let world_transform = match aios_core::get_world_transform(refno).await {
            Ok(Some(trans)) => trans,
            Ok(None) => {
                // 没有 world_transform，跳过
                return Ok(None);
            }
            Err(e) => {
                eprintln!("⚠️  Failed to get world_transform for {}: {}", refno, e);
                return Ok(None);
            }
        };
        
        // 3. 获取属性数据
        let attr_map = if let Some(attr) = self.data_source.query_attr(refno, &pe.noun)? {
            self.build_named_attmap(&pe.noun, &attr)?
        } else {
            // 如果没有属性数据，创建空的 NamedAttrMap
            NamedAttrMap::default()
        };
        
        // 4. 创建 CSG shape
        let csg_shape = match attr_map.create_csg_shape(None) {
            Some(shape) => shape,
            None => {
                // 跳过无法创建 shape 的元素
                return Ok(None);
            }
        };
        
        // 5. 验证 shape
        if !csg_shape.check_valid() {
            return Ok(None);
        }
        
        // 6. 生成几何实例
        let transform = csg_shape.get_trans();
        
        // 检查 NaN
        if transform.translation.is_nan() 
            || transform.rotation.is_nan() 
            || transform.scale.is_nan() 
        {
            return Ok(None);
        }
        
        let geo_param = csg_shape.convert_to_geo_param()
            .unwrap_or(aios_core::parsed_data::geo_params_data::PdmsGeoParam::Unknown);
        let geo_hash = csg_shape.hash_unit_mesh_params();
        
        let unit_flag = match &geo_param {
            aios_core::parsed_data::geo_params_data::PdmsGeoParam::PrimSCylinder(s) => s.unit_flag,
            // PrimLoft(SweepSolid) 仅在“单段直线且无倾斜”时可安全 unit 化复用
            aios_core::parsed_data::geo_params_data::PdmsGeoParam::PrimLoft(s) => s.is_reuse_unit(),
            _ => false,
        };
        
        let inst_geo = EleInstGeo {
            geo_hash,
            refno,
            pts: Default::default(),
            aabb: None,
            transform,
            geo_param,
            visible: true,  // 默认可见
            is_tubi: false,
            geo_type: if attr_map.is_neg() {
                GeoBasicType::Neg
            } else {
                GeoBasicType::Pos
            },
            cata_neg_refnos: vec![],
            unit_flag,
        };
        
        // 7. 构建 EleInstGeosData
        let inst_key = format!("{}_{}", refno, 0);  // 简化的 inst_key
        Ok(Some(EleInstGeosData {
            inst_key,
            refno,
            insts: vec![inst_geo],
            aabb: None,
            type_name: pe.noun.clone(),
        }))
    }
    
    /// 从 AttrDataRow 构建 NamedAttrMap
    fn build_named_attmap(
        &self,
        noun: &str,
        attr: &super::data_source::AttrDataRow
    ) -> Result<NamedAttrMap> {
        let mut map = BTreeMap::new();
        
        // 添加 TYPE 属性（必需）
        map.insert("TYPE".to_string(), NamedAttrValue::WordType(noun.to_string()));
        
        // 转换其他属性
        for (key, value) in &attr.attributes {
            let named_value = match value {
                AttributeValue::Int(v) => NamedAttrValue::IntegerType(*v),
                AttributeValue::Float(v) => NamedAttrValue::F32Type(*v),
                AttributeValue::String(v) => {
                    // 尝试解析 JSON 格式的 Vec3
                    if v.starts_with('[') && v.ends_with(']') {
                        if let Ok(vec3) = Self::parse_vec3_from_json(v) {
                            NamedAttrValue::Vec3Type(vec3)
                        } else {
                            NamedAttrValue::StringType(v.clone())
                        }
                    } else {
                        NamedAttrValue::StringType(v.clone())
                    }
                }
                // Vec3 type is handled via JSON parsing in the String case above
                AttributeValue::Bool(v) => NamedAttrValue::BoolType(*v),
            };
            map.insert(key.clone(), named_value);
        }
        
        Ok(NamedAttrMap { map })
    }
    
    /// 从 JSON 字符串解析 Vec3
    fn parse_vec3_from_json(s: &str) -> Result<Vec3> {
        let json: serde_json::Value = serde_json::from_str(s)?;
        if let Some(arr) = json.as_array() {
            if arr.len() >= 3 {
                let x = arr[0].as_f64().ok_or_else(|| anyhow!("Invalid x"))? as f32;
                let y = arr[1].as_f64().ok_or_else(|| anyhow!("Invalid y"))? as f32;
                let z = arr[2].as_f64().ok_or_else(|| anyhow!("Invalid z"))? as f32;
                return Ok(Vec3::new(x, y, z));
            }
        }
        Err(anyhow!("Invalid Vec3 JSON format"))
    }
    
    /// 批量生成基本体（并行）
    pub async fn build_batch(&self, refnos: &[RefnoEnum]) -> Vec<EleInstGeosData> {
        use futures::stream::{self, StreamExt};
        
        // 使用异步并发处理（因为需要 async 调用 get_world_transform）
        let results = stream::iter(refnos)
            .map(|refno| async move {
                match self.build_primitive(*refno).await {
                    Ok(Some(data)) => Some(data),
                    Ok(None) => None,
                    Err(e) => {
                        eprintln!("⚠️  Failed to build primitive {}: {}", refno, e);
                        None
                    }
                }
            })
            .buffer_unordered(8)  // 并发数
            .collect::<Vec<_>>()
            .await;
        
        results.into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_vec3_from_json() {
        let vec3 = PrimitiveBuilder::parse_vec3_from_json("[1.0, 2.0, 3.0]").unwrap();
        assert_eq!(vec3, Vec3::new(1.0, 2.0, 3.0));
    }
}
