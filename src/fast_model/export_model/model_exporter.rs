//! 模型导出器 Trait 定义和共享逻辑
//!
//! 本模块提供了统一的模型导出接口，支持多种格式（OBJ、XKT 等）。
//! 通过实现 `ModelExporter` Trait，可以轻松扩展到其他导出格式。

use aios_core::{GeomInstQuery, RefnoEnum};
use crate::fast_model::inst_query::query_insts_with_batch;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::fast_model::query_provider;
use crate::fast_model::unit_converter::{LengthUnit, UnitConverter};
use chrono;
use std::io::Write;

/// 模型导出器 Trait
///
/// 所有模型导出器都需要实现此 Trait，提供统一的导出接口。
#[async_trait::async_trait]
pub trait ModelExporter: Send + Sync {
    /// 导出配置类型
    type Config;

    /// 导出统计信息类型
    type Stats;

    /// 导出模型
    ///
    /// # 参数
    ///
    /// * `refnos` - 要导出的参考号列表
    /// * `mesh_dir` - mesh 文件目录
    /// * `output_path` - 输出文件路径
    /// * `config` - 导出配置
    ///
    /// # 返回值
    ///
    /// 返回导出统计信息
    async fn export(
        &self,
        refnos: &[RefnoEnum],
        mesh_dir: &Path,
        output_path: &str,
        config: Self::Config,
    ) -> Result<Self::Stats>;

    /// 获取支持的文件扩展名
    fn file_extension(&self) -> &str;

    /// 获取格式名称
    fn format_name(&self) -> &str;
}

/// 通用导出配置
#[derive(Debug, Clone)]
pub struct CommonExportConfig {
    /// 是否包含子孙节点
    pub include_descendants: bool,

    /// 可选的类型过滤器（如 ["EQUI", "PIPE"]）
    pub filter_nouns: Option<Vec<String>>,

    /// 是否输出详细日志
    pub verbose: bool,

    /// 单位转换器
    pub unit_converter: UnitConverter,

    /// 是否使用基础颜色材质（不使用 PBR）。为 true 时导出 KHR_materials_unlit
    pub use_basic_materials: bool,

    /// 是否包含负实体（Neg 类型几何体）
    /// 默认为 false，只导出正实体（Pos、Compound 等）
    pub include_negative: bool,

    /// 是否允许使用 SurrealDB 进行导出期查询（名称/几何实例等）。
    ///
    /// - true: 使用 SurrealDB 查询（旧路径，用于对照/验证）
    /// - false: 使用缓存数据源（不回退 SurrealDB）
    pub allow_surrealdb: bool,

    /// 当 `allow_surrealdb=false` 时使用的 foyer/instance_cache 目录。
    pub cache_dir: Option<PathBuf>,
}

impl Default for CommonExportConfig {
    fn default() -> Self {
        Self {
            include_descendants: true,
            filter_nouns: None,
            verbose: false,
            unit_converter: UnitConverter::default(),
            use_basic_materials: false,
            include_negative: false,
            allow_surrealdb: true,
            cache_dir: None,
        }
    }
}

impl CommonExportConfig {
    /// 创建带单位转换的配置
    pub fn with_unit_conversion(
        include_descendants: bool,
        filter_nouns: Option<Vec<String>>,
        verbose: bool,
        source_unit: LengthUnit,
        target_unit: LengthUnit,
    ) -> Self {
        Self {
            include_descendants,
            filter_nouns,
            verbose,
            unit_converter: UnitConverter::new(source_unit, target_unit),
            use_basic_materials: false,
            include_negative: false,
            allow_surrealdb: true,
            cache_dir: None,
        }
    }

    /// 创建带单位转换字符串的配置
    pub fn with_unit_conversion_str(
        include_descendants: bool,
        filter_nouns: Option<Vec<String>>,
        verbose: bool,
        source_unit: &str,
        target_unit: &str,
    ) -> Result<Self, String> {
        let source = LengthUnit::from_str(source_unit)?;
        let target = LengthUnit::from_str(target_unit)?;
        Ok(Self::with_unit_conversion(
            include_descendants,
            filter_nouns,
            verbose,
            source,
            target,
        ))
    }
}

/// OBJ 导出配置
#[derive(Debug, Clone)]
pub struct ObjExportConfig {
    /// 通用配置
    pub common: CommonExportConfig,
}

impl Default for ObjExportConfig {
    fn default() -> Self {
        Self {
            common: CommonExportConfig::default(),
        }
    }
}

/// GLB 导出配置
#[derive(Debug, Clone)]
pub struct GlbExportConfig {
    /// 通用配置
    pub common: CommonExportConfig,
}

impl Default for GlbExportConfig {
    fn default() -> Self {
        Self {
            common: CommonExportConfig::default(),
        }
    }
}

/// glTF 导出配置
#[derive(Debug, Clone)]
pub struct GltfExportConfig {
    /// 通用配置
    pub common: CommonExportConfig,
}

impl Default for GltfExportConfig {
    fn default() -> Self {
        Self {
            common: CommonExportConfig::default(),
        }
    }
}

/// XKT 导出配置
#[derive(Debug, Clone)]
pub struct XktExportConfig {
    /// 通用配置
    pub common: CommonExportConfig,

    /// 是否压缩 XKT 文件
    pub compress: bool,

    /// 生成完成后验证 XKT 文件
    pub validate: bool,

    /// 跳过 mesh 生成（默认会生成 mesh）
    pub skip_mesh: bool,

    /// 数据库配置文件路径
    pub db_config: Option<String>,

    /// 数据库编号（用于 mesh 生成）
    pub dbnum: Option<u32>,
}

impl Default for XktExportConfig {
    fn default() -> Self {
        Self {
            common: CommonExportConfig::default(),
            compress: true,
            validate: false,
            skip_mesh: false,
            db_config: None,
            dbnum: None,
        }
    }
}

/// 导出统计信息
#[derive(Debug, Default, Clone)]
pub struct ExportStats {
    /// 输入参考号数量
    pub refno_count: usize,

    /// 子孙节点数量
    pub descendant_count: usize,

    /// 几何体实例数量
    pub geometry_count: usize,

    /// 找到的 mesh 文件数量
    pub mesh_files_found: usize,

    /// 缺失的 mesh 文件数量
    pub mesh_files_missing: usize,

    /// 输出文件大小（字节）
    pub output_file_size: u64,

    /// 总耗时
    pub elapsed_time: std::time::Duration,

    /// glTF 节点数量（包括 root 节点）
    pub node_count: usize,

    /// glTF mesh 数量
    pub mesh_count: usize,
}

impl ExportStats {
    /// 创建新的统计信息
    pub fn new() -> Self {
        Self::default()
    }

    /// 打印统计摘要
    pub fn print_summary(&self, format_name: &str) {
        println!("\n📊 {} 导出统计:", format_name);
        println!("   - 总耗时: {:?}", self.elapsed_time);
        println!("   - 输入参考号: {}", self.refno_count);
        if self.descendant_count > 0 {
            println!("   - 子孙节点: {}", self.descendant_count);
        }
        println!("   - 几何体实例: {}", self.geometry_count);
        println!("   - Mesh 文件找到: {}", self.mesh_files_found);
        if self.mesh_files_missing > 0 {
            println!("   - ⚠️  Mesh 文件缺失: {}", self.mesh_files_missing);
        }
        if self.node_count > 0 {
            println!("   - glTF 节点数: {}", self.node_count);
        }
        if self.mesh_count > 0 {
            println!("   - glTF mesh 数: {}", self.mesh_count);
        }
        if self.output_file_size > 0 {
            println!(
                "   - 文件大小: {:.2} MB",
                self.output_file_size as f64 / 1024.0 / 1024.0
            );
        }
    }
}

/// 共享的辅助函数：收集所有需要导出的 refnos（包括子孙节点）
///
/// # 参数
///
/// * `input_refnos` - 输入的参考号列表
/// * `include_descendants` - 是否包含子孙节点
/// * `filter_nouns` - 可选的类型过滤器
/// * `verbose` - 是否输出详细日志
///
/// # 返回值
///
/// 返回所有需要导出的 refnos（包括子孙节点）
pub async fn collect_export_refnos(
    input_refnos: &[RefnoEnum],
    include_descendants: bool,
    filter_nouns: Option<&[String]>,
    verbose: bool,
) -> Result<Vec<RefnoEnum>> {
    if !include_descendants {
        if verbose {
            println!("📊 仅导出指定节点（不包含子孙）");
        }
        return Ok(input_refnos.to_vec());
    }

    if verbose {
        println!("🌳 收集子孙节点...");
    }

    // cache-only：层级查询严格走 TreeIndex（indextree），不允许导出阶段回退/自动生成/交互询问。
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::gen_model::tree_index_manager::{
        TreeIndexManager, disable_auto_generate_tree, load_index_with_large_stack,
    };
    use aios_core::tool::db_tool::db1_hash;
    use aios_core::tree_query::{TreeQueryFilter, TreeQueryOptions};
    use std::collections::{BTreeMap, HashSet};

    db_meta().ensure_loaded()?;

    // 强制关闭 “缺失则自动从 SurrealDB 生成 tree 文件” 的全局开关，保证导出语义确定。
    disable_auto_generate_tree();

    let tree_dir = TreeIndexManager::with_default_dir(Vec::new())
        .tree_dir()
        .to_path_buf();
    if !tree_dir.exists() {
        anyhow::bail!(
            "TreeIndex 目录不存在: {}\n\
             需要 cache-only 层级查询，请先生成 output/scene_tree/{{dbnum}}.tree 与 output/scene_tree/db_meta_info.json",
            tree_dir.display()
        );
    }

    let noun_hashes: Option<Vec<u32>> = filter_nouns
        .filter(|n| !n.is_empty())
        .map(|nouns| nouns.iter().map(|s| db1_hash(s.as_str())).collect());

    // roots 先按 dbnum 分组，避免跨库读取 tree。
    let mut by_dbnum: BTreeMap<u32, Vec<RefnoEnum>> = BTreeMap::new();
    let mut unresolved: Vec<RefnoEnum> = Vec::new();
    for &root in input_refnos {
        match db_meta().get_dbnum_by_refno(root) {
            Some(dbnum) => by_dbnum.entry(dbnum).or_default().push(root),
            None => unresolved.push(root),
        }
    }
    if !unresolved.is_empty() {
        anyhow::bail!(
            "无法从 db_meta_info.json 推导 dbnum（请先生成 output/scene_tree/db_meta_info.json）: {:?}",
            unresolved
        );
    }

    // 默认包含自身：roots 在前，后面拼接子孙；并保持顺序去重（避免 query 返回包含 roots 时重复）。
    let mut out: Vec<RefnoEnum> = Vec::with_capacity(input_refnos.len());
    let mut seen: HashSet<RefnoEnum> = HashSet::with_capacity(input_refnos.len());
    for &r in input_refnos {
        if seen.insert(r) {
            out.push(r);
        }
    }

    for (dbnum, roots) in by_dbnum {
        let tree_path = tree_dir.join(format!("{dbnum}.tree"));
        if !tree_path.exists() {
            anyhow::bail!(
                "缺少 TreeIndex 文件: {}\n\
                 cache-only 导出不允许自动生成/回退到 SurrealDB；请先生成该 .tree 文件。",
                tree_path.display()
            );
        }

        // 大栈线程加载，避免 Windows 反序列化大 `.tree` 文件触发栈溢出。
        let index = load_index_with_large_stack(&tree_dir, dbnum).with_context(|| {
            format!(
                "加载 TreeIndex 失败: {}",
                tree_path.display()
            )
        })?;

        let options = TreeQueryOptions {
            include_self: false,
            max_depth: None,
            filter: TreeQueryFilter {
                noun_hashes: noun_hashes.clone(),
                ..Default::default()
            },
        };

        for root in roots {
            for r in index.collect_descendants_bfs(root.refno(), &options) {
                let r = RefnoEnum::from(r);
                if r.is_valid() && seen.insert(r) {
                    out.push(r);
                }
            }
        }
    }

    if verbose {
        println!("   - 找到 {} 个节点（包括自己）", out.len());
    }

    Ok(out)
}

/// 共享的辅助函数：查询几何体实例数据
///
/// # 参数
///
/// * `refnos` - 参考号列表
/// * `enable_holes` - 是否启用布尔运算后的 mesh
/// * `verbose` - 是否输出详细日志
///
/// # 返回值
///
/// 返回几何体实例数据
pub async fn query_geometry_instances(
    refnos: &[RefnoEnum],
    enable_holes: bool,
    verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    query_geometry_instances_ext(refnos, enable_holes, false, verbose).await
}

/// 共享的辅助函数：查询几何体实例数据（支持负实体）
///
/// # 参数
///
/// * `refnos` - 参考号列表
/// * `enable_holes` - 是否启用布尔运算后的 mesh
/// * `include_negative` - 是否包含负实体（Neg 类型）
/// * `verbose` - 是否输出详细日志
///
/// # 返回值
///
/// 返回几何体实例数据
pub async fn query_geometry_instances_ext(
    refnos: &[RefnoEnum],
    enable_holes: bool,
    include_negative: bool,
    verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    let _ = include_negative;
    if refnos.is_empty() {
        if verbose {
            println!("⚠️  输入参考号为空，跳过查询");
        }
        return Ok(Vec::new());
    }

    if verbose {
        println!("📊 查询几何体数据...");
        println!("   - 参考号数量: {}", refnos.len());
    }

    const DEFAULT_QUERY_BATCH: usize = 50;
    let geom_insts = query_insts_with_batch(refnos, enable_holes, Some(DEFAULT_QUERY_BATCH))
        .await
        .context("查询 inst_relate 数据失败")?;

    if verbose {
        println!("   - 找到 {} 个几何体组", geom_insts.len());
        let total_instances: usize = geom_insts.iter().map(|g| g.insts.len()).sum();
        println!("   - 总几何体实例数: {}", total_instances);

        // 打印每个几何体组的详细信息
        for (idx, geom_inst) in geom_insts.iter().enumerate().take(5) {
            println!("\n   几何体组 [{}]:", idx + 1);
            println!("     - Refno: {:?}", geom_inst.refno);
            // println!("     - Noun: {}", geom_inst.noun);
            println!("     - 实例数: {}", geom_inst.insts.len());
            for (inst_idx, inst) in geom_inst.insts.iter().enumerate().take(3) {
                println!("       实例 [{}]: geo_hash={}", inst_idx + 1, inst.geo_hash);
            }
        }
        if geom_insts.len() > 5 {
            println!("   ... 还有 {} 个几何体组未显示", geom_insts.len() - 5);
        }
    }

    Ok(geom_insts)
}

/// 缓存路径：从 foyer/instance_cache 读取几何实例数据，构造与 SurrealDB `query_insts` 等价的 GeomInstQuery。
///
/// 约定：该函数**不回退** SurrealDB；若缓存缺失则直接返回错误。
///
/// 推荐：新代码优先从 `crate::fast_model::foyer_cache::query` 入口使用该能力（作为 cache-only 专区门面）。
pub async fn query_geometry_instances_ext_from_cache(
    refnos: &[RefnoEnum],
    cache_dir: &Path,
    enable_holes: bool,
    include_negative: bool,
    verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    crate::fast_model::foyer_cache::query::query_geometry_instances_ext_from_cache(
        refnos,
        cache_dir,
        enable_holes,
        include_negative,
        verbose,
    )
    .await
}
