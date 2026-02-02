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

    // Debug: 打印查询到的 geo_hash
    for geom_inst in &geom_insts {
        println!("[导出调试] refno={}, insts数量={}", 
            geom_inst.refno, geom_inst.insts.len());
        for inst in &geom_inst.insts {
            println!("  - geo_hash={}", inst.geo_hash);
        }
    }


    Ok(geom_insts)
}

/// 缓存路径：从 foyer/instance_cache 读取几何实例数据，构造与 SurrealDB `query_insts` 等价的 GeomInstQuery。
///
/// 约定：该函数**不回退** SurrealDB；若缓存缺失则直接返回错误。
pub async fn query_geometry_instances_ext_from_cache(
    refnos: &[RefnoEnum],
    cache_dir: &Path,
    enable_holes: bool,
    include_negative: bool,
    verbose: bool,
) -> Result<Vec<GeomInstQuery>> {
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::instance_cache::InstanceCacheManager;
    use aios_core::geometry::GeoBasicType;
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::rs_surreal::inst::ModelHashInst;
    use aios_core::types::PlantAabb;
    use aios_core::RefU64;
    use std::collections::{HashMap, HashSet};

    // cache-only：enable_holes=true 时，优先使用 instance_cache 中记录的 inst_relate_bool(Success)。
    // include_negative 暂仅保留签名一致性（缓存里仍会带 Neg/CateNeg 等记录，但导出默认不包含）。
    let _ = include_negative;

    if refnos.is_empty() {
        if verbose {
            println!("⚠️  输入参考号为空，跳过缓存查询");
        }
        return Ok(Vec::new());
    }

    db_meta().ensure_loaded()?;

    // 先按 dbnum 分组，避免跨库扫描 batch。
    //
    // 注意：缓存内的 key/refno 可能是 Refno 或 SesRef([refno,sesno]) 两种形式。
    // 为了与上层（room_calc / export）常用的 Refno 输入兼容，这里按 RefU64 归一化匹配。
    let mut by_dbnum: HashMap<u32, HashMap<RefU64, RefnoEnum>> = HashMap::new();
    let mut unresolved: Vec<RefnoEnum> = Vec::new();
    for &r in refnos {
        match db_meta().get_dbnum_by_refno(r) {
            Some(dbnum) => {
                by_dbnum.entry(dbnum).or_default().insert(r.refno(), r);
            }
            None => unresolved.push(r),
        }
    }
    if !unresolved.is_empty() {
        anyhow::bail!(
            "无法从 db_meta_info.json 推导 dbnum（请先生成 output/scene_tree/db_meta_info.json）: {:?}",
            unresolved
        );
    }

    if verbose {
        println!(
            "📦 缓存查询几何体数据: refnos={}, dbnums={}",
            refnos.len(),
            by_dbnum.len()
        );
        println!("   - 缓存目录: {}", cache_dir.display());
    }

    let cache = InstanceCacheManager::new(cache_dir).await?;

    // world_aabb：优先从 cache 的 inst_info_map 读取；若缺失则回退到 SQLite 空间索引（若启用）。
    #[cfg(feature = "sqlite-index")]
    let sqlite_idx = crate::spatial_index::SqliteSpatialIndex::with_default_path().ok();

    #[derive(Default)]
    struct Acc {
        owner: RefnoEnum,
        world_trans: PlantTransform,
        world_aabb: Option<PlantAabb>,
        has_neg: bool,
        has_cata_neg: bool,
        insts: Vec<ModelHashInst>,
    }

    let mut out: Vec<GeomInstQuery> = Vec::new();
    let mut missing: Vec<RefnoEnum> = Vec::new();
    let mut missing_cata_bool: Vec<RefnoEnum> = Vec::new();

    for (dbnum, want_map) in by_dbnum {
        let batch_ids = cache.list_batches(dbnum);
        if batch_ids.is_empty() {
            missing.extend(want_map.values().copied());
            continue;
        }

        // 先收集本 dbnum 下的 bool 成功结果（two-pass，保证 bool 覆盖原始 inst_geos）。
        //
        // 注意：instance_cache 可能存在多个 batch（多次 --regen-model 会追加 batch），
        // 若按旧到新遍历并“首次命中即使用”，会导致：
        // - 选中旧的 bool mesh_id（磁盘上可能已不存在） -> 导出表现为“某些子孙节点没导出来”
        // - 选中旧的 inst_geo transform（例如 RTOR scale 非 1）-> 尺寸被平方放大
        //
        // 因此这里按 created_at 选择“最新的 Success”。
        let mut bool_success: HashMap<RefU64, (String, i64)> = HashMap::new();
        if enable_holes {
            for batch_id in batch_ids.iter().rev() {
                let Some(batch) = cache.get(dbnum, batch_id).await else {
                    continue;
                };
                for (r, b) in batch.inst_relate_bool_map {
                    let k = r.refno();
                    if !want_map.contains_key(&k) {
                        continue;
                    }
                    if b.status != "Success" || b.mesh_id.is_empty() {
                        continue;
                    }
                    match bool_success.get(&k) {
                        None => {
                            bool_success.insert(k, (b.mesh_id, b.created_at));
                        }
                        Some((_, ts)) if b.created_at > *ts => {
                            bool_success.insert(k, (b.mesh_id, b.created_at));
                        }
                        _ => {}
                    }
                }
            }
        }

        let mut acc_map: HashMap<RefU64, Acc> = HashMap::new();
        // want_refno 中哪些是“元件库负实体（cata_neg）”目标：这类必须走布尔结果（CatePos）。
        // 取“最新 inst_info”给出的 has_cata_neg（避免旧 batch 的脏数据影响导出路径）。
        let mut want_has_cata_neg: HashSet<RefU64> = HashSet::new();
        // 只取最新 batch 的 inst_info/inst_geos/inst_tubi，避免多 batch 合并导致重复/错误缩放。
        let mut seen_meta: HashSet<RefU64> = HashSet::new();
        let mut seen_geos: HashSet<RefU64> = HashSet::new();
        let mut seen_tubi: HashSet<RefU64> = HashSet::new();
        // 某些 dbnum 会出现“新 batch 只有 inst_info（world_transform）但没有 inst_geos”的情况。
        // 若直接用最新 inst_info + 旧 inst_geos，会造成 world/local 不配套，典型表现为尺寸被平方放大。
        // 因此：一旦某 refno 选择了某个 batch 的 inst_geos，则 meta(world_trans/aabb/has_neg/has_cata_neg)
        // 必须优先对齐到同一 batch（若该 batch 有 inst_info）。
        let mut meta_locked_by_geos: HashSet<RefU64> = HashSet::new();
        // tubi 需要“每段自己的 world_transform(含长度 scale)”；它与同 refno 的 inst_info.world_transform
        // 可能不同（例如 refno 同时包含弯头构件与直段 tubing），因此不能强行复用 acc.world_trans。
        // 这里把 tubi 的 world_transform 作为“实例 transform”单独保存，导出侧用 identity world_trans 直接落地。
        let mut tubi_world_insts: HashMap<RefU64, Vec<(RefnoEnum, PlantTransform, Option<PlantAabb>, String)>> =
            HashMap::new();

        for batch_id in batch_ids.iter().rev() {
            let Some(batch) = cache.get(dbnum, batch_id).await else {
                continue;
            };

            // 先从 inst_info_map 组装“最新元数据”（owner/world_trans/aabb/has_neg/has_cata_neg）。
            // 这一步很关键：enable_holes=true 时，raw inst_geos 可能会被跳过，但导出 bool mesh 仍需要正确的世界变换。
            for (refno, info) in batch.inst_info_map.iter() {
                let k = refno.refno();
                if !want_map.contains_key(&k) {
                    continue;
                }
                if !seen_meta.insert(k) {
                    continue;
                }

                if info.has_cata_neg {
                    want_has_cata_neg.insert(k);
                }

                let entry = acc_map.entry(k).or_insert_with(Acc::default);
                let insts = std::mem::take(&mut entry.insts);

                let owner = if info.owner_refno.is_valid() {
                    info.owner_refno
                } else {
                    *refno
                };
                let mut world_aabb: Option<PlantAabb> = info.aabb.map(Into::into);
                #[cfg(feature = "sqlite-index")]
                {
                    if world_aabb.is_none() {
                        if let Some(idx) = sqlite_idx.as_ref() {
                            let id: aios_core::RefU64 = (*refno).into();
                            if let Ok(Some(aabb)) = idx.get_aabb(id) {
                                world_aabb = Some(aabb.into());
                            }
                        }
                    }
                }
                let has_neg = batch
                    .neg_relate_map
                    .get(refno)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                *entry = Acc {
                    owner,
                    world_trans: PlantTransform::from(info.world_transform),
                    world_aabb,
                    has_neg,
                    has_cata_neg: info.has_cata_neg,
                    insts,
                };
            }

            // tubing 节点在 cache 中以 inst_tubi_map(EleGeosInfo) 形式存在：
            // - 通常不会出现在 inst_geos_map（否则会被当作普通构件几何）
            // - 导出/房间计算期需要把它们拼成一条“带 is_tubi=true 的几何实例”
            //
            // 注意：这里用 world_trans + local(identity) 表达 tubing 的世界变换，
            // 以复用导出侧统一的 world_trans * geo_transform 逻辑。
            {
                use aios_core::prim_geo::basic::TUBI_GEO_HASH;
                for (refno, info) in batch.inst_tubi_map.iter() {
                    let k = refno.refno();
                    if !want_map.contains_key(&k) {
                        continue;
                    }
                    if !seen_tubi.insert(k) {
                        continue;
                    }

                    // 记录该 tubi 段的独立 world_transform（通常包含沿轴向的长度 scale）。
                    let owner = if info.owner_refno.is_valid() {
                        info.owner_refno
                    } else {
                        *refno
                    };
                    let geo_hash = info
                        .cata_hash
                        .clone()
                        .unwrap_or_else(|| TUBI_GEO_HASH.to_string());
                    tubi_world_insts
                        .entry(k)
                        .or_default()
                        .push((owner, PlantTransform::from(info.world_transform), info.aabb.map(Into::into), geo_hash));

                    let entry = acc_map.entry(k).or_insert_with(|| Acc {
                        owner: *refno,
                        world_trans: PlantTransform::default(),
                        world_aabb: None,
                        has_neg: false,
                        has_cata_neg: false,
                        insts: Vec::new(),
                    });

                    entry.owner = if info.owner_refno.is_valid() {
                        info.owner_refno
                    } else {
                        *refno
                    };
                    // tubing 的 EleGeosInfo 也带 world_transform/aabb，可作为 inst_info 缺失时的 fallback。
                    // 若已命中 inst_info 的 meta，则不在此处覆写，避免把 has_neg/has_cata_neg 等信息误清零。
                    if !meta_locked_by_geos.contains(&k) && !seen_meta.contains(&k) {
                        seen_meta.insert(k);
                        entry.world_trans = PlantTransform::from(info.world_transform);
                        entry.world_aabb = info.aabb.map(Into::into);
                    }
                }
            }

            // 遍历 inst_info_map，使用 get_inst_key() 查找对应的几何数据。
            // 这样即使多个 refno 共享相同的 cata_hash，也能为每个 refno 获取几何数据。
            for (refno, info) in batch.inst_info_map.iter() {
                let refno_u64 = refno.refno();
                if !want_map.contains_key(&refno_u64) {
                    continue;
                }

                // 使用 info.get_inst_key() 查找对应的几何数据
                let inst_key = info.get_inst_key();
                let geos_data = match batch.inst_geos_map.get(&inst_key) {
                    Some(data) => data,
                    None => continue, // 没有几何数据，跳过
                };

                let entry = acc_map.entry(refno_u64).or_insert_with(|| {
                    let owner = if info.owner_refno.is_valid() {
                        info.owner_refno
                    } else {
                        *refno
                    };
                    let mut world_aabb: Option<PlantAabb> = info.aabb.map(Into::into);
                    #[cfg(feature = "sqlite-index")]
                    {
                        if world_aabb.is_none() {
                            if let Some(idx) = sqlite_idx.as_ref() {
                                let id: aios_core::RefU64 = (*refno).into();
                                if let Ok(Some(aabb)) = idx.get_aabb(id) {
                                    world_aabb = Some(aabb.into());
                                }
                            }
                        }
                    }
                    let has_neg = batch
                        .neg_relate_map
                        .get(refno)
                        .map(|v| !v.is_empty())
                        .unwrap_or(false);
                    Acc {
                        owner,
                        world_trans: PlantTransform::from(info.world_transform),
                        world_aabb,
                        has_neg,
                        has_cata_neg: info.has_cata_neg,
                        insts: Vec::new(),
                    }
                });

                // enable_holes=true 且已有 booled mesh：保留 owner/world_trans/has_neg，但不再收集原始 inst_geos。
                if enable_holes && bool_success.contains_key(&refno_u64) {
                    // 有 bool mesh 时，不应再让旧 batch 的 raw inst_geos 混入（否则可能出现重复/错误缩放）。
                    seen_geos.insert(refno_u64);
                    continue;
                }
                // 只取最新 batch 的 raw inst_geos（避免多 batch 合并导致重复/旧数据污染）。
                if !seen_geos.insert(refno_u64) {
                    continue;
                }

                // 标记本 refno 的几何数据已锁定到当前 batch
                meta_locked_by_geos.insert(refno_u64);

                for inst in &geos_data.insts {
                    if !inst.visible {
                        continue;
                    }
                    match inst.geo_type {
                        GeoBasicType::Pos
                        | GeoBasicType::DesiPos
                        | GeoBasicType::CatePos
                        | GeoBasicType::Compound => {}
                        _ => continue,
                    }

                    entry.insts.push(ModelHashInst {
                        geo_hash: inst.geo_hash.to_string(),
                        geo_transform: PlantTransform::from(inst.geo_transform),
                        is_tubi: inst.is_tubi,
                        unit_flag: inst.geo_param.is_reuse_unit(),
                    });
                }
            }
        }

        // 对每个想要的 refno（以 RefU64 归一化）组装输出；refno 字段使用调用方输入，避免泄露 SesRef 形式。
        for (want_u64, want_refno) in want_map {
            if enable_holes {
                if let Some((mesh_id, _)) = bool_success.get(&want_u64) {
                    // bool mesh 在 cache bool_worker 中已写盘到 lod_{default}/ 目录；
                    // 其坐标系约定为 refno local space。
                    //
                    // 注意：导出侧（export_obj 等）对 has_neg=true 的约定是：
                    // inst.geo_transform 已经是 world_trans.d（等价 SurrealDB booled_id 查询返回值）。
                    // 若此处使用 identity，则导出时会丢失世界变换，常见表现为子节点（布尔结果 mesh）方位/位置不对。
                    let acc = acc_map.remove(&want_u64).unwrap_or(Acc {
                        owner: want_refno,
                        world_trans: PlantTransform::default(),
                        world_aabb: None,
                        has_neg: true,
                        has_cata_neg: false,
                        insts: Vec::new(),
                    });
                    out.push(GeomInstQuery {
                        refno: want_refno,
                        owner: acc.owner,
                        world_aabb: acc.world_aabb,
                        world_trans: acc.world_trans,
                        insts: vec![ModelHashInst {
                            geo_hash: mesh_id.clone(),
                            geo_transform: acc.world_trans,
                            is_tubi: false,
                            unit_flag: false,
                        }],
                        has_neg: true,
                    });
                    continue;
                }

                // 元件库 cata_neg：必须导出布尔结果（CatePos）。缺失时给出明确错误（不要伪装成“缓存缺失/跳过”）。
                if want_has_cata_neg.contains(&want_u64) {
                    missing_cata_bool.push(want_refno);
                    // 清理 acc_map 中的残留条目，避免后续误用/误报。
                    let _ = acc_map.remove(&want_u64);
                    continue;
                }
            }

            match acc_map.remove(&want_u64) {
                Some(acc) if !acc.insts.is_empty() => out.push(GeomInstQuery {
                    refno: want_refno,
                    owner: acc.owner,
                    world_aabb: acc.world_aabb,
                    world_trans: acc.world_trans,
                    insts: acc.insts,
                    has_neg: acc.has_neg,
                }),
                _ => missing.push(want_refno),
            }

            // 追加 tubing world 实例：用 identity world_trans，使导出端 world_trans * inst.geo_transform == inst.geo_transform。
            if let Some(items) = tubi_world_insts.remove(&want_u64) {
                for (owner, wt, aabb, geo_hash) in items {
                    out.push(GeomInstQuery {
                        refno: want_refno,
                        owner,
                        world_aabb: aabb,
                        world_trans: PlantTransform::default(),
                        insts: vec![ModelHashInst {
                            geo_hash,
                            geo_transform: wt,
                            is_tubi: true,
                            unit_flag: false,
                        }],
                        has_neg: false,
                    });
                }
            }
        }
    }

    if !missing_cata_bool.is_empty() {
        missing_cata_bool.sort();
        missing_cata_bool.dedup();
        anyhow::bail!(
            "以下 refno 存在元件库负实体(cata_neg)，但未找到布尔结果缓存(inst_relate_bool=Success)，无法导出 CatePos：{:?}\n\
             处理建议：\n\
             - 确认本次运行启用了布尔运算（apply_boolean_operation=true / 或命令行 --regen-model 已自动开启）\n\
             - 确认缓存布尔 worker 已执行且成功写入 instance_cache 的 inst_relate_bool_map\n\
             - 确认对应 LOD 目录下存在 booled GLB（例如 assets/meshes/lod_L1/<refno>_L1.glb）",
            missing_cata_bool
        );
    }

    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        if verbose {
            println!(
                "⚠️  缓存中未找到以下 refno 的几何实例数据（可能是无几何节点/仅 tubing/或尚未生成），将跳过：{:?}",
                missing
            );
        }
    }

    if verbose {
        println!("✅ 缓存查询几何体数据完成: {} 个几何体组", out.len());
    }
    Ok(out)
}
