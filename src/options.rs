use aios_core::options::DbOption;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn parse_defer_db_write(_raw: Option<bool>) -> bool {
    false
}

fn default_boolean_pipeline_mode() -> BooleanPipelineMode {
    BooleanPipelineMode::DbLegacy
}

fn default_regen_delete_mode() -> RegenDeleteMode {
    RegenDeleteMode::Legacy
}

fn default_model_writer_mode() -> ModelWriterMode {
    ModelWriterMode::Surreal
}

fn default_transform_write_backend() -> TransformWriteBackend {
    TransformWriteBackend::Surreal
}

fn default_transform_read_backend() -> TransformReadBackend {
    TransformReadBackend::Auto
}

fn default_batch_channel_capacity() -> usize {
    100
}

fn default_base_write_concurrency() -> usize {
    8
}

fn default_mesh_compute_concurrency() -> usize {
    4
}

fn default_inst_aabb_write_concurrency() -> usize {
    2
}

fn parse_regen_delete_mode(raw: Option<&str>) -> RegenDeleteMode {
    match raw.map(|s| s.to_ascii_lowercase()) {
        // refno_assoc_index 已硬关闭：为兼容旧配置，这里统一降级为 Legacy。
        Some(mode) if mode == "refno_assoc_index" => RegenDeleteMode::Legacy,
        Some(_) => RegenDeleteMode::Legacy,
        None => RegenDeleteMode::Legacy,
    }
}

fn parse_model_writer_mode(raw: Option<&str>) -> ModelWriterMode {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "drain-only" || mode == "drain_only" || mode == "drain" => {
            ModelWriterMode::DrainOnly
        }
        Some(mode) if mode == "parquet" => ModelWriterMode::Parquet,
        Some(_) | None => ModelWriterMode::Surreal,
    }
}

pub fn parse_transform_write_backend(raw: Option<&str>) -> TransformWriteBackend {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "parquet" => TransformWriteBackend::Parquet,
        Some(mode) if mode == "ducklake" => TransformWriteBackend::DuckLake,
        Some(mode) if mode == "dual" => TransformWriteBackend::Dual,
        Some(_) | None => TransformWriteBackend::Surreal,
    }
}

pub fn parse_transform_read_backend(raw: Option<&str>) -> TransformReadBackend {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "surreal" => TransformReadBackend::Surreal,
        Some(mode) if mode == "parquet" => TransformReadBackend::Parquet,
        Some(mode) if mode == "ducklake" => TransformReadBackend::DuckLake,
        Some(mode) if mode == "rkyv" => TransformReadBackend::Rkyv,
        Some(mode) if mode == "memory" => TransformReadBackend::Memory,
        Some(_) | None => TransformReadBackend::Auto,
    }
}

pub fn parse_transform_compare_backends(raw: Option<&str>) -> Vec<TransformReadBackend> {
    raw.map(|s| {
        s.split(',')
            .map(|part| parse_transform_read_backend(Some(part)))
            .filter(|backend| *backend != TransformReadBackend::Auto)
            .collect()
    })
    .unwrap_or_default()
}

/// 校验数据源模式是否符合当前固定策略。
///
/// 当前策略：输入数据固定读取 SurrealDB。
/// - `use_surrealdb = true`
pub fn validate_data_source_mode(use_surrealdb: bool) -> anyhow::Result<()> {
    if use_surrealdb {
        Ok(())
    } else {
        anyhow::bail!(
            "非法数据源模式: use_surrealdb=false。当前版本已固定输入来源为 SurrealDB，必须满足 use_surrealdb=true。"
        )
    }
}

/// 生成的网格模型格式
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

/// 模型生成结果写入后端。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ModelWriterMode {
    /// 写回 SurrealDB 模型表，保持当前默认行为。
    Surreal,
    /// 只消费生成端 batch 并输出统计，不持久化，用于压测生成吞吐。
    DrainOnly,
    /// File-oriented backend：通过 `CanonicalRawPlanner` 把 batch 转 canonical
    /// raw records，落 JSONL（Phase 1 fallback）到 `parquet_model_writer_output_root`
    /// 指定的目录。typed Parquet 物化推迟到 v4。
    Parquet,
}

impl Default for ModelWriterMode {
    fn default() -> Self {
        Self::Surreal
    }
}

impl ModelWriterMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Surreal => "surreal",
            Self::DrainOnly => "drain-only",
            Self::Parquet => "parquet",
        }
    }

    pub fn writes_to_surreal(&self) -> bool {
        matches!(self, Self::Surreal)
    }
}

/// pe_transform 刷新结果写入后端。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransformWriteBackend {
    /// 写入 SurrealDB pe_transform，保持当前默认行为。
    Surreal,
    /// 写入独立 pe_transform Parquet 文件。
    Parquet,
    /// 写入 Parquet 后生成/执行 DuckLake 注册。
    DuckLake,
    /// 双写 SurrealDB + Parquet/DuckLake，用于对比。
    Dual,
}

impl Default for TransformWriteBackend {
    fn default() -> Self {
        Self::Surreal
    }
}

impl TransformWriteBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Surreal => "surreal",
            Self::Parquet => "parquet",
            Self::DuckLake => "ducklake",
            Self::Dual => "dual",
        }
    }

    pub fn writes_to_surreal(&self) -> bool {
        matches!(self, Self::Surreal | Self::Dual)
    }

    pub fn writes_to_parquet(&self) -> bool {
        matches!(self, Self::Parquet | Self::DuckLake | Self::Dual)
    }

    pub fn uses_ducklake(&self) -> bool {
        matches!(self, Self::DuckLake)
    }
}

/// pe_transform 读取/对比后端。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransformReadBackend {
    Auto,
    Surreal,
    Parquet,
    DuckLake,
    Rkyv,
    Memory,
}

impl Default for TransformReadBackend {
    fn default() -> Self {
        Self::Auto
    }
}

impl TransformReadBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Surreal => "surreal",
            Self::Parquet => "parquet",
            Self::DuckLake => "ducklake",
            Self::Rkyv => "rkyv",
            Self::Memory => "memory",
        }
    }

    pub fn needs_parquet_feature(&self) -> bool {
        matches!(self, Self::Parquet | Self::DuckLake)
    }

    pub fn needs_ducklake_feature(&self) -> bool {
        matches!(self, Self::DuckLake)
    }
}

/// 布尔运算管线模式
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BooleanPipelineMode {
    /// 旧路径：从 DB 扫描待处理布尔任务
    DbLegacy,
    /// 新路径：由内存任务驱动布尔计算
    MemoryTasks,
}

impl Default for BooleanPipelineMode {
    fn default() -> Self {
        Self::DbLegacy
    }
}

/// regen-model 删旧模式
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegenDeleteMode {
    /// 旧路径：多表查询后逐表删除
    Legacy,
    /// 已停用：历史上按 refno_assoc_index 聚合索引删除
    RefnoAssocIndex,
}

impl Default for RegenDeleteMode {
    fn default() -> Self {
        Self::Legacy
    }
}

/// 扩展DbOption，添加异地部署相关的配置
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbOptionExt {
    #[serde(flatten)]
    pub inner: DbOption,

    /// 模型生成完成后，是否导出 instances_{dbnum}.json（输出到 output/instances/instances_{dbnum}.json）
    #[serde(default = "default_false")]
    pub export_instances: bool,

    /// 模型生成完成后，是否按 manual_db_nums 自动导出 Parquet（instances/tubings/transforms 等）
    #[serde(default = "default_false")]
    pub export_parquet_after_gen: bool,

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

    /// 模型生成空跑模式：仅收集 refno 并记录日志，不执行几何生成、DB 写入等
    /// 用于第一步调试分析（如检查 24381_145019 是否进入处理管道）
    #[serde(default)]
    pub gen_model_dry_run: bool,

    /// 生成的模型格式列表
    /// 默认为 [PdmsMesh]
    #[serde(default)]
    pub mesh_formats: Vec<MeshFormat>,

    /// 是否启用 SurrealDB 输入路径（当前固定为 true）。
    #[serde(default = "default_true")]
    pub use_surrealdb: bool,

    /// model 缓存目录（默认 output/instance_cache）
    #[serde(default)]
    pub model_cache_dir: Option<String>,

    /// 延迟写入模式：模型生成阶段不写 SurrealDB，所有 SQL 输出到 .surql 文件。
    ///
    /// 启用后：
    /// - save_instance_data 写入 .surql 文件而非 project_primary_db()
    /// - 跳过 init_model_tables / reconcile_neg_relate / boolean / aabb 写入
    /// - 生成完成后可通过 --import-sql 导入
    #[serde(default)]
    pub defer_db_write: bool,

    /// 布尔运算执行模式
    #[serde(default = "default_boolean_pipeline_mode")]
    pub boolean_pipeline_mode: BooleanPipelineMode,

    /// 模型写入后端：surreal 写库，drain-only 仅消费统计，parquet 落 JSONL canonical raw 文件。
    #[serde(default = "default_model_writer_mode")]
    pub model_writer_mode: ModelWriterMode,

    /// `model_writer_mode=parquet` 时使用的输出根目录。
    /// 实际 layout 为 `<root>/model_writer_storage/raw/<table>/project_name=<project>/dbnum=<dbnum>/batch_<id>.jsonl`，
    /// 与 mission 05-parquet-writer.md §Layout 保持一致。
    #[serde(default)]
    pub parquet_model_writer_output_root: Option<String>,

    /// `model_writer_mode=parquet` 时使用的 dbnum 维度。
    /// 当前 gen-model 流程不天然携带 dbnum，默认 0；v4 把 dbnum 拉进
    /// `ModelWriterContext` 后从 context 取值。
    #[serde(default)]
    pub parquet_model_writer_dbnum: Option<u32>,

    /// Compare 模式 candidate backend：当 Some 时构造
    /// `CompareModelWriterBackend(primary=model_writer_mode, candidate=this)`
    /// 让 orchestrator 在不修改调用点的情况下完成双写 + 行级 diff 日志。
    /// 失败语义遵循 mission `03-writer-architecture.md` §Error handling
    /// （fail fast，禁止静默回退）。
    ///
    /// 约束：
    /// - 不允许等于 primary（自我比较无意义）
    /// - 不允许 `drain-only`（DrainOnly 是 baseline sink，非 candidate writer）
    #[serde(default)]
    pub model_writer_compare_with: Option<ModelWriterMode>,

    /// pe_transform 刷新结果写入后端。
    #[serde(default = "default_transform_write_backend")]
    pub transform_write_backend: TransformWriteBackend,

    /// pe_transform cache miss / 对比读取后端。
    #[serde(default = "default_transform_read_backend")]
    pub transform_read_backend: TransformReadBackend,

    /// pe_transform 对比模式中的读取后端列表。
    #[serde(default)]
    pub transform_compare_backends: Vec<TransformReadBackend>,

    /// pe_transform Parquet 输出目录，默认 output/{project_name}/pe_transform.
    #[serde(default)]
    pub transform_parquet_dir: Option<String>,

    /// DuckLake metadata.ducklake 路径。
    #[serde(default)]
    pub transform_ducklake_metadata: Option<String>,

    /// DuckLake 数据文件根目录。
    #[serde(default)]
    pub transform_ducklake_data_path: Option<String>,

    /// 刷新前是否清理目标 dbnum 的历史 pe_transform。
    #[serde(default)]
    pub clear_transform_before_refresh: bool,

    /// regen-model 删旧模式。
    ///
    /// 注意：`refno_assoc_index` 已停用；即使旧配置显式填写，也会在解析时统一降级到 `Legacy`。
    #[serde(default = "default_regen_delete_mode")]
    pub regen_delete_mode: RegenDeleteMode,

    /// 布尔运算前是否从 DB 批量补齐缺失的 cata 任务
    #[serde(default)]
    pub enable_db_backfill: bool,

    /// batch 级流水线 channel 容量
    #[serde(default = "default_batch_channel_capacity")]
    pub batch_channel_capacity: usize,

    /// 基础写库并发度
    #[serde(default = "default_base_write_concurrency")]
    pub base_write_concurrency: usize,

    /// mesh 计算并发度
    #[serde(default = "default_mesh_compute_concurrency")]
    pub mesh_compute_concurrency: usize,

    /// inst_relate_aabb 写入并发度
    #[serde(default = "default_inst_aabb_write_concurrency")]
    pub inst_aabb_write_concurrency: usize,
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
            .max(super::fast_model::gen_model::config::BatchSize::DEFAULT)
    }

    pub fn get_batch_channel_capacity(&self) -> usize {
        self.batch_channel_capacity.max(1)
    }

    pub fn get_base_write_concurrency(&self) -> usize {
        self.base_write_concurrency.max(1)
    }

    pub fn get_mesh_compute_concurrency(&self) -> usize {
        self.mesh_compute_concurrency.max(1)
    }

    pub fn get_inst_aabb_write_concurrency(&self) -> usize {
        self.inst_aabb_write_concurrency.max(1)
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

    /// 获取 model 缓存目录，默认为 output/{project_name}/instance_cache
    ///
    /// 注意：如果用户已自定义 model_cache_dir，则直接使用用户配置
    pub fn get_model_cache_dir(&self) -> std::path::PathBuf {
        if let Some(ref custom_dir) = self.model_cache_dir {
            return std::path::PathBuf::from(custom_dir);
        }
        self.get_project_output_dir().join("instance_cache")
    }

    /// 获取 pe_transform Parquet 输出目录。
    pub fn get_transform_parquet_dir(&self) -> std::path::PathBuf {
        self.transform_parquet_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.get_project_output_dir().join("pe_transform"))
    }

    pub fn get_transform_ducklake_metadata_path(&self) -> std::path::PathBuf {
        self.transform_ducklake_metadata
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                self.get_project_output_dir()
                    .join("ducklake")
                    .join("metadata.ducklake")
            })
    }

    pub fn get_transform_ducklake_data_path(&self) -> std::path::PathBuf {
        self.transform_ducklake_data_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.get_project_output_dir().join("ducklake").join("data"))
    }

    /// 获取 scene_tree 目录，默认为 output/{project_name}/scene_tree
    pub fn get_scene_tree_dir(&self) -> std::path::PathBuf {
        self.get_project_output_dir().join("scene_tree")
    }

    /// 获取 foyer cache 目录（兼容旧代码路径），默认为 model_cache_dir
    pub fn get_foyer_cache_dir(&self) -> std::path::PathBuf {
        self.get_model_cache_dir()
    }

    /// 获取 db_meta_info.json 路径
    pub fn get_db_meta_info_path(&self) -> std::path::PathBuf {
        self.get_scene_tree_dir().join("db_meta_info.json")
    }

    pub fn validate_model_writer_features(&self) -> anyhow::Result<()> {
        // 1) feature flag 守卫：编译期 feature 集不匹配 → 拒绝
        match self.model_writer_mode {
            ModelWriterMode::Surreal
                if !cfg!(any(
                    feature = "write-to-surrealdb",
                    feature = "surreal-save"
                )) =>
            {
                anyhow::bail!(
                    "model_writer=surreal 需要编译 feature `surreal-save` 或 `write-to-surrealdb`；请使用 mission cargo check feature 集或 --features \"review\""
                )
            }
            // drain-only is a non-persistent throughput sink and does not need a storage feature.
            ModelWriterMode::DrainOnly => return Ok(()),
            // parquet 走文件系统，无需 storage feature；但需要 output_root 配置（运行时守卫见下方 2b）。
            ModelWriterMode::Parquet => {}
            _ => {}
        }

        // 2) 运行时配置组合守卫：Surreal backend 必须配 use_surrealdb=true
        if matches!(self.model_writer_mode, ModelWriterMode::Surreal) && !self.use_surrealdb {
            anyhow::bail!(
                "model_writer=surreal 要求 use_surrealdb=true；当前 use_surrealdb=false 是非法组合（早期拒绝以避免空跑 perf init / pre_check）"
            );
        }

        // 2b) Parquet backend 必须配 parquet_model_writer_output_root
        if matches!(self.model_writer_mode, ModelWriterMode::Parquet) {
            let root = self
                .parquet_model_writer_output_root
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            if root.is_none() {
                anyhow::bail!(
                    "model_writer=parquet 要求 parquet_model_writer_output_root 非空；请在 DbOption 设置目标目录（参考 mission 05-parquet-writer.md §Layout）"
                );
            }
        }

        // 2c) compare_with 合法性早期拒绝（mission 03 §Error handling: fail fast）
        if let Some(candidate) = self.model_writer_compare_with {
            if candidate == self.model_writer_mode {
                anyhow::bail!(
                    "model_writer_compare_with={} 与 model_writer={} 一致；移除 compare_with 或换不同后端",
                    candidate.as_str(),
                    self.model_writer_mode.as_str()
                );
            }
            if matches!(candidate, ModelWriterMode::DrainOnly) {
                anyhow::bail!(
                    "model_writer_compare_with=drain-only 不被支持：DrainOnly 是 baseline 模式而非 candidate writer（mission docs/05 + v2 invariant）"
                );
            }
            // candidate=parquet 需要 output_root；复用 2b 守卫语义
            if matches!(candidate, ModelWriterMode::Parquet) {
                let root = self
                    .parquet_model_writer_output_root
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());
                if root.is_none() {
                    anyhow::bail!(
                        "model_writer_compare_with=parquet 要求 parquet_model_writer_output_root 非空"
                    );
                }
            }
        }

        Ok(())
    }

    pub fn validate_transform_store_features(&self) -> anyhow::Result<()> {
        let parquet_requested = self.transform_write_backend.writes_to_parquet()
            || self.transform_read_backend.needs_parquet_feature()
            || self
                .transform_compare_backends
                .iter()
                .any(TransformReadBackend::needs_parquet_feature);
        if parquet_requested && !cfg!(feature = "transform-store-parquet") {
            anyhow::bail!(
                "pe_transform parquet/ducklake 后端需要编译 feature `transform-store-parquet`；例如 --features \"review,transform-store-parquet\""
            );
        }

        let ducklake_requested = self.transform_write_backend.uses_ducklake()
            || self.transform_read_backend.needs_ducklake_feature()
            || self
                .transform_compare_backends
                .iter()
                .any(TransformReadBackend::needs_ducklake_feature);
        if ducklake_requested && !cfg!(feature = "transform-store-ducklake") {
            anyhow::bail!(
                "pe_transform ducklake 后端需要编译 feature `transform-store-ducklake`；例如 --features \"review,transform-store-ducklake\""
            );
        }

        if !self.transform_compare_backends.is_empty() && !cfg!(feature = "transform-store-compare")
        {
            anyhow::bail!(
                "pe_transform compare 模式需要编译 feature `transform-store-compare`；例如 --features \"review,transform-store-parquet,transform-store-compare\""
            );
        }

        Ok(())
    }
}

impl From<DbOption> for DbOptionExt {
    fn from(option: DbOption) -> Self {
        Self {
            inner: option,
            export_instances: false,
            export_parquet_after_gen: false,
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
            use_surrealdb: true,
            model_cache_dir: None,
            defer_db_write: false,
            boolean_pipeline_mode: BooleanPipelineMode::DbLegacy,
            model_writer_mode: ModelWriterMode::Surreal,
            parquet_model_writer_output_root: None,
            parquet_model_writer_dbnum: None,
            model_writer_compare_with: None,
            transform_write_backend: TransformWriteBackend::Surreal,
            transform_read_backend: TransformReadBackend::Auto,
            transform_compare_backends: Vec::new(),
            transform_parquet_dir: None,
            transform_ducklake_metadata: None,
            transform_ducklake_data_path: None,
            clear_transform_before_refresh: false,
            regen_delete_mode: RegenDeleteMode::Legacy,
            enable_db_backfill: false,
            gen_model_dry_run: false,
            batch_channel_capacity: default_batch_channel_capacity(),
            base_write_concurrency: default_base_write_concurrency(),
            mesh_compute_concurrency: default_mesh_compute_concurrency(),
            inst_aabb_write_concurrency: default_inst_aabb_write_concurrency(),
        }
    }
}

/// 获取扩展的数据库选项
pub fn get_db_option_ext() -> DbOptionExt {
    let db_option = aios_core::get_db_option();
    let db_option_ext = DbOptionExt::from((*db_option).clone());
    if let Err(e) = validate_data_source_mode(db_option_ext.use_surrealdb) {
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
        .unwrap_or(false);

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

    // 数据源策略已固定为 SurrealDB 输入。
    let use_surrealdb = true;

    let model_cache_dir = toml_value
        .get("model_cache_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let defer_db_write =
        parse_defer_db_write(toml_value.get("defer_db_write").and_then(|v| v.as_bool()));

    let boolean_pipeline_mode = toml_value
        .get("boolean_pipeline_mode")
        .and_then(|v| v.as_str())
        .map(|s| match s.to_ascii_lowercase().as_str() {
            "memory_tasks" => BooleanPipelineMode::MemoryTasks,
            _ => BooleanPipelineMode::DbLegacy,
        })
        .unwrap_or(BooleanPipelineMode::DbLegacy);

    let model_writer_mode = parse_model_writer_mode(
        toml_value
            .get("model_writer")
            .or_else(|| toml_value.get("model_writer_mode"))
            .and_then(|v| v.as_str()),
    );

    let transform_write_backend = parse_transform_write_backend(
        toml_value
            .get("transform_write_backend")
            .and_then(|v| v.as_str()),
    );

    let transform_read_backend = parse_transform_read_backend(
        toml_value
            .get("transform_read_backend")
            .and_then(|v| v.as_str()),
    );

    let transform_compare_backends = toml_value
        .get("transform_compare_backends")
        .and_then(|v| {
            if let Some(arr) = v.as_array() {
                Some(
                    arr.iter()
                        .filter_map(|item| item.as_str())
                        .map(|item| parse_transform_read_backend(Some(item)))
                        .filter(|backend| *backend != TransformReadBackend::Auto)
                        .collect::<Vec<_>>(),
                )
            } else {
                v.as_str().map(|s| parse_transform_compare_backends(Some(s)))
            }
        })
        .unwrap_or_default();

    let transform_parquet_dir = toml_value
        .get("transform_parquet_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let transform_ducklake_metadata = toml_value
        .get("transform_ducklake_metadata")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let transform_ducklake_data_path = toml_value
        .get("transform_ducklake_data_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let clear_transform_before_refresh = toml_value
        .get("clear_transform_before_refresh")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let regen_delete_mode =
        parse_regen_delete_mode(toml_value.get("regen_delete_mode").and_then(|v| v.as_str()));

    let enable_db_backfill = toml_value
        .get("enable_db_backfill")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let gen_model_dry_run = toml_value
        .get("gen_model_dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let batch_channel_capacity = toml_value
        .get("batch_channel_capacity")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or_else(default_batch_channel_capacity);

    let base_write_concurrency = toml_value
        .get("base_write_concurrency")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or_else(default_base_write_concurrency);

    let mesh_compute_concurrency = toml_value
        .get("mesh_compute_concurrency")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or_else(default_mesh_compute_concurrency);

    let inst_aabb_write_concurrency = toml_value
        .get("inst_aabb_write_concurrency")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize)
        .unwrap_or_else(default_inst_aabb_write_concurrency);

    let export_parquet_after_gen = toml_value
        .get("export_parquet_after_gen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let parquet_model_writer_output_root = toml_value
        .get("parquet_model_writer_output_root")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let parquet_model_writer_dbnum = toml_value
        .get("parquet_model_writer_dbnum")
        .and_then(|v| v.as_integer())
        .and_then(|v| u32::try_from(v).ok());

    let model_writer_compare_with = toml_value
        .get("model_writer_compare_with")
        .and_then(|v| v.as_str())
        .and_then(|s| match s.trim().to_ascii_lowercase().as_str() {
            "surreal" => Some(ModelWriterMode::Surreal),
            "parquet" => Some(ModelWriterMode::Parquet),
            // drain-only is intentionally excluded (baseline sink, not a candidate writer);
            // unknown values fall through to None.
            _ => None,
        });

    // 构建 DbOptionExt
    let db_option_ext = DbOptionExt {
        inner: db_option,
        export_instances,
        export_parquet_after_gen,
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
        model_cache_dir,
        defer_db_write,
        boolean_pipeline_mode,
        model_writer_mode,
        parquet_model_writer_output_root,
        parquet_model_writer_dbnum,
        model_writer_compare_with,
        transform_write_backend,
        transform_read_backend,
        transform_compare_backends,
        transform_parquet_dir,
        transform_ducklake_metadata,
        transform_ducklake_data_path,
        clear_transform_before_refresh,
        regen_delete_mode,
        enable_db_backfill,
        gen_model_dry_run,
        batch_channel_capacity,
        base_write_concurrency,
        mesh_compute_concurrency,
        inst_aabb_write_concurrency,
    };

    validate_data_source_mode(db_option_ext.use_surrealdb)
        .map_err(|e| anyhow::anyhow!("配置文件 {} 数据源模式非法: {}", config_file, e))?;
    db_option_ext
        .validate_transform_store_features()
        .map_err(|e| anyhow::anyhow!("配置文件 {} transform backend 配置非法: {}", config_file, e))?;

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
    println!(
        "   - model_writer: {}",
        db_option_ext.model_writer_mode.as_str()
    );
    println!(
        "   - transform_write_backend: {}",
        db_option_ext.transform_write_backend.as_str()
    );
    println!(
        "   - transform_read_backend: {}",
        db_option_ext.transform_read_backend.as_str()
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
    println!(
        "   - boolean_pipeline_mode: {:?}",
        db_option_ext.boolean_pipeline_mode
    );
    println!(
        "   - regen_delete_mode: {:?}",
        db_option_ext.regen_delete_mode
    );
    if db_option_ext.enable_db_backfill {
        println!("   - enable_db_backfill: true");
    }

    Ok(db_option_ext)
}

#[cfg(test)]
mod tests {
    use super::{RegenDeleteMode, parse_regen_delete_mode, validate_data_source_mode};

    #[test]
    fn data_source_mode_requires_fixed_surreal_input() {
        assert!(validate_data_source_mode(true).is_ok());
        assert!(validate_data_source_mode(false).is_err());
    }

    #[test]
    fn regen_delete_mode_refno_assoc_index_is_forced_to_legacy() {
        assert_eq!(
            parse_regen_delete_mode(Some("refno_assoc_index")),
            RegenDeleteMode::Legacy
        );
    }
}
