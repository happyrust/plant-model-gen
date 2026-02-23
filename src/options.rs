use aios_core::options::DbOption;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

/// 校验数据源模式（`use_cache` / `use_surrealdb`）是否合法。
///
/// 规则：两者必须严格互斥，且恰好一个为 `true`。
pub fn validate_data_source_mode(use_cache: bool, use_surrealdb: bool) -> anyhow::Result<()> {
    match (use_cache, use_surrealdb) {
        (true, false) | (false, true) => Ok(()),
        _ => anyhow::bail!(
            "非法数据源模式: use_cache={}, use_surrealdb={}。\
             规则要求两者严格互斥且恰好一个为 true。\
             推荐配置：cache 模式(use_cache=true,use_surrealdb=false)；\
             Surreal 模式(use_cache=false,use_surrealdb=true)。",
            use_cache,
            use_surrealdb
        ),
    }
}

/// 生成的网格模型格式
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MeshFormat {
    /// 原始二进制 PdmsMesh 格式 (.mesh)
    PdmsMesh,
    /// GLB 格式 (.glb)
    Glb,
    /// OBJ 格式 (.obj)
    Obj,
}

impl Default for MeshFormat {
    fn default() -> Self {
        Self::PdmsMesh
    }
}

/// 扩展DbOption，添加异地部署相关的配置
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbOptionExt {
    #[serde(flatten)]
    pub inner: DbOption,

    /// 模型生成完成后，是否导出 instances_{dbnum}.json（输出到 output/instances/instances_{dbnum}.json）
    #[serde(default = "default_true")]
    pub export_instances: bool,

    /// 预烘 TriMesh(L0) 输出目录（默认 meshes/trimesh_L0）
    #[serde(default)]
    pub trimesh_l0_dir: Option<String>,

    /// MQTT服务器地址，用于异地部署
    #[serde(default)]
    pub mqtt_server: Option<String>,

    /// MQTT服务器端口，用于异地部署
    #[serde(default)]
    pub mqtt_port: Option<u16>,

    /// HTTP数据服务器地址，用于异地部署
    #[serde(default)]
    pub http_server: Option<String>,

    /// HTTP数据服务器端口，用于异地部署
    #[serde(default)]
    pub http_port: Option<u16>,

    /// 目标会话号，用于历史模型生成
    #[serde(default)]
    pub target_sesno: Option<u32>,

    /// IndexTree 模式下同时进行的 Noun 级任务数量
    /// 默认为 None 时使用合理的并发数（如 CPU 核数）
    #[serde(default)]
    pub index_tree_max_concurrent_targets: Option<usize>,

    /// IndexTree 模式下单个 Noun 的 refno 列表按批次切分的大小
    /// 默认为 None 时复用 gen_model_batch_size
    #[serde(default)]
    pub index_tree_batch_size: Option<usize>,

    /// IndexTree 模式下启用的 noun 类别列表
    /// 可选值: "cate", "loop", "prim" 或具体 noun 名称如 "BRAN", "PANE"
    /// 空 vec 表示启用所有类别（默认行为）
    #[serde(default)]
    pub index_tree_enabled_target_types: Vec<String>,

    /// IndexTree 模式下禁用的 noun 列表
    /// 即使类别启用，这里的 noun 也会被过滤掉
    #[serde(default)]
    pub index_tree_excluded_target_types: Vec<String>,

    /// 调试模式：限制每种 Noun 类型的处理数量
    /// 设置为 None 或 0 表示不限制，设置为具体数字则只处理前 N 个实例
    /// 用于快速测试和调试，避免处理全库数据
    #[serde(default)]
    pub index_tree_debug_limit_per_target_type: Option<usize>,

    /// 生成的模型格式列表
    /// 默认为 [PdmsMesh]
    #[serde(default)]
    pub mesh_formats: Vec<MeshFormat>,

    /// 是否启用 SurrealDB 模型路径
    ///
    /// 注意：与 `use_cache` 必须严格互斥，且恰好一个为 true。
    /// - `true`  => SurrealDB 模式（`use_cache` 必须为 false）
    /// - `false` => 非 SurrealDB 模式
    #[serde(default = "default_false")]
    pub use_surrealdb: bool,

    /// 是否启用 foyer 缓存路径
    ///
    /// 注意：与 `use_surrealdb` 必须严格互斥，且恰好一个为 true。
    /// - `true`  => cache 模式（`use_surrealdb` 必须为 false）
    /// - `false` => 非 cache 模式
    #[serde(default = "default_true")]
    pub use_cache: bool,

    /// 是否双路径对比（主路径 + 副路径）
    #[serde(default)]
    pub dual_run_enabled: bool,

    /// 双路径下主路径是否为缓存
    #[serde(default = "default_true")]
    pub foyer_primary: bool,

    /// 副路径是否允许写入 SurrealDB
    #[serde(default = "default_true")]
    pub secondary_db_write: bool,

    /// foyer 缓存目录（默认 output/instance_cache）
    #[serde(default)]
    pub foyer_cache_dir: Option<String>,

    /// 副路径 mesh 输出目录（默认 output/meshes_shadow）
    #[serde(default)]
    pub secondary_mesh_dir: Option<String>,
}

impl Deref for DbOptionExt {
    type Target = DbOption;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for DbOptionExt {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl DbOptionExt {
    /// 获取 IndexTree 模式下的实际并发数
    /// 如果未配置，返回 CPU 核数（最小为 2，最大为 8）
    pub fn get_index_tree_concurrency(&self) -> usize {
        self.index_tree_max_concurrent_targets.unwrap_or_else(|| {
            let cpu_count = num_cpus::get();
            cpu_count.clamp(2, 8)
        })
    }

    /// 获取 IndexTree 模式下的实际批次大小
    /// 如果未配置，复用 gen_model_batch_size
    pub fn get_index_tree_batch_size(&self) -> usize {
        self.index_tree_batch_size
            .unwrap_or(self.inner.gen_model_batch_size)
    }

    /// 获取预烘 TriMesh(L0) 目录，默认在 meshes/trimesh_L0
    pub fn get_trimesh_l0_dir(&self) -> std::path::PathBuf {
        let base = self.inner.get_meshes_path();
        let dir = self
            .trimesh_l0_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| base.join("trimesh_L0"));
        // 确保目录存在（若创建失败，调用侧再处理）
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("创建 trimesh L0 目录失败: {}, err={}", dir.display(), e);
        }
        dir
    }

    /// 检查 noun 类别是否启用
    /// 空列表表示启用所有类别
    pub fn is_noun_category_enabled(&self, category: &str) -> bool {
        self.index_tree_enabled_target_types.is_empty()
            || self
                .index_tree_enabled_target_types
                .iter()
                .any(|cat| cat == category || cat.to_lowercase() == category.to_lowercase())
    }

    /// 检查具体 noun 是否被排除
    pub fn is_noun_excluded(&self, noun: &str) -> bool {
        self.index_tree_excluded_target_types
            .iter()
            .any(|excluded| excluded == noun || excluded.to_lowercase() == noun.to_lowercase())
    }

    /// 检查具体 noun 是否在启用的列表中（当使用具体 noun 名称时）
    pub fn is_noun_explicitly_enabled(&self, noun: &str) -> bool {
        // 如果启用了具体 noun 名称，则检查
        !self.index_tree_enabled_target_types.is_empty()
            && (self.index_tree_enabled_target_types.iter()
                .any(|cat| cat == noun || cat.to_lowercase() == noun.to_lowercase())
                // 也检查类别名称
                || self.is_noun_category_enabled(noun))
    }

    /// 获取带 project_name 前缀的 output 基础目录
    ///
    /// - 如果 project_name 非空，返回 `output/{project_name}`
    /// - 如果 project_name 为空，panic 报错
    pub fn get_project_output_dir(&self) -> std::path::PathBuf {
        let project_name = &self.inner.project_name;
        if project_name.is_empty() {
            panic!("project_name 不能为空，请在配置文件中设置 project_name");
        }
        std::path::PathBuf::from("output").join(project_name)
    }

    /// 获取 foyer 缓存目录，默认为 output/{project_name}/instance_cache
    ///
    /// 注意：如果用户已自定义 foyer_cache_dir，则直接使用用户配置
    pub fn get_foyer_cache_dir(&self) -> std::path::PathBuf {
        if let Some(ref custom_dir) = self.foyer_cache_dir {
            return std::path::PathBuf::from(custom_dir);
        }
        self.get_project_output_dir().join("instance_cache")
    }

    /// 获取副路径 mesh 输出目录，默认为 output/{project_name}/meshes_shadow
    ///
    /// 注意：如果用户已自定义 secondary_mesh_dir，则直接使用用户配置
    pub fn get_secondary_mesh_dir(&self) -> std::path::PathBuf {
        if let Some(ref custom_dir) = self.secondary_mesh_dir {
            return std::path::PathBuf::from(custom_dir);
        }
        self.get_project_output_dir().join("meshes_shadow")
    }

    /// 获取 scene_tree 目录，默认为 output/{project_name}/scene_tree
    pub fn get_scene_tree_dir(&self) -> std::path::PathBuf {
        self.get_project_output_dir().join("scene_tree")
    }

    /// 获取 db_meta_info.json 路径
    pub fn get_db_meta_info_path(&self) -> std::path::PathBuf {
        self.get_scene_tree_dir().join("db_meta_info.json")
    }
}

impl From<DbOption> for DbOptionExt {
    fn from(option: DbOption) -> Self {
        Self {
            inner: option,
            export_instances: true,
            trimesh_l0_dir: None,
            mqtt_server: None,
            mqtt_port: None,
            http_server: None,
            http_port: None,
            target_sesno: None,
            index_tree_max_concurrent_targets: None,
            index_tree_batch_size: None,
            index_tree_enabled_target_types: Vec::new(),
            index_tree_excluded_target_types: Vec::new(),
            index_tree_debug_limit_per_target_type: None,
            mesh_formats: vec![MeshFormat::PdmsMesh],
            use_surrealdb: false,
            use_cache: true,
            dual_run_enabled: false,
            foyer_primary: true,
            secondary_db_write: true,
            foyer_cache_dir: None,
            secondary_mesh_dir: None,
        }
    }
}

/// 获取扩展的数据库选项
pub fn get_db_option_ext() -> DbOptionExt {
    let db_option = aios_core::get_db_option();
    let db_option_ext = DbOptionExt::from(db_option.clone());
    if let Err(e) = validate_data_source_mode(db_option_ext.use_cache, db_option_ext.use_surrealdb) {
        panic!("DbOptionExt 数据源模式校验失败: {}", e);
    }
    db_option_ext
}

/// 从指定路径加载扩展的数据库选项
pub fn get_db_option_ext_from_path(config_path: &str) -> anyhow::Result<DbOptionExt> {
    use config::{Config, File};

    // 使用 config crate 加载基础 DbOption
    let s = Config::builder()
        .add_source(File::with_name(config_path))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build config: {}", e))?;

    let db_option = s
        .try_deserialize::<DbOption>()
        .map_err(|e| anyhow::anyhow!("Failed to deserialize DbOption: {}", e))?;

    // 读取 TOML 文件内容以提取扩展字段
    let config_file = format!("{}.toml", config_path);
    let content = std::fs::read_to_string(&config_file)
        .map_err(|e| anyhow::anyhow!("Failed to read config file {}: {}", config_file, e))?;

    // 解析 TOML 以提取扩展字段
    let toml_value: toml::Value = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse TOML from {}: {}", config_file, e))?;

    // 不兼容旧键：发现即报错，避免静默误跑
    let legacy_key_mapping = [
        ("full_noun_mode", "(已移除，IndexTree 现在是默认管线)"),
        (
            "full_noun_max_concurrent_nouns",
            "index_tree_max_concurrent_targets",
        ),
        ("full_noun_batch_size", "index_tree_batch_size"),
        (
            "full_noun_enabled_categories",
            "index_tree_enabled_target_types",
        ),
        (
            "full_noun_excluded_nouns",
            "index_tree_excluded_target_types",
        ),
        (
            "debug_limit_per_noun",
            "index_tree_debug_limit_per_target_type",
        ),
    ];
    let legacy_hits: Vec<(&str, &str)> = legacy_key_mapping
        .iter()
        .copied()
        .filter(|(legacy, _)| toml_value.get(*legacy).is_some())
        .collect();
    if !legacy_hits.is_empty() {
        let migration = legacy_hits
            .iter()
            .map(|(legacy, new_key)| format!("{} -> {}", legacy, new_key))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow::anyhow!(
            "配置文件 {} 使用了已移除的旧键，请迁移后重试: {}",
            config_file,
            migration
        ));
    }

    let index_tree_max_concurrent_targets = toml_value
        .get("index_tree_max_concurrent_targets")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    let index_tree_batch_size = toml_value
        .get("index_tree_batch_size")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    // 解析启用的 noun 类别
    let index_tree_enabled_target_types = toml_value
        .get("index_tree_enabled_target_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 解析禁用的 noun 列表
    let index_tree_excluded_target_types = toml_value
        .get("index_tree_excluded_target_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 解析调试限制
    let index_tree_debug_limit_per_target_type = toml_value
        .get("index_tree_debug_limit_per_target_type")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .filter(|&v| v > 0); // 0 表示不限制，转换为 None

    // 解析预烘 TriMesh(L0) 目录
    let trimesh_l0_dir = toml_value
        .get("trimesh_l0_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 是否在模型生成完毕后导出 instances.json
    // 默认 true（不开关也会导出，除非显式设为 false）
    let export_instances = toml_value
        .get("export_instances")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // 解析输出格式
    let mesh_formats = toml_value
        .get("mesh_formats")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.as_str().and_then(|s| match s.to_lowercase().as_str() {
                        "pdmsmesh" | "mesh" => Some(MeshFormat::PdmsMesh),
                        "glb" => Some(MeshFormat::Glb),
                        "obj" => Some(MeshFormat::Obj),
                        _ => None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![MeshFormat::PdmsMesh]);

    // 解析缓存/双路径配置
    let use_surrealdb = toml_value
        .get("use_surrealdb")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let use_cache = toml_value
        .get("use_cache")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let dual_run_enabled = toml_value
        .get("dual_run_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let foyer_primary = toml_value
        .get("foyer_primary")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let secondary_db_write = toml_value
        .get("secondary_db_write")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let foyer_cache_dir = toml_value
        .get("foyer_cache_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let secondary_mesh_dir = toml_value
        .get("secondary_mesh_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 构建 DbOptionExt
    let db_option_ext = DbOptionExt {
        inner: db_option,
        export_instances,
        trimesh_l0_dir,
        mqtt_server: None,
        mqtt_port: None,
        http_server: None,
        http_port: None,
        target_sesno: None,
        index_tree_max_concurrent_targets,
        index_tree_batch_size,
        index_tree_enabled_target_types,
        index_tree_excluded_target_types,
        index_tree_debug_limit_per_target_type,
        mesh_formats,
        use_surrealdb,
        use_cache,
        dual_run_enabled,
        foyer_primary,
        secondary_db_write,
        foyer_cache_dir,
        secondary_mesh_dir,
    };

    validate_data_source_mode(db_option_ext.use_cache, db_option_ext.use_surrealdb).map_err(
        |e| {
            anyhow::anyhow!(
                "配置文件 {} 数据源模式非法: {}",
                config_file,
                e
            )
        },
    )?;

    // 打印加载的配置
    println!("📋 加载的配置:");
    println!(
        "   - default_lod: {:?}",
        db_option_ext.inner.mesh_precision.default_lod
    );
    println!(
        "   - LOD profiles 数量: {}",
        db_option_ext.inner.mesh_precision.lod_profiles.len()
    );
    if !db_option_ext.index_tree_enabled_target_types.is_empty() {
        println!(
            "   - 启用的 noun 类别: {:?}",
            db_option_ext.index_tree_enabled_target_types
        );
    }
    if !db_option_ext.index_tree_excluded_target_types.is_empty() {
        println!(
            "   - 排除的 noun: {:?}",
            db_option_ext.index_tree_excluded_target_types
        );
    }

    Ok(db_option_ext)
}

#[cfg(test)]
mod tests {
    use super::validate_data_source_mode;

    #[test]
    fn data_source_mode_accepts_only_exclusive_true() {
        assert!(validate_data_source_mode(true, false).is_ok());
        assert!(validate_data_source_mode(false, true).is_ok());
        assert!(validate_data_source_mode(true, true).is_err());
        assert!(validate_data_source_mode(false, false).is_err());
    }
}
