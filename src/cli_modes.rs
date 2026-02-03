use std::path::{Path, PathBuf};
use std::str::FromStr;

use aios_core::pdms_types::{RefU64, RefnoEnum};
use anyhow::{Context, Result, anyhow};

use aios_core::init_surreal;
use aios_core::{DBType, query_mdb_db_nums};
use aios_database::fast_model::export_glb::GlbExporter;
use aios_database::fast_model::export_gltf::GltfExporter;
use aios_database::fast_model::export_gltf::export_gltf_for_refnos;
use aios_database::fast_model::export_instanced_bundle::export_instanced_bundle_for_refnos;
use aios_database::fast_model::export_model::export_obj::ObjExporter;
use aios_database::options::DbOptionExt;
// use aios_database::fast_model::export_xkt::XktExporter;
use aios_database::fast_model::model_exporter::{
    CommonExportConfig, GlbExportConfig, GltfExportConfig, ModelExporter, ObjExportConfig,
    XktExportConfig, collect_export_refnos,
};
use aios_database::fast_model::unit_converter::{LengthUnit, UnitConverter};

/// 统一的导出配置结构体
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// 参考号列表
    pub refnos_str: Vec<String>,
    /// 输出路径（可选）
    pub output_path: Option<String>,
    /// 过滤类型（可选）
    pub filter_nouns: Option<Vec<String>>,
    /// 是否包含子孙节点
    pub include_descendants: bool,
    /// 源单位
    pub source_unit: String,
    /// 目标单位
    pub target_unit: String,
    /// 是否详细输出
    pub verbose: bool,
    /// 是否重新生成 plant mesh
    pub regenerate_plant_mesh: bool,
    /// 数据库编号（用于按 SITE 导出）
    pub dbnum: Option<u32>,
    /// 是否使用基础颜色材质（非 PBR）
    pub use_basic_materials: bool,
    /// 是否运行所有 dbnum（全库导出模式）
    pub run_all_dbnos: bool,
    /// 是否按 SITE 拆分导出
    pub split_by_site: bool,
    /// 是否包含负实体（Neg 类型几何体）
    pub include_negative: bool,
    /// 是否导出 SVG 截面
    pub export_svg: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            refnos_str: Vec::new(),
            output_path: None,
            filter_nouns: None,
            include_descendants: true,
            source_unit: "mm".to_string(),
            target_unit: "mm".to_string(),
            verbose: false,
            regenerate_plant_mesh: false,
            dbnum: None,
            use_basic_materials: false,
            run_all_dbnos: false,
            split_by_site: false,
            include_negative: false,
            export_svg: false,
        }
    }
}

impl ExportConfig {
    /// 创建新的导出配置
    pub fn new(refnos_str: Vec<String>) -> Self {
        Self {
            refnos_str,
            export_svg: false,
            ..Default::default()
        }
    }

    /// 设置输出路径
    pub fn with_output_path(mut self, output_path: Option<String>) -> Self {
        self.output_path = output_path;
        self
    }

    /// 设置过滤类型
    pub fn with_filter_nouns(mut self, filter_nouns: Option<Vec<String>>) -> Self {
        self.filter_nouns = filter_nouns;
        self
    }

    /// 设置是否包含子孙节点
    pub fn with_include_descendants(mut self, include_descendants: bool) -> Self {
        self.include_descendants = include_descendants;
        self
    }

    /// 设置单位转换
    pub fn with_unit_conversion(mut self, source_unit: &str, target_unit: &str) -> Self {
        self.source_unit = source_unit.to_string();
        self.target_unit = target_unit.to_string();
        self
    }

    /// 设置详细输出
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// 设置重新生成 plant mesh
    pub fn with_regenerate_plant_mesh(mut self, regenerate_plant_mesh: bool) -> Self {
        self.regenerate_plant_mesh = regenerate_plant_mesh;
        self
    }

    /// 设置是否默认跑全库
    pub fn with_run_all_dbnos(mut self, run_all_dbnos: bool) -> Self {
        self.run_all_dbnos = run_all_dbnos;
        self
    }

    /// 设置数据库编号
    pub fn with_dbno(mut self, dbnum: Option<u32>) -> Self {
        self.dbnum = dbnum;
        self
    }

    /// 设置是否按 SITE 拆分导出
    pub fn with_split_by_site(mut self, split_by_site: bool) -> Self {
        self.split_by_site = split_by_site;
        self
    }

    /// 从命令行参数构建导出配置（用于全库导出模式）
    pub fn build_for_all_dbnos(
        output_path: Option<String>,
        filter_nouns: Option<Vec<String>>,
        include_descendants: bool,
        source_unit: String,
        target_unit: String,
        verbose: bool,
        regenerate_plant_mesh: bool,
        use_basic_materials: bool,
        split_by_site: bool,
        include_negative: bool,
        export_svg: bool,
    ) -> Self {
        Self {
            refnos_str: vec![],
            output_path,
            filter_nouns,
            include_descendants,
            source_unit,
            target_unit,
            verbose,
            regenerate_plant_mesh,
            dbnum: None,
            use_basic_materials,
            run_all_dbnos: true, // 关键：全库导出
            split_by_site,
            include_negative,
            export_svg,
        }
    }

    /// 从命令行参数构建 XKT 导出配置（用于全库导出模式）
    pub fn build_xkt_for_all_dbnos(
        output_path: Option<String>,
        filter_nouns: Option<Vec<String>>,
        include_descendants: bool,
        source_unit: String,
        target_unit: String,
        verbose: bool,
        regenerate_plant_mesh: bool,
        compress: bool,
        validate: bool,
        skip_mesh: bool,
        db_config: Option<String>,
        split_by_site: bool,
    ) -> Self {
        Self {
            refnos_str: vec![],
            output_path,
            filter_nouns,
            include_descendants,
            source_unit,
            target_unit,
            verbose,
            regenerate_plant_mesh,
            dbnum: None,
            use_basic_materials: false,
            run_all_dbnos: true, // 关键：全库导出
            split_by_site,
            include_negative: false,
            export_svg: false,
        }
    }

    /// 解析参考号
    pub fn parse_refnos(&self) -> Result<Vec<RefnoEnum>> {
        let mut refnos = Vec::new();
        for s in &self.refnos_str {
            let normalized = s.replace('_', "/");
            if let Ok(ref_u64) = RefU64::from_str(&normalized) {
                refnos.push(RefnoEnum::Refno(ref_u64));
            }
        }

        if refnos.is_empty() {
            return Err(anyhow!("无效的参考号"));
        }

        Ok(refnos)
    }

    /// 获取 mesh 目录（自动根据 default_lod 添加 LOD 子目录）
    pub fn get_mesh_dir(&self, db_option_ext: &DbOptionExt) -> PathBuf {
        let base_dir = if let Some(ref path) = db_option_ext.inner.meshes_path {
            PathBuf::from(path)
        } else {
            PathBuf::from("assets/meshes")
        };

        // 根据 default_lod 自动添加 LOD 子目录
        let lod = db_option_ext.inner.mesh_precision.default_lod;
        let lod_dir = base_dir.join(format!("lod_{:?}", lod));

        println!(
            "📂 使用 LOD 目录: {} (LOD 级别: {:?})",
            lod_dir.display(),
            lod
        );

        lod_dir
    }

    /// 打印导出参数
    pub fn print_export_params(&self, mesh_dir: &PathBuf) {
        println!("\n📋 导出参数:");
        println!("   - 参考号: {:?}", self.refnos_str);
        if let Some(ref nouns) = self.filter_nouns {
            println!("   - 过滤类型: {:?}", nouns);
        }
        println!("   - 包含子孙节点: {}", self.include_descendants);
        println!("   - Mesh 目录: {}", mesh_dir.display());
        println!("   - 源单位: {}", self.source_unit);
        println!("   - 目标单位: {}", self.target_unit);
        println!("   - 详细输出: {}", self.verbose);
        println!("   - 基础材质: {}", self.use_basic_materials);
        println!("   - 全库导出: {}", self.run_all_dbnos);
        println!("   - 按 SITE 拆分: {}", self.split_by_site);
    }
}

fn parse_length_unit(unit: &str) -> LengthUnit {
    match unit.to_lowercase().as_str() {
        "mm" => LengthUnit::Millimeter,
        "cm" => LengthUnit::Centimeter,
        "dm" => LengthUnit::Decimeter,
        "m" => LengthUnit::Meter,
        "in" => LengthUnit::Inch,
        "ft" => LengthUnit::Foot,
        "yd" => LengthUnit::Yard,
        _ => LengthUnit::Millimeter,
    }
}

/// 连接 SurrealDB（用于读取 PDMS 输入数据或写入模型数据）。
///
/// 约定（重要）：cache-only 也需要连接 SurrealDB 作为“输入数据源”（PE/属性/世界矩阵等）。
/// cache-only 的区别仅在于：不写入 inst_* 等模型相关表，且导出期实例数据优先从 foyer cache 读取。
async fn ensure_surreal_connected(db_option_ext: &DbOptionExt) -> Result<()> {
    if db_option_ext.use_surrealdb {
        println!("\n📡 连接数据库（SurrealDB 写入启用）...");
    } else {
        println!("\n📡 连接数据库（SurrealDB 只读）...");
    }
    init_surreal()
        .await
        .context("初始化 SurrealDB 失败（需要读取 PDMS 输入数据）")?;
    println!("✅ 数据库连接成功");
    Ok(())
}

/// 导出 OBJ 模型模式
pub async fn export_obj_mode(config: ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🎯 OBJ 导出模式");
    println!("================");

    // cache-only 也需要连接 SurrealDB（输入数据源）；区别仅在于 instances 的读取/写入策略。
    ensure_surreal_connected(db_option_ext).await?;
    if !db_option_ext.use_surrealdb {
        println!("📦 cache-only：OBJ 实例数据从 foyer cache 读取（不从 SurrealDB 查询 inst_relate）");
    }

    // 如果需要导出 SVG，设置环境变量
    if config.export_svg {
        println!("🎨 启用 SVG 截面导出");
        unsafe {
            std::env::set_var("EXPORT_SVG", "true");
        }
    } else {
        unsafe {
            std::env::remove_var("EXPORT_SVG");
        }
    }

    // 获取 mesh 目录
    let mesh_dir = config.get_mesh_dir(db_option_ext);

    // 打印导出参数
    config.print_export_params(&mesh_dir);

    // 如果未指定 dbnum 且未提供 refnos，但要求全库导出，则在此处理
    if config.run_all_dbnos && config.dbnum.is_none() && config.refnos_str.is_empty() {
        println!("\n🔁 进入全库 OBJ 导出模式 (MDB 所有 dbnum)");
        let dbnos = query_mdb_db_nums(None, DBType::DESI).await?;
        if dbnos.is_empty() {
            println!("⚠️ MDB 未返回任何 dbnum，跳过导出");
            return Ok(());
        }
        for db in dbnos {
            let mut per_db_config = config.clone();
            per_db_config.dbnum = Some(db);
            if let Err(e) = export_obj_mode_for_db(&per_db_config, db_option_ext).await {
                println!("❌ 导出 dbnum={} 失败: {}", db, e);
            }
        }
        println!("\n🎉 全库 OBJ 导出完成");
        return Ok(());
    }

    // 检查是否指定了 dbnum
    if config.dbnum.is_some() {
        export_obj_mode_for_db(&config, db_option_ext).await?;
    } else {
        // 原有逻辑：按 refnos 导出
        // 解析参考号
        let refnos = config.parse_refnos()?;

        // 检查是否需要重新生成 plant mesh
        if config.regenerate_plant_mesh {
            println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
            println!("   - 强制开启 replace_mesh 和 gen_mesh");

            unsafe {
                std::env::set_var("FORCE_REPLACE_MESH", "true");
            }

            // 无论是否写库，--regen-model 都表示“重建模型数据”：
            // - use_surrealdb=false：生成结果落地 foyer cache（SurrealDB 仅作为输入源，不写 inst_*）
            // - use_surrealdb=true ：同时允许写入/对照验证
            use aios_database::fast_model::gen_all_geos_data;

            let mut db_option_clone = db_option_ext.inner.clone();
            let original_replace_mesh = db_option_clone.replace_mesh;
            let original_gen_mesh = db_option_clone.gen_mesh;
            db_option_clone.replace_mesh = Some(true);
            db_option_clone.gen_mesh = true;

            let mut db_option_ext_override = db_option_ext.clone();
            db_option_ext_override.inner = db_option_clone.clone();

            // 导出若包含子孙节点，regen 也必须覆盖同一范围；此处使用 TreeIndex 计算子孙集合。
            let regen_refnos =
                collect_export_refnos(&refnos, config.include_descendants, None, config.verbose)
                    .await?;

            // 生成时需要读取输入数据（PE/属性/世界矩阵等），因此需连接 SurrealDB。
            ensure_surreal_connected(db_option_ext).await?;
            gen_all_geos_data(regen_refnos, &db_option_ext_override, None, None).await?;

            db_option_clone.replace_mesh = original_replace_mesh;
            db_option_clone.gen_mesh = original_gen_mesh;

            unsafe {
                std::env::remove_var("FORCE_REPLACE_MESH");
            }

            println!("✅ Plant mesh 重新生成完成");
        }

        let exporter = ObjExporter::new();
        for refno in &refnos {
            // 确定输出文件名
            let final_output_path = if let Some(ref path) = config.output_path {
                path.clone()
            } else {
                let base_name = get_output_filename_for_refno(*refno).await;
                // 确保输出到 output/{project_name} 目录
                format!("{}/{}", db_option_ext.get_project_output_dir().display(), base_name)
            };

            println!("\n🔄 导出 {} -> {} ...", refno, final_output_path);

            let export_config = ObjExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    // OBJ 导出：默认 cache-only；如需 SurrealDB 需显式 `--use-surrealdb`。
                    allow_surrealdb: db_option_ext.use_surrealdb,
                    cache_dir: if db_option_ext.use_surrealdb {
                        None
                    } else {
                        Some(db_option_ext.get_foyer_cache_dir())
                    },
                },
            };

            exporter
                .export(&[*refno], &mesh_dir, &final_output_path, export_config)
                .await?;

            println!("✅ 导出成功: {}", final_output_path);
        }
    }

    println!("\n🎉 导出完成!");
    Ok(())
}

async fn export_obj_mode_for_db(config: &ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    let mesh_dir = config.get_mesh_dir(db_option_ext);
    let dbnum = config
        .dbnum
        .expect("dbnum required in export_obj_mode_for_db");
    println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
    println!("📊 查询该数据库下的所有 SITE...");

    use aios_database::fast_model::query_provider;
    let sites: Vec<RefnoEnum> = query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
    println!("   - 找到 {} 个 SITE", sites.len());

    if sites.is_empty() {
        println!("⚠️  未找到任何 SITE，跳过导出");
        return Ok(());
    }

    if config.regenerate_plant_mesh {
        println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
        println!("   - 强制开启 replace_mesh 和 gen_mesh");
        unsafe {
            std::env::set_var("FORCE_REPLACE_MESH", "true");
        }

        use aios_database::fast_model::gen_all_geos_data;
        ensure_surreal_connected(db_option_ext).await?;

        let mut db_option_clone = db_option_ext.inner.clone();
        let original_replace_mesh = db_option_clone.replace_mesh;
        let original_gen_mesh = db_option_clone.gen_mesh;
        db_option_clone.replace_mesh = Some(true);
        db_option_clone.gen_mesh = true;
        let mut db_option_ext_override = db_option_ext.clone();
        db_option_ext_override.inner = db_option_clone.clone();
        gen_all_geos_data(sites.clone(), &db_option_ext_override, None, None).await?;
        db_option_clone.replace_mesh = original_replace_mesh;
        db_option_clone.gen_mesh = original_gen_mesh;

        unsafe {
            std::env::remove_var("FORCE_REPLACE_MESH");
        }
        println!("✅ Plant mesh 重新生成完成");
    }

    let exporter = ObjExporter::new();

    // 检查是否按 SITE 拆分（默认合并）
    if config.split_by_site {
        // 拆分模式：每个 SITE 单独导出
        println!("\n📂 拆分模式：每个 SITE 导出为独立文件");
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "obj").await;
            let output_file = format!("{}/{}", db_option_ext.get_project_output_dir().display(), site_name);
            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );
            let export_config = ObjExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::default(),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    // dbnum/SITE 导出：默认仍使用 SurrealDB（全库查询与命名依赖）。
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            if let Err(e) = exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                println!(
                    "❌ [{}/{}] 导出失败: {} - {}",
                    idx + 1,
                    sites.len(),
                    output_file,
                    e
                );
            } else {
                println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
            }
        }
    } else {
        // 默认合并模式：将所有 SITE 合并到一个文件
        println!("\n🔀 合并模式：将所有 SITE 合并到一个文件（默认）");
        let output_file = format!("{}/dbno_{}.obj", db_option_ext.get_project_output_dir().display(), dbnum);
        println!(
            "🔄 导出合并文件: {} (包含 {} 个 SITE)",
            output_file,
            sites.len()
        );

        let export_config = ObjExportConfig {
            common: CommonExportConfig {
                include_descendants: config.include_descendants,
                filter_nouns: config.filter_nouns.clone(),
                verbose: config.verbose,
                unit_converter: UnitConverter::default(),
                use_basic_materials: config.use_basic_materials,
                include_negative: config.include_negative,
                allow_surrealdb: true,
                cache_dir: None,
            },
        };

        // 将所有 SITE 一次性导出
        if let Err(e) = exporter
            .export(&sites, &mesh_dir, &output_file, export_config)
            .await
        {
            println!("❌ 导出失败: {} - {}", output_file, e);
        } else {
            println!("✅ 导出成功: {}", output_file);
        }
    }

    Ok(())
}

/// 导出 GLB 模型模式
pub async fn export_glb_mode(config: ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🎯 GLB 导出模式");
    println!("================");

    ensure_surreal_connected(db_option_ext).await?;
    if !db_option_ext.use_surrealdb {
        println!("📦 cache-only：GLB 实例数据从 foyer cache 读取（不从 SurrealDB 查询 inst_relate）");
    }

    // 获取 mesh 目录
    let mesh_dir = config.get_mesh_dir(db_option_ext);

    // 打印导出参数
    config.print_export_params(&mesh_dir);

    // 全库导出（无 dbnum 且无 refnos）
    if config.run_all_dbnos && config.dbnum.is_none() && config.refnos_str.is_empty() {
        println!("\n🔁 进入全库 GLB 导出模式 (MDB 所有 dbnum)");
        let dbnos: Vec<u32> = query_mdb_db_nums(None, DBType::DESI).await?;
        if dbnos.is_empty() {
            println!("⚠️ MDB 未返回任何 dbnum，跳过导出");
            return Ok(());
        }
        for db in dbnos {
            let mut per_db_config = config.clone();
            per_db_config.dbnum = Some(db);
            if let Err(e) = export_glb_mode_for_db(&per_db_config, db_option_ext).await {
                println!("❌ 导出 dbnum={} 失败: {}", db, e);
            }
        }
        println!("\n🎉 全库 GLB 导出完成");
        return Ok(());
    }

    // 检查是否指定了 dbnum
    if config.dbnum.is_some() {
        export_glb_mode_for_db(&config, db_option_ext).await?;
    } else {
        // 原有逻辑：按 refnos 导出
        // 解析参考号
        let refnos = config.parse_refnos()?;

        // 检查是否需要重新生成 plant mesh
        if config.regenerate_plant_mesh {
            println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
            println!("   - 强制开启 replace_mesh 和 gen_mesh");

            use aios_database::fast_model::gen_all_geos_data;
        ensure_surreal_connected(db_option_ext).await?;

            unsafe {
                std::env::set_var("FORCE_REPLACE_MESH", "true");
            }

            let mut db_option_clone = db_option_ext.inner.clone();
            let original_replace_mesh = db_option_clone.replace_mesh;
            let original_gen_mesh = db_option_clone.gen_mesh;
            db_option_clone.replace_mesh = Some(true);
            db_option_clone.gen_mesh = true;

            let mut db_option_ext_override = db_option_ext.clone();
            db_option_ext_override.inner = db_option_clone.clone();
            let regen_refnos =
                collect_export_refnos(&refnos, config.include_descendants, None, config.verbose)
                    .await?;
            gen_all_geos_data(regen_refnos, &db_option_ext_override, None, None).await?;

            db_option_clone.replace_mesh = original_replace_mesh;
            db_option_clone.gen_mesh = original_gen_mesh;

            unsafe {
                std::env::remove_var("FORCE_REPLACE_MESH");
            }

            println!("✅ Plant mesh 重新生成完成");
        }

        let exporter = GlbExporter::new();
        for refno in &refnos {
            let final_output_path = if let Some(ref path) = config.output_path {
                path.clone()
            } else {
                let base_name =
                    get_output_filename_for_refno(*refno).await;
                // 确保输出到 output/{project_name} 目录
                format!("{}/{}.glb", db_option_ext.get_project_output_dir().display(), base_name.replace(".obj", ""))
            };

            println!("\n🔄 导出 {} -> {} ...", refno, final_output_path);

            let export_config = GlbExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: db_option_ext.use_surrealdb,
                    cache_dir: if db_option_ext.use_surrealdb {
                        None
                    } else {
                        Some(db_option_ext.get_foyer_cache_dir())
                    },
                },
            };
            let _ = GlbExporter::new()
                .export(&[*refno], &mesh_dir, &final_output_path, export_config)
                .await?;

            println!("✅ 导出成功: {}", final_output_path);
        }
    }

    println!("\n🎉 导出完成!");
    Ok(())
}

async fn export_glb_mode_for_db(config: &ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    let mesh_dir = config.get_mesh_dir(db_option_ext);
    let dbnum = config
        .dbnum
        .expect("dbnum required in export_glb_mode_for_db");
    println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
    println!("📊 查询该数据库下的所有 SITE...");

    use aios_database::fast_model::query_provider;
    let sites: Vec<RefnoEnum> = query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
    println!("   - 找到 {} 个 SITE", sites.len());

    if sites.is_empty() {
        println!("⚠️  未找到任何 SITE，跳过导出");
        return Ok(());
    }

    if config.regenerate_plant_mesh {
        println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
        println!("   - 强制开启 replace_mesh 和 gen_mesh");
        use aios_database::fast_model::gen_all_geos_data;
        ensure_surreal_connected(db_option_ext).await?;
        unsafe {
            std::env::set_var("FORCE_REPLACE_MESH", "true");
        }
        let mut db_option_clone = db_option_ext.inner.clone();
        let original_replace_mesh = db_option_clone.replace_mesh;
        let original_gen_mesh = db_option_clone.gen_mesh;
        db_option_clone.replace_mesh = Some(true);
        db_option_clone.gen_mesh = true;
        let mut db_option_ext_override = db_option_ext.clone();
        db_option_ext_override.inner = db_option_clone.clone();
        gen_all_geos_data(sites.clone(), &db_option_ext_override, None, None).await?;
        db_option_clone.replace_mesh = original_replace_mesh;
        db_option_clone.gen_mesh = original_gen_mesh;
        unsafe {
            std::env::remove_var("FORCE_REPLACE_MESH");
        }
        println!("✅ Plant mesh 重新生成完成");
    }

    let exporter = GlbExporter::new();

    // 检查是否按 SITE 拆分（默认合并）
    if config.split_by_site {
        // 拆分模式：每个 SITE 单独导出
        println!("\n📂 拆分模式：每个 SITE 导出为独立文件");
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "glb").await;
            let output_file = format!("{}/{}", db_option_ext.get_project_output_dir().display(), site_name);
            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );
            let export_config = GlbExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::default(),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            if let Err(e) = exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                println!(
                    "❌ [{}/{}] 导出失败: {} - {}",
                    idx + 1,
                    sites.len(),
                    output_file,
                    e
                );
            } else {
                println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
            }
        }
    } else {
        // 默认合并模式：将所有 SITE 合并到一个文件
        println!("\n🔀 合并模式：将所有 SITE 合并到一个文件（默认）");
        let output_file = format!("{}/dbno_{}.glb", db_option_ext.get_project_output_dir().display(), dbnum);
        println!(
            "🔄 导出合并文件: {} (包含 {} 个 SITE)",
            output_file,
            sites.len()
        );

        let export_config = GlbExportConfig {
            common: CommonExportConfig {
                include_descendants: config.include_descendants,
                filter_nouns: config.filter_nouns.clone(),
                verbose: config.verbose,
                unit_converter: UnitConverter::default(),
                use_basic_materials: config.use_basic_materials,
                include_negative: config.include_negative,
                allow_surrealdb: true,
                cache_dir: None,
            },
        };

        // 将所有 SITE 一次性导出
        if let Err(e) = exporter
            .export(&sites, &mesh_dir, &output_file, export_config)
            .await
        {
            println!("❌ 导出失败: {} - {}", output_file, e);
        } else {
            println!("✅ 导出成功: {}", output_file);
        }
    }

    Ok(())
}

/// 导出 glTF 模型模式
pub async fn export_gltf_mode(config: ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    println!("\n🎯 glTF 导出模式");
    println!("================");

    ensure_surreal_connected(db_option_ext).await?;
    if !db_option_ext.use_surrealdb {
        println!("📦 cache-only：glTF 实例数据从 foyer cache 读取（不从 SurrealDB 查询 inst_relate）");
    }

    // 获取 mesh 目录
    let mesh_dir = config.get_mesh_dir(db_option_ext);

    // 打印导出参数
    config.print_export_params(&mesh_dir);

    // 全库导出（无 dbnum 且无 refnos）
    if config.run_all_dbnos && config.dbnum.is_none() && config.refnos_str.is_empty() {
        println!("\n🔁 进入全库 GLTF 导出模式 (MDB 所有 dbnum)");
        let dbnos: Vec<u32> = query_mdb_db_nums(None, DBType::DESI).await?;
        if dbnos.is_empty() {
            println!("⚠️ MDB 未返回任何 dbnum，跳过导出");
            return Ok(());
        }
        for db in dbnos {
            let mut per_db_config = config.clone();
            per_db_config.dbnum = Some(db);
            if let Err(e) = export_gltf_mode_for_db(&per_db_config, db_option_ext).await {
                println!("❌ 导出 dbnum={} 失败: {}", db, e);
            }
        }
        println!("\n🎉 全库 GLTF 导出完成");
        return Ok(());
    }

    // 检查是否指定了 dbnum
    if let Some(dbnum) = config.dbnum {
        println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
        println!("📊 查询该数据库下的所有 SITE...");

        use aios_database::fast_model::query_provider;
        let sites: Vec<RefnoEnum> = query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
        println!("   - 找到 {} 个 SITE", sites.len());

        if sites.is_empty() {
            println!("⚠️  未找到任何 SITE，跳过导出");
            return Ok(());
        }

        // 检查是否需要重新生成 plant mesh
        if config.regenerate_plant_mesh {
            println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
            println!("   - 强制开启 replace_mesh 和 gen_mesh");

            use aios_database::fast_model::gen_all_geos_data;
            ensure_surreal_connected(db_option_ext).await?;

            unsafe {
                std::env::set_var("FORCE_REPLACE_MESH", "true");
            }

            let mut db_option_clone = db_option_ext.inner.clone();
            let original_replace_mesh = db_option_clone.replace_mesh;
            let original_gen_mesh = db_option_clone.gen_mesh;
            db_option_clone.replace_mesh = Some(true);
            db_option_clone.gen_mesh = true;

            let mut db_option_ext_override = db_option_ext.clone();
            db_option_ext_override.inner = db_option_clone.clone();
            gen_all_geos_data(sites.clone(), &db_option_ext_override, None, None).await?;

            db_option_clone.replace_mesh = original_replace_mesh;
            db_option_clone.gen_mesh = original_gen_mesh;

            unsafe {
                std::env::remove_var("FORCE_REPLACE_MESH");
            }

            println!("✅ Plant mesh 重新生成完成");
        }

        let exporter = GltfExporter::new();
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "gltf").await;
            let output_file = format!("{}/{}", db_option_ext.get_project_output_dir().display(), site_name);

            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );

            let export_config = GltfExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: db_option_ext.use_surrealdb,
                    cache_dir: if db_option_ext.use_surrealdb {
                        None
                    } else {
                        Some(db_option_ext.get_foyer_cache_dir())
                    },
                },
            };
            match exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                Ok(_) => {
                    println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
                }
                Err(e) => {
                    println!(
                        "❌ [{}/{}] 导出失败: {} - {}",
                        idx + 1,
                        sites.len(),
                        output_file,
                        e
                    );
                }
            }
        }
    } else {
        // 原有逻辑：按 refnos 导出
        // 解析参考号
        let refnos = config.parse_refnos()?;

        // 检查是否需要重新生成 plant mesh
        if config.regenerate_plant_mesh {
            println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
            println!("   - 强制开启 replace_mesh 和 gen_mesh");

            use aios_database::fast_model::gen_all_geos_data;

            unsafe {
                std::env::set_var("FORCE_REPLACE_MESH", "true");
            }

            let mut db_option_clone = db_option_ext.inner.clone();
            let original_replace_mesh = db_option_clone.replace_mesh;
            let original_gen_mesh = db_option_clone.gen_mesh;
            db_option_clone.replace_mesh = Some(true);
            db_option_clone.gen_mesh = true;

            let mut db_option_ext_override = db_option_ext.clone();
            db_option_ext_override.inner = db_option_clone.clone();
            let regen_refnos =
                collect_export_refnos(&refnos, config.include_descendants, None, config.verbose)
                    .await?;
            gen_all_geos_data(regen_refnos, &db_option_ext_override, None, None).await?;

            db_option_clone.replace_mesh = original_replace_mesh;
            db_option_clone.gen_mesh = original_gen_mesh;

            unsafe {
                std::env::remove_var("FORCE_REPLACE_MESH");
            }

            println!("✅ Plant mesh 重新生成完成");
        }

        let exporter = GltfExporter::new();
        for refno in &refnos {
            let final_output_path = if let Some(ref path) = config.output_path {
                path.clone()
            } else {
                let base_name =
                    get_output_filename_for_refno(*refno).await;
                // 确保输出到 output/{project_name} 目录
                format!("{}/{}.gltf", db_option_ext.get_project_output_dir().display(), base_name.replace(".obj", ""))
            };

            println!("\n🔄 导出 {} -> {} ...", refno, final_output_path);

            let export_config = GltfExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::new(
                        parse_length_unit(&config.source_unit),
                        parse_length_unit(&config.target_unit),
                    ),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            exporter
                .export(&[*refno], &mesh_dir, &final_output_path, export_config)
                .await?;

            println!("✅ 导出成功: {}", final_output_path);
        }
    }

    println!("\n🎉 导出完成!");
    Ok(())
}

async fn export_gltf_mode_for_db(config: &ExportConfig, db_option_ext: &DbOptionExt) -> Result<()> {
    let mesh_dir = config.get_mesh_dir(db_option_ext);
    let dbnum = config
        .dbnum
        .expect("dbnum required in export_gltf_mode_for_db");
    println!("\n🔍 检测到 dbnum 参数: {}", dbnum);
    println!("📊 查询该数据库下的所有 SITE...");

    use aios_database::fast_model::query_provider;
    let sites: Vec<RefnoEnum> = query_provider::query_by_type(&["SITE"], dbnum as i32, None).await?;
    println!("   - 找到 {} 个 SITE", sites.len());

    if sites.is_empty() {
        println!("⚠️  未找到任何 SITE，跳过导出");
        return Ok(());
    }

    if config.regenerate_plant_mesh {
        println!("\n🔄 检测到 --regen-model 参数，开始重新生成几何体数据...");
        println!("   - 强制开启 replace_mesh 和 gen_mesh");
        use aios_database::fast_model::gen_all_geos_data;
        ensure_surreal_connected(db_option_ext).await?;
        unsafe {
            std::env::set_var("FORCE_REPLACE_MESH", "true");
        }
        let mut db_option_clone = db_option_ext.inner.clone();
        let original_replace_mesh = db_option_clone.replace_mesh;
        let original_gen_mesh = db_option_clone.gen_mesh;
        db_option_clone.replace_mesh = Some(true);
        db_option_clone.gen_mesh = true;
        let mut db_option_ext_override = db_option_ext.clone();
        db_option_ext_override.inner = db_option_clone.clone();
        gen_all_geos_data(sites.clone(), &db_option_ext_override, None, None).await?;
        db_option_clone.replace_mesh = original_replace_mesh;
        db_option_clone.gen_mesh = original_gen_mesh;
        unsafe {
            std::env::remove_var("FORCE_REPLACE_MESH");
        }
        println!("✅ Plant mesh 重新生成完成");
    }

    let exporter = GltfExporter::new();

    // 检查是否按 SITE 拆分（默认合并）
    if config.split_by_site {
        // 拆分模式：每个 SITE 单独导出
        println!("\n📂 拆分模式：每个 SITE 导出为独立文件");
        for (idx, site_refno) in sites.iter().enumerate() {
            let site_name = get_site_name_for_export(*site_refno, dbnum, "gltf").await;
            let output_file = format!("{}/{}", db_option_ext.get_project_output_dir().display(), site_name);
            println!(
                "\n🔄 [{}/{}] 导出 SITE: {} -> {}",
                idx + 1,
                sites.len(),
                site_refno,
                output_file
            );
            let export_config = GltfExportConfig {
                common: CommonExportConfig {
                    include_descendants: config.include_descendants,
                    filter_nouns: config.filter_nouns.clone(),
                    verbose: config.verbose,
                    unit_converter: UnitConverter::default(),
                    use_basic_materials: config.use_basic_materials,
                    include_negative: config.include_negative,
                    allow_surrealdb: true,
                    cache_dir: None,
                },
            };
            if let Err(e) = exporter
                .export(&[*site_refno], &mesh_dir, &output_file, export_config)
                .await
            {
                println!(
                    "❌ [{}/{}] 导出失败: {} - {}",
                    idx + 1,
                    sites.len(),
                    output_file,
                    e
                );
            } else {
                println!("✅ [{}/{}] 导出成功: {}", idx + 1, sites.len(), output_file);
            }
        }
    } else {
        // 默认合并模式：将所有 SITE 合并到一个文件
        println!("\n🔀 合并模式：将所有 SITE 合并到一个文件（默认）");
        let output_file = format!("{}/dbno_{}.gltf", db_option_ext.get_project_output_dir().display(), dbnum);
        println!(
            "🔄 导出合并文件: {} (包含 {} 个 SITE)",
            output_file,
            sites.len()
        );

        let export_config = GltfExportConfig {
            common: CommonExportConfig {
                include_descendants: config.include_descendants,
                filter_nouns: config.filter_nouns.clone(),
                verbose: config.verbose,
                unit_converter: UnitConverter::default(),
                use_basic_materials: config.use_basic_materials,
                include_negative: config.include_negative,
                allow_surrealdb: true,
                cache_dir: None,
            },
        };

        // 将所有 SITE 一次性导出
        if let Err(e) = exporter
            .export(&sites, &mesh_dir, &output_file, export_config)
            .await
        {
            println!("❌ 导出失败: {} - {}", output_file, e);
        } else {
            println!("✅ 导出成功: {}", output_file);
        }
    }

    Ok(())
}

/// 获取输出文件名（优先基于 PE.name；失败则回退为 refno）
pub async fn get_output_filename_for_refno(refno: RefnoEnum) -> String {
    use aios_database::fast_model::query_provider;

    // 1. 尝试获取 PE 的 name
    if let Ok(Some(pe)) = query_provider::get_pe(refno).await {
        let name = pe.name;

        // 如果 PE.name 不为空，使用它
        if !name.is_empty() {
            let clean_name = sanitize_filename(&name);
            return format!("{}.obj", clean_name);
        }

        // 如果 PE.name 为空，尝试从 NamedAttrMap 获取 NAME 属性
        if let Ok(attmap) = aios_core::get_named_attmap(refno).await {
            if let Some(attr_name) = attmap.get_as_string("NAME") {
                if !attr_name.is_empty() {
                    let clean_name = sanitize_filename(&attr_name);
                    return format!("{}.obj", clean_name);
                }
            }
        }
    }

    // 2. 如果 name 为空或查询失败，使用 refno
    format!("{}.obj", refno.to_string().replace('/', "_"))
}

/// 获取 SITE 名称用于导出（带 dbnum 前缀）
pub async fn get_site_name_for_export(refno: RefnoEnum, dbnum: u32, extension: &str) -> String {
    use aios_database::fast_model::query_provider;

    // 1. 尝试获取 PE 的 name
    let site_name = if let Ok(Some(pe)) = query_provider::get_pe(refno).await {
        let name = pe.name;

        // 如果 PE.name 不为空，使用它
        if !name.is_empty() {
            sanitize_filename(&name)
        } else {
            // 尝试从 NamedAttrMap 获取 NAME 属性
            if let Ok(attmap) = aios_core::get_named_attmap(refno).await {
                if let Some(attr_name) = attmap.get_as_string("NAME") {
                    if !attr_name.is_empty() {
                        sanitize_filename(&attr_name)
                    } else {
                        refno.to_string().replace('/', "_")
                    }
                } else {
                    refno.to_string().replace('/', "_")
                }
            } else {
                refno.to_string().replace('/', "_")
            }
        }
    } else {
        // 如果查询失败，使用 refno
        refno.to_string().replace('/', "_")
    };

    format!("{}_{}.{}", dbnum, site_name, extension)
}

fn sanitize_filename(name: &str) -> String {
    let mut result = name
        .replace('/', "_")
        .replace('\\', "_")
        .replace(':', "_")
        .replace('*', "_")
        .replace('?', "_")
        .replace('"', "_")
        .replace('<', "_")
        .replace('>', "_")
        .replace('|', "_")
        .replace(' ', "_");

    // 移除开头的斜线（第一个字符如果是 _，说明原来第一个字符是 /，需要去掉）
    if result.starts_with('_') {
        result = result.strip_prefix('_').unwrap_or(&result).to_string();
    }

    result
}

#[cfg(feature = "grpc")]
use clap::ArgMatches;

#[cfg(feature = "grpc")]
/// 启动 GRPC 服务器模式
pub async fn start_grpc_server_mode(
    matches: &ArgMatches,
    _db_option_ext: DbOptionExt,
) -> Result<()> {
    use aios_database::grpc_service::{init_grpc_logging, server::GrpcServerConfig};

    // 初始化日志
    init_grpc_logging()?;

    // 获取端口配置
    let port: u16 = matches
        .get_one::<String>("grpc-port")
        .unwrap()
        .parse()
        .map_err(|_| anyhow!("Invalid port number"))?;

    // 创建服务器配置
    let config = GrpcServerConfig {
        host: "0.0.0.0".to_string(),
        port,
        max_concurrent_tasks: 4,
        enable_reflection: true,
    };

    println!(
        "Starting AIOS Database GRPC Server...\nServer will listen on {}:{}",
        config.host, config.port
    );

    aios_database::grpc_service::server::start_grpc_server_with_config(config).await?;

    Ok(())
}

async fn export_instanced_bundle_mode(
    config: ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use std::sync::Arc;

    println!("\n🎯 Instanced Bundle 导出模式");
    println!("================");

    // 解析参考号
    let refnos: Vec<RefnoEnum> = if config.refnos_str.is_empty() {
        return Err(anyhow!("请指定参考号"));
    } else {
        config
            .refnos_str
            .iter()
            .map(|s| RefU64::from_str(s).map(RefnoEnum::Refno))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("解析参考号失败: {}", e))?
    };

    println!("   - 参考号数量: {}", refnos.len());
    if config.verbose {
        for refno in &refnos {
            println!("      {}", refno);
        }
    }

    // 确定输出目录
    let output_dir = config.output_path.clone().unwrap_or_else(|| {
        let first_refno = refnos[0].to_string().replace('/', "_");
        format!("{}/instanced-bundle/{}", db_option_ext.get_project_output_dir().display(), first_refno)
    });

    println!("   - 输出目录: {}", output_dir);

    // 获取 mesh 目录
    let mesh_dir = PathBuf::from(db_option_ext.get_meshes_path());
    println!("   - Mesh 目录: {}", mesh_dir.display());

    // 执行导出
    export_instanced_bundle_for_refnos(
        &refnos,
        &mesh_dir,
        &PathBuf::from(&output_dir),
        Arc::new(db_option_ext.inner.clone()),
        config.verbose,
    )
    .await?;

    println!("\n✅ Instanced Bundle 导出完成");
    println!("   输出目录: {}", output_dir);

    Ok(())
}

/// 统一的模型导出模式（支持多种格式）
pub async fn export_model_mode(
    format: &str,
    config: ExportConfig,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    match format.to_lowercase().as_str() {
        "obj" => {
            let obj_config = config.with_unit_conversion("mm", "mm");
            export_obj_mode(obj_config, db_option_ext).await
        }
        "glb" => {
            let glb_config = config.with_unit_conversion("mm", "mm");
            export_glb_mode(glb_config, db_option_ext).await
        }
        "gltf" => {
            let gltf_config = config.with_unit_conversion("mm", "mm");
            export_gltf_mode(gltf_config, db_option_ext).await
        }
        "xkt" => {
            return Err(anyhow!("XKT 导出功能已禁用，需要重新启用 gen_model 特性"));
        }
        "instanced-bundle" | "instanced_bundle" => {
            export_instanced_bundle_mode(config, db_option_ext).await
        }
        _ => Err(anyhow!(
            "不支持的导出格式: {}，支持的格式: obj, glb, gltf, xkt, instanced-bundle",
            format
        )),
    }
}

/// 导出所有 inst_relate 实体（Prepack LOD 格式）
pub async fn export_all_relates_mode(
    dbnum: Option<u32>,
    verbose: bool,
    output_override: Option<PathBuf>,
    owner_types: Option<Vec<String>>,
    name_config_path: Option<PathBuf>,
    export_all_lods: bool,
    export_refnos: Option<String>,
    source_unit: String,
    target_unit: String,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::export_model::NameConfig;
    use aios_database::fast_model::export_model::export_prepack_lod::export_all_relates_prepack_lod;
    use std::sync::Arc;

    println!("\n🎯 导出所有 inst_relate 实体模式");
    println!("============================");

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 加载名称配置（如果提供了路径）
    let name_config = if let Some(path) = name_config_path {
        Some(NameConfig::load_from_excel(&path)?)
    } else {
        None
    };

    // 调用导出函数（通过 Deref 访问内部的 DbOption）
    let db_option = Arc::new((**db_option_ext).clone());
    export_all_relates_prepack_lod(
        dbnum,
        verbose,
        output_override,
        owner_types,
        name_config,
        db_option,
        export_all_lods,
        export_refnos,
        source_unit,
        target_unit,
    )
    .await?;

    println!("\n🎉 导出完成！");
    Ok(())
}

pub async fn export_all_parquet_mode(
    dbnum: Option<u32>,
    verbose: bool,
    output_override: Option<PathBuf>,
    owner_types: Option<Vec<String>>,
    name_config_path: Option<PathBuf>,
    export_all_lods: bool,
    export_refnos: Option<String>,
    source_unit: String,
    target_unit: String,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::export_model::NameConfig;
    use aios_database::fast_model::export_model::export_prepack_lod::export_all_relates_prepack_lod_parquet;
    use std::sync::Arc;

    println!("\n🎯 导出所有 inst_relate 实体模式 (Parquet)");
    println!("============================");

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 加载名称配置（如果提供了路径）
    let name_config = if let Some(path) = name_config_path {
        Some(NameConfig::load_from_excel(&path)?)
    } else {
        None
    };

    // 调用导出函数（通过 Deref 访问内部的 DbOption）
    let db_option = Arc::new((**db_option_ext).clone());
    export_all_relates_prepack_lod_parquet(
        dbnum,
        verbose,
        output_override,
        owner_types,
        name_config,
        db_option,
        export_all_lods,
        export_refnos,
        source_unit,
        target_unit,
    )
    .await?;

    println!("\n🎉 导出完成！");
    Ok(())
}

/// 导出指定 dbnum 的实例数据为简化 JSON 格式（含 AABB）
///
/// # 参数
/// - `autorun`: 若为 `true`（默认），缓存缺失时自动生成模型数据；若为 `false`，则询问用户确认
pub async fn export_dbnum_instances_json_mode(
    dbnum: u32,
    verbose: bool,
    output_override: Option<PathBuf>,
    db_option_ext: &DbOptionExt,
    autorun: bool,
) -> Result<()> {
    use aios_database::fast_model::export_model::export_prepack_lod::{
        export_dbnum_instances_json, export_dbnum_instances_json_from_cache,
        export_global_trans_aabb_json,
    };
    use std::sync::Arc;

    println!("\n🎯 导出 dbnum 实例数据为 JSON（含 AABB）");
    println!("====================================");

    // 设置输出目录
    let output_dir = output_override.unwrap_or_else(|| db_option_ext.get_project_output_dir().join("instances"));

    if db_option_ext.use_cache {
        let cache_dir = db_option_ext.get_foyer_cache_dir();
        let mesh_dir = ExportConfig::default().get_mesh_dir(db_option_ext);
        let mesh_lod_tag = format!("{:?}", db_option_ext.inner.mesh_precision.default_lod);
        let result = export_dbnum_instances_json_from_cache(
            dbnum,
            &output_dir,
            &cache_dir,
            Some(&mesh_dir),
            Some(mesh_lod_tag.as_str()),
            verbose,
            None,
        )
        .await;
        match result {
            Ok((stats, trans_count, aabb_count)) => {
                println!("\n🎉 导出完成！（缓存路径）");
                println!("📊 统计信息:");
                println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
                println!("   - 子节点数量: {}", stats.descendant_count);
                println!("   - 输出文件大小: {} 字节", stats.output_file_size);
                println!("   - 变换矩阵数量 (trans): {}", trans_count);
                println!("   - 包围盒数量 (aabb): {}", aabb_count);
                println!("   - 耗时: {:?}", stats.elapsed_time);
                return Ok(());
            }
            Err(e) => {
                // 检测是否是缓存缺失错误，提供自动/交互式生成选项
                let err_msg = e.to_string();
                if err_msg.contains("缓存中未找到") || err_msg.contains("批次数据") {
                    println!("\n⚠️  dbnum={} 尚未生成模型数据（缓存为空）", dbnum);

                    // autorun 模式：自动开始生成；否则询问用户
                    let should_generate = if autorun {
                        println!("🔄 autorun 模式已开启，自动开始生成模型数据...");
                        true
                    } else {
                        println!();
                        print!("是否现在开始生成模型数据？(y/n): ");
                        use std::io::{self, Write};
                        io::stdout().flush().ok();

                        let mut input = String::new();
                        if io::stdin().read_line(&mut input).is_ok() {
                            let answer = input.trim().to_lowercase();
                            answer == "y" || answer == "yes"
                        } else {
                            false
                        }
                    };

                    if should_generate {
                        println!("\n🚀 开始生成 dbnum={} 的模型数据...", dbnum);

                        // 调用模型生成逻辑
                        use aios_database::fast_model::gen_all_geos_data;
                        use aios_database::versioned_db::database::sync_pdms;

                        // 连接数据库（生成需要从 SurrealDB 读取输入数据）
                        ensure_surreal_connected(db_option_ext).await?;

                        // Step 1: 检测 TreeIndex 是否存在，若缺失则通过 gen_tree_only 解析生成
                        let tree_path = db_option_ext.get_project_output_dir().join("scene_tree").join(format!("{}.tree", dbnum));
                        if !tree_path.exists() {
                            println!("📂 检测到 TreeIndex 缺失: {}", tree_path.display());
                            println!("🔄 正在通过 PDMS 解析生成 TreeIndex (gen_tree_only 模式)...");

                            let mut parse_option = db_option_ext.inner.clone();
                            parse_option.gen_tree_only = true;
                            parse_option.total_sync = true;
                            parse_option.manual_db_nums = Some(vec![dbnum]);
                            parse_option.save_db = Some(false); // 不写入 SurrealDB

                            if let Err(e) = sync_pdms(&parse_option).await {
                                println!("⚠️  TreeIndex 生成失败: {}", e);
                                println!("   请确保 PDMS 数据库文件存在且可访问");
                                return Err(anyhow!("TreeIndex 生成失败: {}", e));
                            }

                            println!("✅ TreeIndex 生成完成");
                        }

                        // Step 2: 构建生成配置
                        let mut db_option_clone = db_option_ext.inner.clone();
                        db_option_clone.manual_db_nums = Some(vec![dbnum]);
                        db_option_clone.gen_mesh = true;
                        db_option_clone.replace_mesh = Some(true);

                        let mut db_option_ext_override = db_option_ext.clone();
                        db_option_ext_override.inner = db_option_clone;
                        db_option_ext_override.use_cache = true; // 确保缓存写入
                        db_option_ext_override.use_surrealdb = true; // 需要从 SurrealDB 读取输入数据
                        db_option_ext_override.inner.save_db = Some(false); // 不写回 SurrealDB
                        db_option_ext_override.export_instances = false; // 禁用自动导出，由我们的代码单独处理
                        // 禁用 Full Noun 模式，使用全库生成以确保所有类型节点都被处理
                        db_option_ext_override.full_noun_mode = false;

                        unsafe {
                            std::env::set_var("FORCE_REPLACE_MESH", "true");
                        }

                        // Step 3: 生成模型（仅写入 foyer cache）
                        // 捕获错误但继续尝试导出（缓存可能已有部分数据）
                        match gen_all_geos_data(vec![], &db_option_ext_override, None, None).await {
                            Ok(_) => {
                                println!("✅ 模型生成完成");
                            }
                            Err(e) => {
                                eprintln!("⚠️  模型生成过程中出现错误: {}", e);
                                eprintln!("   尝试继续导出已生成的缓存数据...");
                            }
                        }

                        unsafe {
                            std::env::remove_var("FORCE_REPLACE_MESH");
                        }
                        println!("\n🔄 重新尝试导出...");

                        // 重新尝试导出
                        let retry_result = export_dbnum_instances_json_from_cache(
                            dbnum,
                            &output_dir,
                            &cache_dir,
                            Some(&mesh_dir),
                            Some(mesh_lod_tag.as_str()),
                            verbose,
                            None,
                        )
                        .await;

                        match retry_result {
                            Ok((stats, trans_count, aabb_count)) => {
                                println!("\n🎉 导出完成！（缓存路径）");
                                println!("📊 统计信息:");
                                println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
                                println!("   - 子节点数量: {}", stats.descendant_count);
                                println!("   - 输出文件大小: {} 字节", stats.output_file_size);
                                println!("   - 变换矩阵数量 (trans): {}", trans_count);
                                println!("   - 包围盒数量 (aabb): {}", aabb_count);
                                println!("   - 耗时: {:?}", stats.elapsed_time);
                                return Ok(());
                            }
                            Err(retry_e) => {
                                return Err(retry_e);
                            }
                        }
                    }

                    // 用户拒绝或 autorun=false 时无效输入，给出手动命令建议
                    println!("\n💡 建议：请手动运行以下命令生成模型数据：");
                    println!("   cargo run --bin aios-database -- --debug-model --dbnum {} --regen-model", dbnum);
                    return Err(anyhow!(
                        "dbnum={} 尚未生成模型数据，请先生成后再导出",
                        dbnum
                    ));
                }
                return Err(e);
            }
        }
    }

    if !db_option_ext.use_surrealdb {
        return Err(anyhow!("未启用 SurrealDB 且缓存导出失败，无法继续导出"));
    }

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 调用导出函数（SurrealDB 路径）
    let db_option = Arc::new((**db_option_ext).clone());
    let stats = export_dbnum_instances_json(
        dbnum,
        &output_dir,
        db_option,
        verbose,
        None, // 使用默认毫米单位
    )
    .await?;

    // 导出全局 trans.json 和 aabb.json（SurrealDB）
    let (trans_count, aabb_count) =
        export_global_trans_aabb_json(&output_dir, None, verbose).await?;

    println!("\n🎉 导出完成！");
    println!("📊 统计信息:");
    println!("   - BRAN/HANG/EQUI 分组数量: {}", stats.refno_count);
    println!("   - 子节点数量: {}", stats.descendant_count);
    println!("   - 输出文件大小: {} 字节", stats.output_file_size);
    println!("   - 变换矩阵数量 (trans): {}", trans_count);
    println!("   - 包围盒数量 (aabb): {}", aabb_count);
    println!("   - 耗时: {:?}", stats.elapsed_time);
    Ok(())
}

/// 导入 instances.json 到 SQLite 空间索引
#[cfg(feature = "sqlite-index")]
pub fn import_spatial_index_mode(
    json_path: &Path,
    sqlite_path: &Path,
    verbose: bool,
) -> Result<()> {
    use aios_database::sqlite_index::{ImportConfig, SqliteAabbIndex, i64_to_refno_str};

    println!("\n🗃️ 导入 instances.json 到 SQLite 空间索引");
    println!("==========================================");
    println!("   - 输入文件: {}", json_path.display());
    println!("   - 输出文件: {}", sqlite_path.display());

    // 检查输入文件是否存在
    if !json_path.exists() {
        return Err(anyhow!("输入文件不存在: {}", json_path.display()));
    }

    // 确保输出目录存在
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 如果 SQLite 文件已存在，先删除
    if sqlite_path.exists() {
        if verbose {
            println!("   ⚠️ 删除已存在的 SQLite 文件");
        }
        std::fs::remove_file(sqlite_path)?;
    }

    // 创建 SQLite 索引
    let idx = SqliteAabbIndex::open(sqlite_path)?;
    idx.init_schema()?;
    println!("   ✅ SQLite 索引创建成功");

    // 导入配置：EQUI 粗粒度，BRAN/HANG 细粒度
    let config = ImportConfig::default();
    if verbose {
        println!(
            "   配置: EQUI 粗粒度={}, BRAN/HANG 细粒度={}",
            config.equi_coarse, config.bran_fine
        );
    }

    // 执行导入
    let stats = idx.import_from_instances_json(json_path, &config)?;

    println!("\n🎉 导入完成！");
    println!("📊 统计信息:");
    println!("   - EQUI (粗粒度): {}", stats.equi_count);
    println!("   - Children (细粒度): {}", stats.children_count);
    println!("   - Tubings (细粒度): {}", stats.tubings_count);
    println!("   - 总计遍历: {}", stats.total_inserted);
    println!("   - 去重后唯一记录: {}", stats.unique_count);

    // 验证查询
    if verbose {
        let all_aabbs = idx.query_all_aabbs()?;
        println!("\n🔍 验证查询:");
        println!("   查询到 {} 条 AABB 记录", all_aabbs.len());
        if let Some((id, minx, maxx, miny, maxy, minz, maxz)) = all_aabbs.first() {
            let refno = i64_to_refno_str(*id);
            println!(
                "   示例: refno={}, AABB=[{:.1},{:.1}]x[{:.1},{:.1}]x[{:.1},{:.1}]",
                refno, minx, maxx, miny, maxy, minz, maxz
            );
        }
    }

    Ok(())
}

#[cfg(not(feature = "sqlite-index"))]
pub fn import_spatial_index_mode(
    _json_path: &Path,
    _sqlite_path: &Path,
    _verbose: bool,
) -> Result<()> {
    Err(anyhow!(
        "sqlite-index 特性未启用，请使用 --features sqlite-index 编译"
    ))
}

// ============ 房间计算 CLI 模式 ============

/// 房间计算配置
#[derive(Debug, Clone)]
pub struct RoomComputeCliConfig {
    /// 房间关键词（可选，为空则使用配置文件中的默认值）
    pub room_keywords: Option<Vec<String>>,
    /// 数据库编号列表（可选，为空则处理所有）
    pub db_nums: Option<Vec<u32>>,
    /// 是否强制重建
    pub force_rebuild: bool,
    /// 是否详细输出
    pub verbose: bool,
}

/// 房间计算 CLI 模式
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
pub async fn room_compute_mode(
    room_keywords: Option<Vec<String>>,
    db_nums: Option<Vec<u32>>,
    force_rebuild: bool,
    verbose: bool,
    db_option_ext: &DbOptionExt,
) -> Result<()> {
    use aios_database::fast_model::{RoomBuildStats, build_room_relations};
    use std::time::Instant;

    println!("\n🏠 房间计算模式");
    println!("==========================================");

    let start_time = Instant::now();

    // 获取房间关键词
    let keywords = room_keywords.unwrap_or_else(|| db_option_ext.get_room_key_word());
    println!("   - 房间关键词: {:?}", keywords);

    if let Some(ref nums) = db_nums {
        println!("   - 数据库编号: {:?}", nums);
    } else {
        println!("   - 数据库编号: 全部");
    }
    println!("   - 强制重建: {}", force_rebuild);

    // 初始化数据库连接
    println!("\n📡 初始化数据库连接...");
    init_surreal().await?;

    // 执行房间关系构建
    println!("\n🔄 开始构建房间关系...");

    let stats = build_room_relations(&db_option_ext.inner).await?;

    let duration = start_time.elapsed();

    // 输出结果
    println!("\n🎉 房间计算完成！");
    println!("==========================================");
    println!("📊 统计信息:");
    println!("   - 处理房间数: {}", stats.total_rooms);
    println!("   - 处理面板数: {}", stats.total_panels);
    println!("   - 处理构件数: {}", stats.total_components);
    println!("   - 构建耗时: {}ms", stats.build_time_ms);
    println!("   - 缓存命中率: {:.2}%", stats.cache_hit_rate * 100.0);
    println!("   - 内存使用: {:.2}MB", stats.memory_usage_mb);
    println!("   - 总耗时: {:.2}s", duration.as_secs_f64());

    Ok(())
}

/// 房间计算 CLI 模式（无 sqlite-index 特性时的占位实现）
#[cfg(not(all(not(target_arch = "wasm32"), feature = "sqlite-index")))]
pub async fn room_compute_mode(
    _room_keywords: Option<Vec<String>>,
    _db_nums: Option<Vec<u32>>,
    _force_rebuild: bool,
    _verbose: bool,
    _db_option_ext: &DbOptionExt,
) -> Result<()> {
    Err(anyhow!(
        "房间计算需要 sqlite-index 特性，请使用 --features sqlite-index 编译"
    ))
}

/// 导出房间实例数据 CLI 模式
///
/// 导出房间计算结果为 JSON 格式：
/// - `room_relations.json`: 房间号 → 构件列表的简单映射
/// - `room_geometries.json`: 房间 AABB + 面板几何实例
pub async fn export_room_instances_mode(output_dir: Option<PathBuf>, verbose: bool) -> Result<()> {
    use aios_database::fast_model::export_model::export_room_instances::export_room_instances;

    println!("\n🏠 导出房间实例数据");
    println!("====================================");

    // 连接数据库
    println!("📡 连接数据库...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 设置输出目录
    let output_path = output_dir.unwrap_or_else(|| PathBuf::from("output/room_instances"));

    println!("📁 输出目录: {}", output_path.display());

    // 调用导出函数
    let (relations_stats, geometries_stats) = export_room_instances(&output_path, verbose).await?;

    println!("\n🎉 导出完成！");
    println!("📊 统计信息:");
    println!("   - room_relations.json:");
    println!("     - 房间数: {}", relations_stats.total_rooms);
    println!("     - 构件数: {}", relations_stats.total_components);
    println!("     - 耗时: {} ms", relations_stats.export_time_ms);
    println!("   - room_geometries.json:");
    println!("     - 房间数: {}", geometries_stats.total_rooms);
    println!("     - 面板数: {}", geometries_stats.total_panels);
    println!("     - 耗时: {} ms", geometries_stats.export_time_ms);

    Ok(())
}
