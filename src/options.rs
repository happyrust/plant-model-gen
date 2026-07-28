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
    BooleanPipelineMode::MemoryTasks
}

fn default_version_retention() -> String {
    // specs/022：用户决策默认无限保留（"0"）；磁盘只增不减，站点需评估盘余量。
    "0".to_string()
}

/// 构造 SurrealDB RocksDB 连接串（用于 `surreal start` 与嵌入式 open）。
///
/// `versioned=true` 时追加 `?versioned=true&retention=<r>`，开启 fork 的
/// RocksDB user-defined-timestamps 版本化存储（specs/022）。
///
/// 注意：versioned 是建库属性——已存在的非版本化数据目录不能以
/// `versioned=true` 重新打开（UDT comparator 不匹配），必须新建数据目录重灌。
pub fn rocksdb_conn_str(data_path: &str, versioned: bool, retention: &str) -> String {
    debug_assert!(
        !data_path.contains('?'),
        "data_path 不应包含 query 参数: {data_path}"
    );
    if versioned {
        let retention = retention.trim();
        let retention = if retention.is_empty() { "0" } else { retention };
        format!("rocksdb://{data_path}?versioned=true&retention={retention}")
    } else {
        format!("rocksdb://{data_path}")
    }
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

fn parse_model_writer_mode(raw: Option<&str>) -> anyhow::Result<ModelWriterMode> {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "drain-only" || mode == "drain_only" || mode == "drain" => {
            Ok(ModelWriterMode::DrainOnly)
        }
        Some(mode) if mode == "ducklake" || mode == "duck-lake" || mode == "duck_lake" => {
            anyhow::bail!(
                "model_writer={mode} 已退役；请显式改为 surreal 或 drain-only，系统不会静默回退"
            )
        }
        Some(mode) if mode == "surreal" => Ok(ModelWriterMode::Surreal),
        Some(mode) => anyhow::bail!("未知 model_writer={mode}；仅支持 surreal 或 drain-only"),
        None => Ok(ModelWriterMode::Surreal),
    }
}

pub fn parse_transform_write_backend(raw: Option<&str>) -> anyhow::Result<TransformWriteBackend> {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "parquet" => Ok(TransformWriteBackend::Parquet),
        Some(mode) if mode == "ducklake" => anyhow::bail!(
            "transform_write_backend=ducklake 已退役；请显式改为 surreal、parquet 或 dual"
        ),
        Some(mode) if mode == "dual" => Ok(TransformWriteBackend::Dual),
        Some(mode) if mode == "surreal" => Ok(TransformWriteBackend::Surreal),
        Some(mode) => {
            anyhow::bail!("未知 transform_write_backend={mode}；仅支持 surreal、parquet 或 dual")
        }
        None => Ok(TransformWriteBackend::Surreal),
    }
}

pub fn parse_transform_read_backend(raw: Option<&str>) -> anyhow::Result<TransformReadBackend> {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "surreal" => Ok(TransformReadBackend::Surreal),
        Some(mode) if mode == "parquet" => Ok(TransformReadBackend::Parquet),
        Some(mode) if mode == "ducklake" => anyhow::bail!(
            "transform_read_backend=ducklake 已退役；请显式改为 auto、surreal、parquet、rkyv 或 memory"
        ),
        Some(mode) if mode == "rkyv" => Ok(TransformReadBackend::Rkyv),
        Some(mode) if mode == "memory" => Ok(TransformReadBackend::Memory),
        Some(mode) if mode == "auto" => Ok(TransformReadBackend::Auto),
        Some(mode) => anyhow::bail!(
            "未知 transform_read_backend={mode}；仅支持 auto、surreal、parquet、rkyv 或 memory"
        ),
        None => Ok(TransformReadBackend::Auto),
    }
}

pub fn parse_transform_compare_backends(
    raw: Option<&str>,
) -> anyhow::Result<Vec<TransformReadBackend>> {
    let mut backends = Vec::new();
    if let Some(raw) = raw {
        for part in raw.split(',') {
            let backend = parse_transform_read_backend(Some(part))?;
            if backend != TransformReadBackend::Auto {
                backends.push(backend);
            }
        }
    }
    Ok(backends)
}

/// 保留旧 `use_surrealdb` 配置的兼容校验入口。
///
/// 生成输入源由 `generation_read_backend` 显式决定；该旧开关不再固定输入后端。
/// Surreal versioned adapter 的可用性由 `validate_generation_read_features` 校验。
pub fn validate_data_source_mode(use_surrealdb: bool) -> anyhow::Result<()> {
    let _ = use_surrealdb;
    Ok(())
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

/// specs/027（ADR-0007）：DuckLake 退役后的配置分级检测。
///
/// - **行为键**残留（站点显式声明过的存储语义）→ 硬错误 + 修复指引：
///   `parse_storage_backend = "ducklake"`、`generation_read_backend = "ducklake"|"compare"`；
/// - **惰性键**残留（代码已不读取）→ 仅警告：
///   `ducklake_*`、`duckdb_*`、`generation_input_manifest`。
pub fn check_retired_ducklake_keys(raw_toml: &toml::Value) -> anyhow::Result<()> {
    let Some(table) = raw_toml.as_table() else {
        return Ok(());
    };

    if let Some(value) = table.get("parse_storage_backend").and_then(|v| v.as_str()) {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized == "ducklake" {
            anyhow::bail!(
                "parse_storage_backend=ducklake 已退役（ADR-0007：版本单源 = Surreal MVCC + sesno 锚点）。\
                 请删除该配置行（解析固定写 Surreal 主库）后重启。"
            );
        }
    }
    if let Some(value) = table
        .get("generation_read_backend")
        .and_then(|v| v.as_str())
    {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized == "ducklake" || normalized == "compare" {
            anyhow::bail!(
                "generation_read_backend={normalized} 已退役（ADR-0008：模型生成主表直读）。\
                 请删除该配置行后重启；双后端对比能力已随 DuckLake 退役移除。"
            );
        }
    }

    for key in table.keys() {
        let lower = key.to_ascii_lowercase();
        if lower.starts_with("ducklake_")
            || lower.starts_with("duckdb_")
            || lower == "generation_input_manifest"
            || lower == "generation_read_backend"
            || lower == "parse_storage_backend"
        {
            log::warn!(
                "配置键 `{key}` 已随 DuckLake 退役失效（ADR-0007/0008），当前被忽略；建议从配置文件删除"
            );
        }
    }
    Ok(())
}

/// specs/027 FR-015 / T011：history 解析配置退役检测。
///
/// - `sync_history = true` → **硬错误**：历史会话回放已随版本单源架构移除
///   （pdms_io fork 不再提供 sync_history，旧分支会静默跳过文件解析、丢数据）；
///   会话元数据请等待独立导入入口（spec 027 T010），数据历史由锚点体系提供。
/// - `sync_history = false` → 兼容期忽略并告警（建议删除该键）。
/// - `sync_versioned` 残留 → 已退役，仅告警。
pub fn check_retired_history_keys(raw_toml: &toml::Value) -> anyhow::Result<()> {
    let Some(table) = raw_toml.as_table() else {
        return Ok(());
    };
    if let Some(value) = table.get("sync_history").and_then(|v| v.as_bool()) {
        if value {
            anyhow::bail!(
                "sync_history=true 已退役（spec 027 FR-015：历史会话回放已随版本单源架构移除，\
                 旧分支只会静默跳过文件解析）。数据/模型历史由 sesno 锚点体系提供\
                 （incremental-sesno / model-version history *）；\
                 会话元数据请使用独立导入入口（spec 027 T010）。请删除该配置行后重试。"
            );
        }
        log::warn!(
            "配置键 `sync_history` 已退役（spec 027 FR-015），false 在兼容期被忽略；建议从配置文件删除"
        );
    }
    if table.contains_key("sync_versioned") {
        log::warn!(
            "配置键 `sync_versioned` 已退役（spec 027 FR-015），当前被忽略；建议从配置文件删除"
        );
    }
    Ok(())
}

/// specs/022/024：`version_retention` 格式校验。
///
/// 与 surreal fork `kvs::config::parse_duration` 语法对齐：纯秒数（`0`=无限保留）
/// 或 `数字+单位`（us/µs/ms/s/m/h/d，单单位、不允许 `30d8h` 复合）。
/// fork 端非法值会被 `.ok()` 静默吞掉退回默认，因此必须在部署侧 fail-loud。
pub fn validate_version_retention(value: &str) -> anyhow::Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("version_retention 不能为空字符串（无限保留请写 \"0\"）");
    }
    if trimmed.parse::<u64>().is_ok() {
        return Ok(());
    }
    let unit_start = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (num, unit) = trimmed.split_at(unit_start);
    let valid_unit = matches!(unit, "us" | "µs" | "ms" | "s" | "m" | "h" | "d");
    if num.is_empty() || !valid_unit || num.parse::<u64>().is_err() {
        anyhow::bail!(
            "version_retention=\"{trimmed}\" 格式非法：只接受纯秒数（\"0\"=无限保留）\
             或 数字+单位（us/ms/s/m/h/d，例如 \"90d\"），不支持复合单位；\
             surreal 端会静默忽略非法值并退回默认，必须在这里拦截"
        );
    }
    Ok(())
}

/// specs/022 建库属性防护：把 versioned 参数与数据目录绑定（P5 双轨真源收敛）。
///
/// `versioned` 是 RocksDB 建库属性（UDT comparator），但托管站点配置 / 全局
/// `DB_OPTION_FILE` 都是与目录分离的第二真源，配置翻转后直接启动只会得到
/// comparator mismatch 的晦涩报错。此函数在启动前用旁车标记文件
/// `<data_path>.plant3d-versioned.json` 固化目录的建库属性：
///
/// - 标记存在且 `versioned` 不一致 → 硬错误（指引新建目录重灌）；
/// - 标记存在且 retention 不同 → 更新标记（retention 可在线调整）；
/// - 标记缺失 + 目录已有数据 → 以当前配置为准补写标记并告警（兼容存量目录）；
/// - 标记缺失 + 目录为空/不存在 → 写入标记（新建目录）。
pub fn bind_versioned_dir_marker(
    data_path: &str,
    versioned: bool,
    retention: &str,
) -> anyhow::Result<()> {
    use std::path::Path;

    let marker_path = format!("{}.plant3d-versioned.json", data_path.trim_end_matches(['/', '\\']));
    let marker_path = Path::new(&marker_path);
    let dir = Path::new(data_path);
    let dir_has_data = dir.exists()
        && std::fs::read_dir(dir)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);

    #[derive(serde::Serialize, serde::Deserialize)]
    struct VersionedDirMarker {
        versioned: bool,
        retention: String,
    }

    if marker_path.exists() {
        let raw = std::fs::read_to_string(marker_path).map_err(|e| {
            anyhow::anyhow!("读取 versioned 目录标记失败 {}: {e}", marker_path.display())
        })?;
        let marker: VersionedDirMarker = serde_json::from_str(&raw).map_err(|e| {
            anyhow::anyhow!(
                "versioned 目录标记损坏 {}: {e}；请人工确认目录建库属性后修复/删除该标记",
                marker_path.display()
            )
        })?;
        if marker.versioned != versioned {
            anyhow::bail!(
                "数据目录 {} 的建库属性为 versioned={}，但当前配置要求 versioned={}。\
                 versioned 是 RocksDB 建库属性，不能原地切换；请新建数据目录并全量重灌\
                 （specs/022-versioned-pe-att-storage/quickstart.md），或恢复配置。",
                data_path,
                marker.versioned,
                versioned
            );
        }
        if marker.retention != retention {
            let content = serde_json::to_string_pretty(&VersionedDirMarker {
                versioned,
                retention: retention.to_string(),
            })?;
            std::fs::write(marker_path, content)?;
        }
        return Ok(());
    }

    if dir_has_data {
        log::warn!(
            "数据目录 {} 缺少 versioned 标记（存量目录），按当前配置 versioned={} retention={} 补写标记；\
             若与实际建库属性不符，SurrealDB 启动会报 comparator mismatch",
            data_path,
            versioned,
            retention
        );
    }
    if let Some(parent) = marker_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(&VersionedDirMarker {
        versioned,
        retention: retention.to_string(),
    })?;
    std::fs::write(marker_path, content)?;
    Ok(())
}

/// 模型生成结果写入后端。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ModelWriterMode {
    /// 写回 SurrealDB 模型表，保持当前默认行为。
    Surreal,
    /// 只消费生成端 batch 并输出统计，不持久化，用于压测生成吞吐。
    DrainOnly,
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
    /// 双写 SurrealDB + Parquet，用于对比。
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
            Self::Dual => "dual",
        }
    }

    pub fn writes_to_surreal(&self) -> bool {
        matches!(self, Self::Surreal | Self::Dual)
    }

    pub fn writes_to_parquet(&self) -> bool {
        matches!(self, Self::Parquet | Self::Dual)
    }
}

/// pe_transform 读取/对比后端。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransformReadBackend {
    Auto,
    Surreal,
    Parquet,
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
            Self::Rkyv => "rkyv",
            Self::Memory => "memory",
        }
    }

    pub fn needs_parquet_feature(&self) -> bool {
        matches!(self, Self::Parquet)
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
        Self::MemoryTasks
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

    /// 输出根目录，默认 output。项目产物写入 <output_root>/<project_name>。
    #[serde(default)]
    pub output_root: Option<String>,

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

    /// GenPipeline 同时进行的 Noun 级任务数量
    /// 默认为 None 时使用合理的并发数（如 CPU 核数）
    #[serde(default)]
    pub gen_pipeline_max_concurrent: Option<usize>,

    /// GenPipeline 单个 Noun 的 refno 列表按批次切分的大小
    /// 默认为 None 时复用 gen_model_batch_size
    #[serde(default)]
    pub gen_pipeline_batch_size: Option<usize>,

    /// GenPipeline 启用的 noun 类别列表
    /// 可选值: "cate", "loop", "prim" 或具体 noun 名称如 "BRAN", "PANE"
    /// 空 vec 表示启用所有类别（默认行为）
    #[serde(default)]
    pub gen_pipeline_enabled_target_types: Vec<String>,

    /// GenPipeline 禁用的 noun 列表
    /// 即使类别启用，这里的 noun 也会被过滤掉
    #[serde(default)]
    pub gen_pipeline_excluded_target_types: Vec<String>,

    /// 调试模式：限制每种 Noun 类型的处理数量
    /// 设置为 None 或 0 表示不限制，设置为具体数字则只处理前 N 个实例
    /// 用于快速测试和调试，避免处理全库数据
    #[serde(default)]
    pub gen_pipeline_debug_limit_per_target_type: Option<usize>,

    /// 模型生成空跑模式：仅收集 refno 并记录日志，不执行几何生成、DB 写入等
    /// 用于第一步调试分析（如检查 24381_145019 是否进入处理管道）
    #[serde(default)]
    pub gen_model_dry_run: bool,

    /// 生成的模型格式列表
    /// 默认为 [PdmsMesh]
    #[serde(default)]
    pub mesh_formats: Vec<MeshFormat>,

    /// 旧配置：是否启用 SurrealDB 进程。它不再表达模型生成输入后端。
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

    /// 模型写入后端：surreal 写库，drain-only 仅消费统计。
    #[serde(default = "default_model_writer_mode")]
    pub model_writer_mode: ModelWriterMode,

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

    /// 刷新前是否清理目标 dbnum 的历史 pe_transform。
    #[serde(default)]
    pub clear_transform_before_refresh: bool,

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

    /// 是否为本项目的 SurrealDB(RocksDB) 实例开启 MVCC 版本化存储（specs/022）。
    ///
    /// 开启后 PE/ATT 历史可通过 `SELECT ... VERSION $t` 时间旅行查询。
    /// 注意：versioned 是建库属性，已存在的非版本化数据目录不能直接以
    /// versioned=true 打开（会因 comparator 不匹配失败），必须新建数据目录并
    /// 重新解析灌库。因此默认 false，仅新站点/新数据目录显式开启。
    #[serde(default = "default_false")]
    pub versioned_storage: bool,

    /// 版本保留期，透传给启动参数 retention（fork 的 datastore_retention 语法，
    /// 如 "90d"/"30d"；"0" 表示无限保留——磁盘只增不减）。默认 `"0"`（全量历史）。
    #[serde(default = "default_version_retention")]
    pub version_retention: String,
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
    /// 获取 GenPipeline 实际并发数
    /// 如果未配置，返回 CPU 核数（最小为 2，最大为 8）
    pub fn get_gen_pipeline_concurrency(&self) -> usize {
        self.gen_pipeline_max_concurrent.unwrap_or_else(|| {
            let cpu_count = num_cpus::get();
            cpu_count.clamp(2, 8)
        })
    }

    /// 获取 GenPipeline 实际批次大小
    /// 如果未配置，复用 gen_model_batch_size
    pub fn get_gen_pipeline_batch_size(&self) -> usize {
        // 与 fast_model::gen_model::config::BatchSize::DEFAULT 保持一致；
        // 独立成常量以免瘦构建（无 gen_model）无法引用该模块。
        const GEN_PIPELINE_BATCH_SIZE_DEFAULT: usize = 100;
        self.gen_pipeline_batch_size
            .unwrap_or(self.inner.gen_model_batch_size)
            .max(GEN_PIPELINE_BATCH_SIZE_DEFAULT)
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
        self.gen_pipeline_enabled_target_types.is_empty()
            || self
                .gen_pipeline_enabled_target_types
                .iter()
                .any(|cat| cat == category || cat.to_lowercase() == category.to_lowercase())
    }

    /// 检查具体 noun 是否被排除
    pub fn is_noun_excluded(&self, noun: &str) -> bool {
        self.gen_pipeline_excluded_target_types
            .iter()
            .any(|excluded| excluded == noun || excluded.to_lowercase() == noun.to_lowercase())
    }

    /// 检查具体 noun 是否在启用的列表中（当使用具体 noun 名称时）
    pub fn is_noun_explicitly_enabled(&self, noun: &str) -> bool {
        // 如果启用了具体 noun 名称，则检查
        !self.gen_pipeline_enabled_target_types.is_empty()
            && (self.gen_pipeline_enabled_target_types.iter()
                .any(|cat| cat == noun || cat.to_lowercase() == noun.to_lowercase())
                // 也检查类别名称
                || self.is_noun_category_enabled(noun))
    }

    /// 获取输出根目录，默认 output。
    pub fn get_output_root(&self) -> std::path::PathBuf {
        self.output_root
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("output"))
    }

    /// 获取带 project_name 前缀的 output 基础目录
    ///
    /// - 如果 project_name 非空，返回 `<output_root>/{project_name}`
    /// - 如果 project_name 为空，panic 报错
    pub fn get_project_output_dir(&self) -> std::path::PathBuf {
        let project_name = &self.inner.project_name;
        if project_name.is_empty() {
            panic!("project_name 不能为空，请在配置文件中设置 project_name");
        }
        self.get_output_root().join(project_name)
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
        match self.model_writer_mode {
            ModelWriterMode::Surreal if !cfg!(feature = "write-to-surrealdb") => {
                anyhow::bail!(
                    "model_writer=surreal 需要编译 feature `write-to-surrealdb`；请使用 --features \"review\" 或显式加入 write-to-surrealdb"
                )
            }
            ModelWriterMode::DrainOnly if !cfg!(feature = "model-writer-drain") => {
                anyhow::bail!(
                    "model_writer=drain-only 需要编译 feature `model-writer-drain`；例如 --features \"review,model-writer-drain\""
                )
            }
            _ => Ok(()),
        }
    }

    /// specs/027（ADR-0008）：生成读取固定为 Surreal 主库；唯一前置是 SurrealDB 可用。
    pub fn validate_generation_read_features(&self) -> anyhow::Result<()> {
        if !self.use_surrealdb {
            anyhow::bail!("模型生成需要可用的 SurrealDB（use_surrealdb=true）");
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
                "pe_transform parquet 后端需要编译 feature `transform-store-parquet`；例如 --features \"review,transform-store-parquet\""
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
        let export_parquet_after_gen = option.export_parquet;
        Self {
            inner: option,
            export_instances: false,
            export_parquet_after_gen,
            output_root: None,
            trimesh_l0_dir: None,
            mqtt_server: None,
            mqtt_port: None,
            http_server: None,
            http_port: None,
            gen_pipeline_max_concurrent: None,
            gen_pipeline_batch_size: None,
            gen_pipeline_enabled_target_types: Vec::new(),
            gen_pipeline_excluded_target_types: Vec::new(),
            gen_pipeline_debug_limit_per_target_type: None,
            mesh_formats: vec![MeshFormat::PdmsMesh],
            use_surrealdb: true,
            model_cache_dir: None,
            defer_db_write: false,
            boolean_pipeline_mode: BooleanPipelineMode::MemoryTasks,
            model_writer_mode: ModelWriterMode::Surreal,
            transform_write_backend: TransformWriteBackend::Surreal,
            transform_read_backend: TransformReadBackend::Auto,
            transform_compare_backends: Vec::new(),
            transform_parquet_dir: None,
            clear_transform_before_refresh: false,
            enable_db_backfill: false,
            gen_model_dry_run: false,
            batch_channel_capacity: default_batch_channel_capacity(),
            base_write_concurrency: default_base_write_concurrency(),
            mesh_compute_concurrency: default_mesh_compute_concurrency(),
            inst_aabb_write_concurrency: default_inst_aabb_write_concurrency(),
            versioned_storage: false,
            version_retention: default_version_retention(),
        }
    }
}

/// 读取当前运行时配置中的 versioned 存储参数（specs/022）。
///
/// versioned_storage/version_retention 是 DbOptionExt 扩展字段，
/// `aios_core::get_db_option()` 拿不到；这里从 DB_OPTION_FILE 指向的 toml 提取。
///
/// fail-loud 语义（P4）：配置文件**不存在**视为合法的非 versioned 部署，
/// 返回关闭；文件**存在但读取/解析失败**必须报错——静默按关闭处理会把
/// versioned 库拼成非 versioned 连接串，启动时只会得到 comparator mismatch
/// 的晦涩报错，与真因（配置损坏）完全脱节。
pub fn current_versioned_params() -> anyhow::Result<(bool, String)> {
    let raw = std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".into());
    let config_path = raw.strip_suffix(".toml").unwrap_or(&raw).to_string();
    let config_file = format!("{}.toml", config_path);
    if !std::path::Path::new(&config_file).exists() {
        return Ok((false, default_version_retention()));
    }
    let ext = get_db_option_ext_from_path(&config_path).map_err(|e| {
        anyhow::anyhow!(
            "读取 versioned 存储参数失败（配置文件 {} 存在但不可用，拒绝按非 versioned 静默降级）: {e}",
            config_file
        )
    })?;
    Ok((ext.versioned_storage, ext.version_retention))
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
        ("full_noun_mode", "(已移除，GenPipeline 是默认管线)"),
        (
            "full_noun_max_concurrent_nouns",
            "gen_pipeline_max_concurrent",
        ),
        ("full_noun_batch_size", "gen_pipeline_batch_size"),
        (
            "full_noun_enabled_categories",
            "gen_pipeline_enabled_target_types",
        ),
        (
            "full_noun_excluded_nouns",
            "gen_pipeline_excluded_target_types",
        ),
        (
            "debug_limit_per_noun",
            "gen_pipeline_debug_limit_per_target_type",
        ),
        (
            "index_tree_max_concurrent_targets",
            "gen_pipeline_max_concurrent",
        ),
        ("index_tree_batch_size", "gen_pipeline_batch_size"),
        (
            "index_tree_enabled_target_types",
            "gen_pipeline_enabled_target_types",
        ),
        (
            "index_tree_excluded_target_types",
            "gen_pipeline_excluded_target_types",
        ),
        (
            "index_tree_debug_limit_per_target_type",
            "gen_pipeline_debug_limit_per_target_type",
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

    if std::env::var_os("AIOS_TREE_QUERY_SOURCE").is_some() {
        return Err(anyhow::anyhow!(
            "环境变量 AIOS_TREE_QUERY_SOURCE 已退役：层级查询仅支持 pe_owner / GenerationRead，请删除该变量后重试"
        ));
    }

    let gen_pipeline_max_concurrent = toml_value
        .get("gen_pipeline_max_concurrent")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    let gen_pipeline_batch_size = toml_value
        .get("gen_pipeline_batch_size")
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    // 解析启用的 noun 类别
    let gen_pipeline_enabled_target_types = toml_value
        .get("gen_pipeline_enabled_target_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 解析禁用的 noun 列表
    let gen_pipeline_excluded_target_types = toml_value
        .get("gen_pipeline_excluded_target_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 解析调试限制
    let gen_pipeline_debug_limit_per_target_type = toml_value
        .get("gen_pipeline_debug_limit_per_target_type")
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

    // use_surrealdb 只控制 SurrealDB 进程/副本可用性，不再代表模型生成输入源。
    let use_surrealdb = toml_value
        .get("use_surrealdb")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // specs/027（ADR-0007）：DuckLake 配置键分级检测（行为键硬错误、惰性键警告）。
    check_retired_ducklake_keys(&toml_value)?;
    // specs/027 FR-015 / T011：sync_history=true 硬错误、false 与 sync_versioned 告警。
    check_retired_history_keys(&toml_value)?;

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
        .unwrap_or(BooleanPipelineMode::MemoryTasks);

    let model_writer_mode = parse_model_writer_mode(
        toml_value
            .get("model_writer")
            .or_else(|| toml_value.get("model_writer_mode"))
            .and_then(|v| v.as_str()),
    )?;

    let transform_write_backend = parse_transform_write_backend(
        toml_value
            .get("transform_write_backend")
            .and_then(|v| v.as_str()),
    )?;

    let transform_read_backend = parse_transform_read_backend(
        toml_value
            .get("transform_read_backend")
            .and_then(|v| v.as_str()),
    )?;

    let transform_compare_backends =
        if let Some(value) = toml_value.get("transform_compare_backends") {
            if let Some(items) = value.as_array() {
                let mut parsed = Vec::new();
                for item in items.iter().filter_map(|item| item.as_str()) {
                    let backend = parse_transform_read_backend(Some(item))?;
                    if backend != TransformReadBackend::Auto {
                        parsed.push(backend);
                    }
                }
                parsed
            } else {
                parse_transform_compare_backends(value.as_str())?
            }
        } else {
            Vec::new()
        };

    let transform_parquet_dir = toml_value
        .get("transform_parquet_dir")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let clear_transform_before_refresh = toml_value
        .get("clear_transform_before_refresh")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

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
        .unwrap_or(db_option.export_parquet);

    let output_root = toml_value
        .get("output_root")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let versioned_storage = toml_value
        .get("versioned_storage")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let version_retention = toml_value
        .get("version_retention")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_version_retention);
    // specs/022/024（P7）：retention 非法值在 surreal 端会被静默吞掉，这里必须拦截。
    validate_version_retention(&version_retention)
        .map_err(|e| anyhow::anyhow!("配置文件 {} version_retention 非法: {}", config_file, e))?;

    // 构建 DbOptionExt
    let db_option_ext = DbOptionExt {
        inner: db_option,
        export_instances,
        export_parquet_after_gen,
        output_root,
        trimesh_l0_dir,
        mqtt_server: None,
        mqtt_port: None,
        http_server: None,
        http_port: None,
        gen_pipeline_max_concurrent,
        gen_pipeline_batch_size,
        gen_pipeline_enabled_target_types,
        gen_pipeline_excluded_target_types,
        gen_pipeline_debug_limit_per_target_type,
        mesh_formats,
        use_surrealdb,
        model_cache_dir,
        defer_db_write,
        boolean_pipeline_mode,
        model_writer_mode,
        transform_write_backend,
        transform_read_backend,
        transform_compare_backends,
        transform_parquet_dir,
        clear_transform_before_refresh,
        enable_db_backfill,
        gen_model_dry_run,
        batch_channel_capacity,
        base_write_concurrency,
        mesh_compute_concurrency,
        inst_aabb_write_concurrency,
        versioned_storage,
        version_retention,
    };

    validate_data_source_mode(db_option_ext.use_surrealdb)
        .map_err(|e| anyhow::anyhow!("配置文件 {} 数据源模式非法: {}", config_file, e))?;
    db_option_ext
        .validate_transform_store_features()
        .map_err(|e| {
            anyhow::anyhow!("配置文件 {} transform backend 配置非法: {}", config_file, e)
        })?;

    if std::env::var_os("AIOS_QUIET_CONFIG").is_none() {
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
        if !db_option_ext.gen_pipeline_enabled_target_types.is_empty() {
            println!(
                "   - 启用的 noun 类别: {:?}",
                db_option_ext.gen_pipeline_enabled_target_types
            );
        }
        if !db_option_ext.gen_pipeline_excluded_target_types.is_empty() {
            println!(
                "   - 排除的 noun: {:?}",
                db_option_ext.gen_pipeline_excluded_target_types
            );
        }
        println!(
            "   - boolean_pipeline_mode: {:?}",
            db_option_ext.boolean_pipeline_mode
        );
        if db_option_ext.enable_db_backfill {
            println!("   - enable_db_backfill: true");
        }
        if let Some(output_root) = db_option_ext.output_root.as_deref() {
            println!("   - output_root: {}", output_root);
        }
    }

    Ok(db_option_ext)
}

#[cfg(test)]
mod tests {
    use super::{BooleanPipelineMode, DbOptionExt, validate_data_source_mode};
    use aios_core::options::DbOption;
    use std::path::PathBuf;

    #[test]
    fn legacy_data_source_flag_no_longer_selects_generation_input() {
        assert!(validate_data_source_mode(true).is_ok());
        assert!(validate_data_source_mode(false).is_ok());
    }

    // specs/027（ADR-0007/0008）：generation_read_backend / parse_storage_backend
    // 两枚举已退役删除，对应的显式解析测试随之移除。

    #[test]
    fn versioned_generation_defaults_to_surreal_baseline_and_memory_tasks() {
        let options = DbOptionExt::from(DbOption::default());
        assert_eq!(
            options.boolean_pipeline_mode,
            BooleanPipelineMode::MemoryTasks
        );
        assert!(!options.enable_db_backfill);
    }

    #[test]
    fn project_output_dir_defaults_to_output_project_name() {
        let mut option = DbOption::default();
        option.project_name = "demo".to_string();
        let db_option_ext = DbOptionExt::from(option);

        assert_eq!(
            db_option_ext.get_project_output_dir(),
            PathBuf::from("output").join("demo")
        );
    }

    #[test]
    fn project_output_dir_uses_configured_output_root() {
        let mut option = DbOption::default();
        option.project_name = "demo".to_string();
        let mut db_option_ext = DbOptionExt::from(option);
        db_option_ext.output_root = Some("runtime/admin_sites/site-8080/output".to_string());

        assert_eq!(
            db_option_ext.get_project_output_dir(),
            PathBuf::from("runtime/admin_sites/site-8080/output").join("demo")
        );
    }
}
