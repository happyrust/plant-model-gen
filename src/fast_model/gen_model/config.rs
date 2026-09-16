use super::errors::{GenPipelineError, Result};
use crate::generation_read::hash_serializable;
use crate::options::{BooleanPipelineMode, DbOptionExt, MeshFormat};
use aios_core::options::DbOption;
use serde::Serialize;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;

const GENERATION_CONTRACT_SCHEMA_VERSION: u32 = 1;
const GEOMETRY_ALGORITHM_VERSION: &str = "plant-model-geometry/v1";

/// Result-affecting inputs for one generation run.
///
/// Runtime throughput controls and output destinations intentionally live in
/// [`ExecutionTuning`] and are excluded from this value and its hash.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct GenerationContract {
    schema_version: u32,
    geometry_algorithm_version: &'static str,
    enabled_nouns: Vec<String>,
    excluded_nouns: Vec<String>,
    debug_limit_per_target_type: Option<usize>,
    validate_sjus_map: bool,
    strict_validation: bool,
    mesh_enabled: bool,
    mesh_precision_hash: String,
    boolean_enabled: bool,
    boolean_mode: BooleanPipelineMode,
    boolean_db_backfill: bool,
    respect_tufl: bool,
    dry_run: bool,
    skip_inst_relate_aabb: bool,
    skip_final_aabb_sweep: bool,
}

impl GenerationContract {
    pub(crate) fn from_db_option(opt: &DbOptionExt, config: &GenPipelineConfig) -> Self {
        Self::from_db_option_with_integrity(
            opt,
            config,
            env_flag("AIOS_SKIP_INST_RELATE_AABB"),
            env_flag("AIOS_SKIP_FINAL_AABB_SWEEP"),
            env_flag("AIOS_RESPECT_TUFL"),
        )
    }

    fn from_db_option_with_integrity(
        opt: &DbOptionExt,
        config: &GenPipelineConfig,
        skip_inst_relate_aabb: bool,
        skip_final_aabb_sweep: bool,
        respect_tufl: bool,
    ) -> Self {
        Self {
            schema_version: GENERATION_CONTRACT_SCHEMA_VERSION,
            geometry_algorithm_version: GEOMETRY_ALGORITHM_VERSION,
            enabled_nouns: canonical_nouns(&config.enabled_categories),
            excluded_nouns: canonical_nouns(&config.excluded_nouns),
            debug_limit_per_target_type: config.gen_pipeline_debug_limit_per_target_type,
            validate_sjus_map: config.validate_sjus_map,
            strict_validation: config.strict_validation,
            mesh_enabled: opt.inner.gen_mesh,
            mesh_precision_hash: stable_hash_serializable(&opt.inner.mesh_precision),
            boolean_enabled: opt.inner.apply_boolean_operation,
            boolean_mode: opt.boolean_pipeline_mode.clone(),
            boolean_db_backfill: opt.enable_db_backfill,
            respect_tufl,
            dry_run: opt.gen_model_dry_run,
            skip_inst_relate_aabb,
            skip_final_aabb_sweep,
        }
    }

    pub(crate) fn contract_hash(&self) -> String {
        hash_serializable(self)
    }

    pub(crate) fn dry_run(&self) -> bool {
        self.dry_run
    }

    pub(crate) fn respect_tufl(&self) -> bool {
        self.respect_tufl
    }

    pub(crate) fn skip_inst_relate_aabb(&self) -> bool {
        self.skip_inst_relate_aabb
    }

    pub(crate) fn skip_final_aabb_sweep(&self) -> bool {
        self.skip_final_aabb_sweep
    }
}

/// Non-semantic runtime controls. Changes here may affect throughput or output
/// location, but must not change `GenerationContract::contract_hash`.
#[derive(Debug, Clone)]
pub(crate) struct ExecutionTuning {
    pub noun_concurrency: usize,
    pub noun_batch_size: usize,
    pub channel_capacity: usize,
    pub base_write_concurrency: usize,
    pub mesh_compute_concurrency: usize,
    pub inst_aabb_write_concurrency: usize,
    pub read_backend: String,
    pub writer_backend: String,
    pub output_root: Option<String>,
    pub export_formats: Vec<MeshFormat>,
    pub export_instances: bool,
    pub export_parquet_after_gen: bool,
    pub parquet_stream_writer_enabled: bool,
    pub perf_report_disabled: bool,
}

impl ExecutionTuning {
    pub(crate) fn from_db_option(opt: &DbOptionExt) -> Self {
        Self {
            noun_concurrency: opt.get_gen_pipeline_concurrency(),
            noun_batch_size: opt.get_gen_pipeline_batch_size(),
            channel_capacity: opt.get_batch_channel_capacity(),
            base_write_concurrency: opt.get_base_write_concurrency(),
            mesh_compute_concurrency: opt.get_mesh_compute_concurrency(),
            inst_aabb_write_concurrency: opt.get_inst_aabb_write_concurrency(),
            // specs/027（ADR-0008）：generation_read_backend 退役后统一 Surreal 主表直读。
            read_backend: "surreal".to_string(),
            writer_backend: opt.model_writer_mode.as_str().to_string(),
            output_root: opt.output_root.clone(),
            export_formats: opt.mesh_formats.clone(),
            export_instances: opt.export_instances,
            export_parquet_after_gen: opt.export_parquet_after_gen,
            parquet_stream_writer_enabled: env_flag("AIOS_ENABLE_PARQUET_STREAM_WRITER"),
            perf_report_disabled: env_flag("AIOS_DISABLE_PERF_REPORT"),
        }
    }
}

fn canonical_nouns(values: &[String]) -> Vec<String> {
    let mut values: Vec<_> = values
        .iter()
        .map(|value| value.trim().to_uppercase())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

fn stable_hash_serializable(value: &impl Serialize) -> String {
    let value = serde_json::to_value(value).expect("generation contract value must serialize");
    hash_serializable(&canonical_json(value))
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(values) => {
            let values = values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect::<BTreeMap<_, _>>();
            serde_json::to_value(values).expect("canonical generation contract object")
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        other => other,
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// 类型安全的并发配置
///
/// 保证并发数始终在有效范围内（MIN-MAX）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Concurrency(NonZeroUsize);

impl Concurrency {
    /// 最小并发数
    pub const MIN: usize = 2;

    /// 最大并发数。
    ///
    /// 历史上限为 8：高核数服务器即便显式配置更高的
    /// `gen_pipeline_max_concurrent` 也会被静默夹回，只留一条 log::warn。
    /// 现放宽为 64 作为防误配置的护栏（例如把 batch_size 填错到并发上）；
    /// 对 SurrealDB 的写入压力由 write pipeline 的
    /// base_write/mesh_compute/inst_aabb 三级 semaphore 单独限流，与此处无关。
    pub const MAX: usize = 64;

    /// 默认并发数
    pub const DEFAULT: usize = 4;

    /// 创建新的并发配置
    ///
    /// # Arguments
    /// * `n` - 并发数，会自动限制在 MIN-MAX 范围内
    ///
    /// # Errors
    /// * 如果 n 为 0，返回 InvalidConcurrency 错误
    ///
    /// # Examples
    /// ```
    /// let concurrency = Concurrency::new(6)?; // Ok(6)
    /// let concurrency = Concurrency::new(100)?; // Ok(MAX) - 自动限制
    /// let concurrency = Concurrency::new(0)?; // Err - 无效值
    /// ```
    pub fn new(n: usize) -> Result<Self> {
        if n == 0 {
            return Err(GenPipelineError::InvalidConcurrency(
                n,
                Self::MIN,
                Self::MAX,
            ));
        }

        let clamped = n.clamp(Self::MIN, Self::MAX);

        // 如果值被修正，发出警告
        if clamped != n {
            log::warn!(
                "并发数 {} 超出范围，已自动调整为 {}（范围：{}-{}）",
                n,
                clamped,
                Self::MIN,
                Self::MAX
            );
        }

        // SAFETY: clamped 范围是 [MIN, MAX]，MIN >= 2，所以不可能为 0
        Ok(Self(unsafe { NonZeroUsize::new_unchecked(clamped) }))
    }

    /// 创建默认并发配置
    pub fn default() -> Self {
        // SAFETY: DEFAULT = 4，不为 0
        Self(unsafe { NonZeroUsize::new_unchecked(Self::DEFAULT) })
    }

    /// 获取并发数值
    pub fn get(&self) -> usize {
        self.0.get()
    }
}

impl Default for Concurrency {
    fn default() -> Self {
        Self::default()
    }
}

/// 类型安全的批次大小配置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchSize(NonZeroUsize);

impl BatchSize {
    /// 最小批次大小
    pub const MIN: usize = 10;

    /// 最大批次大小
    pub const MAX: usize = 1000;

    /// 默认批次大小
    pub const DEFAULT: usize = 100;

    /// 创建新的批次大小配置
    pub fn new(n: usize) -> Result<Self> {
        if n == 0 {
            return Err(GenPipelineError::InvalidBatchSize(n));
        }

        let clamped = n.clamp(Self::MIN, Self::MAX);

        if clamped != n {
            log::warn!(
                "批次大小 {} 超出范围，已自动调整为 {}（范围：{}-{}）",
                n,
                clamped,
                Self::MIN,
                Self::MAX
            );
        }

        Ok(Self(unsafe { NonZeroUsize::new_unchecked(clamped) }))
    }

    /// 创建默认批次大小
    pub fn default() -> Self {
        Self(unsafe { NonZeroUsize::new_unchecked(Self::DEFAULT) })
    }

    /// 获取批次大小值
    pub fn get(&self) -> usize {
        self.0.get()
    }
}

impl Default for BatchSize {
    fn default() -> Self {
        Self::default()
    }
}

/// GenPipeline的统一配置
///
/// 封装所有 GenPipeline 相关配置，提供类型安全和验证
#[derive(Debug, Clone)]
pub struct GenPipelineConfig {
    /// 并发处理的 Noun 数量
    pub concurrency: Concurrency,

    /// 每批次处理的 refno 数量
    pub batch_size: BatchSize,

    /// 是否验证 SJUS map（建议启用）
    pub validate_sjus_map: bool,

    /// 是否在验证失败时严格报错（false 则只警告）
    pub strict_validation: bool,

    /// 启用的 noun 类别/名称列表，空表示启用所有
    pub enabled_categories: Vec<String>,

    /// 禁用的 noun 列表
    pub excluded_nouns: Vec<String>,

    /// 调试模式：限制每种 Noun 类型的处理数量（None 表示不限制）
    pub gen_pipeline_debug_limit_per_target_type: Option<usize>,
}

impl GenPipelineConfig {
    /// 从 DbOption 创建配置
    ///
    /// # Arguments
    /// * `opt` - 数据库配置选项
    ///
    /// # Errors
    /// * 如果并发数或批次大小无效
    ///
    /// 注意：由于 DbOption 在 aios-core 中可能没有 index_tree_* 字段，
    /// 这个函数用于兼容性。实际使用时建议使用 from_db_option_ext。
    pub fn from_db_option(_opt: &DbOption) -> Result<Self> {
        // 使用默认配置，因为标准 DbOption 可能没有这些字段
        Ok(Self::default())
    }

    /// 从 DbOptionExt 创建配置（推荐）
    ///
    /// DbOptionExt 在 src/options.rs 中定义，包含 GenPipeline 相关字段
    pub fn from_db_option_ext(opt: &crate::options::DbOptionExt) -> Result<Self> {
        let concurrency = Concurrency::new(opt.get_gen_pipeline_concurrency())?;
        let batch_size = BatchSize::new(opt.get_gen_pipeline_batch_size())?;

        Ok(Self {
            concurrency,
            batch_size,
            validate_sjus_map: true,  // 默认启用验证
            strict_validation: false, // 默认只警告，不报错
            enabled_categories: opt.gen_pipeline_enabled_target_types.clone(),
            excluded_nouns: opt.gen_pipeline_excluded_target_types.clone(),
            gen_pipeline_debug_limit_per_target_type: opt.gen_pipeline_debug_limit_per_target_type,
        })
    }

    /// 创建默认配置
    pub fn default() -> Self {
        Self {
            concurrency: Concurrency::default(),
            batch_size: BatchSize::default(),
            validate_sjus_map: true,
            strict_validation: false,
            enabled_categories: Vec::new(),
            excluded_nouns: Vec::new(),
            gen_pipeline_debug_limit_per_target_type: None,
        }
    }

    /// 构建器模式：设置并发数
    pub fn with_concurrency(mut self, concurrency: Concurrency) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// 构建器模式：设置批次大小
    pub fn with_batch_size(mut self, batch_size: BatchSize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// 构建器模式：设置严格验证
    pub fn with_strict_validation(mut self, strict: bool) -> Self {
        self.strict_validation = strict;
        self
    }

    /// 构建器模式：设置启用的类别
    pub fn with_enabled_categories(mut self, categories: Vec<String>) -> Self {
        self.enabled_categories = categories;
        self
    }

    /// 构建器模式：设置排除的 noun
    pub fn with_excluded_nouns(mut self, nouns: Vec<String>) -> Self {
        self.excluded_nouns = nouns;
        self
    }

    /// 检查 noun 类别是否启用
    pub fn is_category_enabled(&self, category: &str) -> bool {
        self.enabled_categories.is_empty()
            || self
                .enabled_categories
                .iter()
                .any(|cat| cat == category || cat.to_lowercase() == category.to_lowercase())
    }

    /// 检查具体 noun 是否被排除
    pub fn is_noun_excluded(&self, noun: &str) -> bool {
        self.excluded_nouns
            .iter()
            .any(|excluded| excluded == noun || excluded.to_lowercase() == noun.to_lowercase())
    }

    /// 检查具体 noun 是否应该处理
    /// 综合考虑类别启用和 noun 排除
    pub fn should_process_noun(&self, noun: &str, category: &str) -> bool {
        // 如果被明确排除，则不处理
        if self.is_noun_excluded(noun) {
            return false;
        }

        // 如果启用了具体 noun 名称，优先检查
        let has_explicit_nouns = self
            .enabled_categories
            .iter()
            .any(|cat| !matches!(cat.to_lowercase().as_str(), "cate" | "loop" | "prim"));

        if has_explicit_nouns {
            // 如果有具体的 noun 名称，则检查 noun 是否在列表中
            return self
                .enabled_categories
                .iter()
                .any(|cat| cat == noun || cat.to_lowercase() == noun.to_lowercase());
        }

        // 否则检查类别是否启用
        self.is_category_enabled(category)
    }

    /// 打印配置信息
    pub fn print_info(&self) {
        println!("╔════════════════════════════════════════╗");
        println!("║    GenPipeline 默认管线配置                ║");
        println!("╠════════════════════════════════════════╣");
        println!("║ 并发 Noun 数: {:<24} ║", self.concurrency.get());
        println!("║ 批次大小: {:<28} ║", self.batch_size.get());
        println!(
            "║ SJUS 验证: {:<27} ║",
            if self.validate_sjus_map {
                "✅ 启用"
            } else {
                "❌ 禁用"
            }
        );
        println!(
            "║ 严格模式: {:<28} ║",
            if self.strict_validation {
                "✅ 启用"
            } else {
                "❌ 禁用"
            }
        );

        if !self.enabled_categories.is_empty() {
            println!("╠════════════════════════════════════════╣");
            println!("║ 启用类别: {:<27} ║", self.enabled_categories.join(", "));
        }

        if !self.excluded_nouns.is_empty() {
            println!("╠════════════════════════════════════════╣");
            println!("║ 排除 Noun: {:<26} ║", self.excluded_nouns.join(", "));
        }

        if let Some(limit) = self.gen_pipeline_debug_limit_per_target_type {
            println!("╠════════════════════════════════════════╣");
            println!("║ 调试限制: 每个 Noun 最多 {:<8} 个实例 ║", limit);
        }

        println!("╚════════════════════════════════════════╝");
    }
}

impl Default for GenPipelineConfig {
    fn default() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_for(
        opt: &DbOptionExt,
        skip_inst_relate_aabb: bool,
        skip_final_aabb_sweep: bool,
    ) -> GenerationContract {
        let config = GenPipelineConfig::from_db_option_ext(opt).expect("config");
        GenerationContract::from_db_option_with_integrity(
            opt,
            &config,
            skip_inst_relate_aabb,
            skip_final_aabb_sweep,
            false,
        )
    }

    #[test]
    fn test_concurrency_valid_range() {
        let c1 = Concurrency::new(4).unwrap();
        assert_eq!(c1.get(), 4);

        let c2 = Concurrency::new(2).unwrap();
        assert_eq!(c2.get(), 2);

        let c3 = Concurrency::new(8).unwrap();
        assert_eq!(c3.get(), 8);
    }

    #[test]
    fn test_concurrency_clamping() {
        // 超出最大值
        let c1 = Concurrency::new(100).unwrap();
        assert_eq!(c1.get(), Concurrency::MAX);

        // 低于最小值但不为 0
        let c2 = Concurrency::new(1).unwrap();
        assert_eq!(c2.get(), Concurrency::MIN);
    }

    #[test]
    fn test_concurrency_zero_error() {
        let result = Concurrency::new(0);
        assert!(result.is_err());

        if let Err(GenPipelineError::InvalidConcurrency(val, min, max)) = result {
            assert_eq!(val, 0);
            assert_eq!(min, Concurrency::MIN);
            assert_eq!(max, Concurrency::MAX);
        } else {
            panic!("Expected InvalidConcurrency error");
        }
    }

    #[test]
    fn test_batch_size() {
        let b1 = BatchSize::new(100).unwrap();
        assert_eq!(b1.get(), 100);

        let b2 = BatchSize::new(0);
        assert!(b2.is_err());

        let b3 = BatchSize::new(5000).unwrap();
        assert_eq!(b3.get(), BatchSize::MAX);
    }

    #[test]
    fn test_config_builder() {
        let config = GenPipelineConfig::default()
            .with_concurrency(Concurrency::new(6).unwrap())
            .with_strict_validation(true);

        assert_eq!(config.concurrency.get(), 6);
        assert!(config.strict_validation);
    }

    #[test]
    fn contract_hash_normalizes_noun_case_order_and_duplicates() {
        let mut left = DbOptionExt::from(DbOption::default());
        left.gen_pipeline_enabled_target_types = vec!["bran".into(), "CATE".into(), "BRAN".into()];
        left.gen_pipeline_excluded_target_types = vec!["box".into(), "CYLI".into()];
        let mut right = DbOptionExt::from(DbOption::default());
        right.gen_pipeline_enabled_target_types = vec!["cate".into(), "BRAN".into()];
        right.gen_pipeline_excluded_target_types = vec!["cyli".into(), "BOX".into()];

        assert_eq!(
            contract_for(&left, false, false).contract_hash(),
            contract_for(&right, false, false).contract_hash()
        );
    }

    #[test]
    fn execution_tuning_does_not_change_contract_hash() {
        let left = DbOptionExt::from(DbOption::default());
        let mut right = left.clone();
        right.gen_pipeline_max_concurrent = Some(8);
        right.gen_pipeline_batch_size = Some(700);
        right.batch_channel_capacity = 31;
        right.base_write_concurrency = 7;
        right.mesh_compute_concurrency = 6;
        right.inst_aabb_write_concurrency = 5;
        right.model_writer_mode = crate::options::ModelWriterMode::DrainOnly;
        right.output_root = Some("somewhere-else".into());
        right.mesh_formats = vec![MeshFormat::Obj, MeshFormat::Glb];
        right.export_instances = true;
        right.export_parquet_after_gen = true;

        assert_eq!(
            contract_for(&left, false, false).contract_hash(),
            contract_for(&right, false, false).contract_hash()
        );
    }

    #[test]
    fn semantic_configuration_changes_contract_hash() {
        let base = DbOptionExt::from(DbOption::default());
        let base_hash = contract_for(&base, false, false).contract_hash();

        let mut noun = base.clone();
        noun.gen_pipeline_excluded_target_types.push("BOX".into());
        assert_ne!(base_hash, contract_for(&noun, false, false).contract_hash());

        let mut precision = base.clone();
        precision
            .inner
            .mesh_precision
            .non_scalable_geo_types
            .push("TEST".into());
        assert_ne!(
            base_hash,
            contract_for(&precision, false, false).contract_hash()
        );

        let mut boolean = base.clone();
        boolean.inner.apply_boolean_operation = !base.inner.apply_boolean_operation;
        assert_ne!(
            base_hash,
            contract_for(&boolean, false, false).contract_hash()
        );

        let mut dry_run = base.clone();
        dry_run.gen_model_dry_run = !base.gen_model_dry_run;
        assert_ne!(
            base_hash,
            contract_for(&dry_run, false, false).contract_hash()
        );

        assert_ne!(base_hash, contract_for(&base, true, false).contract_hash());
        assert_ne!(base_hash, contract_for(&base, false, true).contract_hash());
        let config = GenPipelineConfig::from_db_option_ext(&base).expect("config");
        assert_ne!(
            base_hash,
            GenerationContract::from_db_option_with_integrity(&base, &config, false, false, true,)
                .contract_hash()
        );
    }

    // #[test]
    // fn test_config_from_db_option() {
    //     let mut db_opt = DbOption::default();
    //     db_opt.gen_pipeline = true;
    //     db_opt.gen_pipeline_max_concurrent = 6;
    //     db_opt.gen_pipeline_batch_size = 200;

    //     let config = GenPipelineConfig::from_db_option(&db_opt).unwrap();

    //     assert!(config.enabled);
    //     assert_eq!(config.concurrency.get(), 6);
    //     assert_eq!(config.batch_size.get(), 200);
    // }
}
