use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_LIBRARY_PATH: &str = "assets/material/plant_pipeline_materials.json";

/// 默认灰色材质 (RGB: 144, 164, 174 -> #90a4ae)
const DEFAULT_GRAY_COLOR: [f32; 4] = [0.565, 0.643, 0.682, 1.0];

#[derive(Debug, Deserialize, Clone)]
pub struct MaterialLibraryFile {
    #[serde(default)]
    pub default_material: Option<String>,
    #[serde(default)]
    pub materials: Vec<MaterialDefinition>,
    #[serde(default)]
    pub noun_bindings: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MaterialDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "pbrMetallicRoughness")]
    #[serde(default)]
    pub pbr_metallic_roughness: Option<PbrMetallicRoughness>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct PbrMetallicRoughness {
    #[serde(default, rename = "baseColorFactor")]
    pub base_color_factor: Option<[f32; 4]>,
    #[serde(default, rename = "baseColorTexture")]
    pub base_color_texture: Option<TextureReference>,
    #[serde(default, rename = "metallicFactor")]
    pub metallic_factor: Option<f32>,
    #[serde(default, rename = "roughnessFactor")]
    pub roughness_factor: Option<f32>,
    #[serde(default, rename = "metallicRoughnessTexture")]
    pub metallic_roughness_texture: Option<TextureReference>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TextureReference {
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default, rename = "texCoord")]
    pub tex_coord: Option<u32>,
    #[serde(default)]
    pub scale: Option<f32>,
}

pub struct MaterialLibrary {
    materials: Vec<MaterialDefinition>,
    noun_bindings: HashMap<String, String>,
    index_map: HashMap<String, usize>,
    default_material: Option<String>,
    source_path: PathBuf,
}

impl MaterialLibrary {
    /// 创建一个使用默认灰色材质的空材质库
    pub fn with_default_gray() -> Self {
        let default_material = MaterialDefinition {
            name: "DefaultGray".to_string(),
            description: Some("默认灰色材质".to_string()),
            pbr_metallic_roughness: Some(PbrMetallicRoughness {
                base_color_factor: Some(DEFAULT_GRAY_COLOR),
                base_color_texture: None,
                metallic_factor: Some(0.1),
                roughness_factor: Some(0.5),
                metallic_roughness_texture: None,
            }),
        };

        let mut index_map = HashMap::new();
        index_map.insert("DefaultGray".to_string(), 0);

        Self {
            materials: vec![default_material],
            noun_bindings: HashMap::new(),
            index_map,
            default_material: Some("DefaultGray".to_string()),
            source_path: PathBuf::from("(内置默认)"),
        }
    }

    /// 加载默认材质库，如果文件不存在则使用默认灰色材质
    pub fn load_default() -> Result<Self> {
        match Self::load_from_path(DEFAULT_LIBRARY_PATH) {
            Ok(lib) => Ok(lib),
            Err(_) => {
                eprintln!("[材质库] 未找到 {}, 使用默认灰色材质", DEFAULT_LIBRARY_PATH);
                Ok(Self::with_default_gray())
            }
        }
    }

    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let text = fs::read_to_string(path_ref)
            .with_context(|| format!("读取材质库文件失败: {}", path_ref.display()))?;
        let file: MaterialLibraryFile = serde_json::from_str(&text)
            .with_context(|| format!("解析材质库 JSON 失败: {}", path_ref.display()))?;

        let mut index_map = HashMap::new();
        for (idx, material) in file.materials.iter().enumerate() {
            index_map.insert(material.name.clone(), idx);
        }

        Ok(Self {
            materials: file.materials,
            noun_bindings: file.noun_bindings,
            index_map,
            default_material: file.default_material,
            source_path: path_ref.to_path_buf(),
        })
    }

    pub fn material_index_for_noun(&self, noun: &str) -> Option<usize> {
        let key = noun.to_uppercase();
        if let Some(material_name) = self.noun_bindings.get(&key) {
            return self.index_map.get(material_name).copied();
        }
        if let Some(default_name) = self.default_material.as_ref() {
            return self.index_map.get(default_name).copied();
        }
        None
    }

    pub fn materials(&self) -> &[MaterialDefinition] {
        &self.materials
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// 获取或创建材质索引
    /// 仅使用材质库映射（不做颜色配置/动态材质创建）。
    pub fn get_or_create_material_for_noun(
        &self,
        noun: &str,
        _use_basic: bool,
        _dynamic_materials: &mut Vec<Value>,
    ) -> Option<usize> {
        if let Some(idx) = self.material_index_for_noun(noun) {
            return Some(idx);
        }
        // 兜底：如果材质库里有任何材质，返回第一个，避免上游出现 None 导致崩溃。
        (!self.materials.is_empty()).then_some(0)
    }
}

impl MaterialDefinition {
    pub fn to_gltf_material(&self) -> Value {
        let mut material = json!({
            "name": self.name
        });

        if let Some(desc) = &self.description {
            material["extras"] = json!({ "description": desc });
        }

        if let Some(pbr) = &self.pbr_metallic_roughness {
            let mut pbr_json = json!({});
            if let Some(color) = pbr.base_color_factor {
                pbr_json["baseColorFactor"] = json!(color);
            }
            if let Some(texture) = &pbr.base_color_texture {
                pbr_json["baseColorTexture"] = texture.to_gltf_value();
            }
            if let Some(metallic) = pbr.metallic_factor {
                pbr_json["metallicFactor"] = json!(metallic);
            }
            if let Some(roughness) = pbr.roughness_factor {
                pbr_json["roughnessFactor"] = json!(roughness);
            }
            if let Some(texture) = &pbr.metallic_roughness_texture {
                pbr_json["metallicRoughnessTexture"] = texture.to_gltf_value();
            }
            material["pbrMetallicRoughness"] = pbr_json;
        }

        material
    }

    /// 以基础颜色（非 PBR）导出材质：
    /// - 使用 KHR_materials_unlit 扩展
    /// - 仅输出 baseColorFactor（若存在），忽略 metallic/roughness 等 PBR 参数
    pub fn to_basic_unlit_gltf_material(&self) -> Value {
        let mut material = json!({
            "name": self.name
        });

        if let Some(desc) = &self.description {
            material["extras"] = json!({ "description": desc });
        }

        // 基础颜色来自 pbr.baseColorFactor，如果存在
        if let Some(pbr) = &self.pbr_metallic_roughness {
            let mut pbr_basic = json!({});
            if let Some(color) = pbr.base_color_factor {
                pbr_basic["baseColorFactor"] = json!(color);
            }
            // 仅提供 baseColorFactor 作为基础颜色承载
            if !pbr_basic.as_object().unwrap().is_empty() {
                material["pbrMetallicRoughness"] = pbr_basic;
            }
        }

        // 标记为 Unlit
        material["extensions"] = json!({
            "KHR_materials_unlit": {}
        });

        material
    }
}

impl TextureReference {
    pub fn to_gltf_value(&self) -> Value {
        let mut value = json!({});
        if let Some(index) = self.index {
            value["index"] = json!(index);
        }
        if let Some(tex_coord) = self.tex_coord {
            value["texCoord"] = json!(tex_coord);
        }
        if let Some(scale) = self.scale {
            value["scale"] = json!(scale);
        }
        value
    }
}
