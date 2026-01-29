//! 模型导出器 Trait 定义和共享逻辑
//!
//! 本模块提供了统一的模型导出接口，支持多种格式（OBJ、XKT 等）。
//! 通过实现 `ModelExporter` Trait，可以轻松扩展到其他导出格式。

use aios_core::{GeomInstQuery, RefnoEnum, query_insts_with_batch};
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

    // 如果有过滤条件，使用过滤查询；否则查询所有子孙节点
    let descendants = if let Some(nouns) = filter_nouns {
        let nouns_slice: Vec<&str> = nouns.iter().map(|s| s.as_str()).collect();
        query_provider::query_multi_descendants(input_refnos, &nouns_slice)
            .await
            .context("查询子孙节点失败")?
    } else {
        // 查询所有子孙节点（传入空数组表示不过滤类型）
        query_provider::query_multi_descendants(input_refnos, &[])
            .await
            .context("查询子孙节点失败")?
    };

    // 默认包含自身：roots 在前，后面拼接子孙；并保持顺序去重（避免 query 返回包含 roots 时重复）。
    let mut out: Vec<RefnoEnum> = Vec::with_capacity(input_refnos.len() + descendants.len());
    let mut seen: std::collections::HashSet<RefnoEnum> =
        std::collections::HashSet::with_capacity(input_refnos.len() + descendants.len());
    for &r in input_refnos {
        if seen.insert(r) {
            out.push(r);
        }
    }
    for r in descendants {
        if seen.insert(r) {
            out.push(r);
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
    let geom_insts = aios_core::query_insts_with_batch(refnos, enable_holes, Some(DEFAULT_QUERY_BATCH))
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
    use std::collections::HashMap;

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
        insts: Vec<ModelHashInst>,
    }

    let mut out: Vec<GeomInstQuery> = Vec::new();
    let mut missing: Vec<RefnoEnum> = Vec::new();

    for (dbnum, want_map) in by_dbnum {
        let batch_ids = cache.list_batches(dbnum);
        if batch_ids.is_empty() {
            missing.extend(want_map.values().copied());
            continue;
        }

        // 先收集本 dbnum 下的 bool 成功结果（two-pass，保证 bool 覆盖原始 inst_geos）。
        let mut bool_success: HashMap<RefU64, String> = HashMap::new();
        if enable_holes {
            for batch_id in &batch_ids {
                let Some(batch) = cache.get(dbnum, batch_id).await else {
                    continue;
                };
                for (r, b) in batch.inst_relate_bool_map {
                    let k = r.refno();
                    if !want_map.contains_key(&k) {
                        continue;
                    }
                    if b.status == "Success" && !b.mesh_id.is_empty() {
                        bool_success.entry(k).or_insert(b.mesh_id);
                    }
                }
            }
        }

        let mut acc_map: HashMap<RefU64, Acc> = HashMap::new();

        for batch_id in batch_ids {
            let Some(batch) = cache.get(dbnum, &batch_id).await else {
                continue;
            };

            // 逐个 inst_key 扫描并按 refno 聚合；只处理本次需要的 refno 集合。
            for geos_data in batch.inst_geos_map.values() {
                let refno = geos_data.refno;
                let refno_u64 = refno.refno();
                if !want_map.contains_key(&refno_u64) {
                    continue;
                }

                let entry = acc_map.entry(refno_u64).or_insert_with(|| {
                    if let Some(info) = batch.inst_info_map.get(&refno) {
                        let owner = if info.owner_refno.is_valid() {
                            info.owner_refno
                        } else {
                            refno
                        };
                        let mut world_aabb: Option<PlantAabb> = info.aabb.map(Into::into);
                        #[cfg(feature = "sqlite-index")]
                        {
                            if world_aabb.is_none() {
                                if let Some(idx) = sqlite_idx.as_ref() {
                                    let id: aios_core::RefU64 = refno.into();
                                    if let Ok(Some(aabb)) = idx.get_aabb(id) {
                                        world_aabb = Some(aabb.into());
                                    }
                                }
                            }
                        }
                        let has_neg = batch
                            .neg_relate_map
                            .get(&refno)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false);
                        Acc {
                            owner,
                            world_trans: PlantTransform::from(info.world_transform),
                            world_aabb,
                            has_neg,
                            insts: Vec::new(),
                        }
                    } else {
                        #[cfg(feature = "sqlite-index")]
                        let world_aabb = sqlite_idx
                            .as_ref()
                            .and_then(|idx| idx.get_aabb(refno.into()).ok().flatten())
                            .map(Into::into);
                        #[cfg(not(feature = "sqlite-index"))]
                        let world_aabb = None;
                        Acc {
                            owner: refno,
                            world_trans: PlantTransform::default(),
                            world_aabb,
                            has_neg: false,
                            insts: Vec::new(),
                        }
                    }
                });

                // enable_holes=true 且已有 booled mesh：保留 owner/world_trans/has_neg，但不再收集原始 inst_geos。
                if enable_holes && bool_success.contains_key(&refno_u64) {
                    continue;
                }

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
                        transform: PlantTransform::from(inst.transform),
                        is_tubi: inst.is_tubi,
                        unit_flag: inst.unit_flag,
                    });
                }
            }
        }

        // 对每个想要的 refno（以 RefU64 归一化）组装输出；refno 字段使用调用方输入，避免泄露 SesRef 形式。
        for (want_u64, want_refno) in want_map {
            if enable_holes {
                if let Some(mesh_id) = bool_success.get(&want_u64) {
                    // bool mesh 在 cache bool_worker 中已写盘到 lod_{default}/ 目录；
                    // 其坐标系约定为 refno local space。
                    //
                    // 注意：导出侧（export_obj 等）对 has_neg=true 的约定是：
                    // inst.transform 已经是 world_trans.d（等价 SurrealDB booled_id 查询返回值）。
                    // 若此处使用 identity，则导出时会丢失世界变换，常见表现为子节点（布尔结果 mesh）方位/位置不对。
                    let acc = acc_map.remove(&want_u64).unwrap_or(Acc {
                        owner: want_refno,
                        world_trans: PlantTransform::default(),
                        world_aabb: None,
                        has_neg: true,
                        insts: Vec::new(),
                    });
                    out.push(GeomInstQuery {
                        refno: want_refno,
                        owner: acc.owner,
                        world_aabb: acc.world_aabb,
                        world_trans: acc.world_trans,
                        insts: vec![ModelHashInst {
                            geo_hash: mesh_id.clone(),
                            transform: acc.world_trans,
                            is_tubi: false,
                            unit_flag: false,
                        }],
                        has_neg: true,
                    });
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
        }
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
