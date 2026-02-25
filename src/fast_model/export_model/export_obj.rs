use aios_core::RefnoEnum;
use aios_core::shape::pdms_shape::PlantMesh;
use anyhow::{Context, Result};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::fast_model::unit_converter::UnitConverter;
use chrono;
use std::io::BufWriter;
use std::io::Write;

use super::export_common::{ExportData, collect_export_data};
use super::model_exporter::{
    CommonExportConfig, ExportStats, ModelExporter, ObjExportConfig, collect_export_refnos,
    query_geometry_instances_ext, query_geometry_instances_ext_from_cache,
};

fn mesh_has_invalid_normals(mesh: &PlantMesh) -> bool {
    // glam::Vec3 implements is_finite; use component checks if upstream changes.
    mesh.normals.iter().any(|n| {
        !(n.x.is_finite() && n.y.is_finite() && n.z.is_finite())
            || n.length_squared().is_nan()
    })
}

/// OBJ 导出前：保证 normals 与 vertices 同步且为有限值，避免写出 `vn NaN NaN NaN`。
fn ensure_normals_sane(mesh: &mut PlantMesh) {
    use glam::Vec3;

    let vertex_count = mesh.vertices.len();
    if vertex_count == 0 {
        mesh.normals.clear();
        return;
    }

    // 若 normals 缺失或含 NaN/Inf，则重算（即便长度已对齐也要校验）。
    if mesh.normals.len() == vertex_count && !mesh_has_invalid_normals(mesh) {
        return;
    }

    // 计算几何体中心（用于法线整体翻转判定）
    let mut center = Vec3::ZERO;
    for &v in &mesh.vertices {
        center += v;
    }
    center /= vertex_count as f32;

    let mut normals = vec![Vec3::ZERO; vertex_count];
    let mut dot_sum = 0.0f32;
    let mut dot_count = 0u32;

    for tri in mesh.indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let a_idx = tri[0] as usize;
        let b_idx = tri[1] as usize;
        let c_idx = tri[2] as usize;
        if a_idx >= vertex_count || b_idx >= vertex_count || c_idx >= vertex_count {
            continue;
        }

        let a = mesh.vertices[a_idx];
        let b = mesh.vertices[b_idx];
        let c = mesh.vertices[c_idx];
        let normal = (b - a).cross(c - a);
        if normal.length_squared() > f32::EPSILON {
            normals[a_idx] += normal;
            normals[b_idx] += normal;
            normals[c_idx] += normal;

            let triangle_center = (a + b + c) / 3.0;
            let to_center = triangle_center - center;
            dot_sum += normal.dot(to_center);
            dot_count += 1;
        }
    }

    if dot_count > 0 && dot_sum < 0.0 {
        for normal in normals.iter_mut() {
            *normal = -*normal;
        }
    }

    for normal in normals.iter_mut() {
        if normal.length_squared() > f32::EPSILON {
            *normal = normal.normalize();
        } else {
            // 保持 ZERO，确保导出稳定（不会产生 NaN）。
            *normal = Vec3::ZERO;
        }
    }

    mesh.normals = normals;
}

/// 带单位转换的 OBJ 导出函数
pub(crate) fn export_mesh_to_obj_with_unit_conversion(
    mesh: &aios_core::shape::pdms_shape::PlantMesh,
    output_path: &str,
    unit_converter: &UnitConverter,
) -> Result<()> {
    if unit_converter.needs_conversion() {
        // 如果需要单位转换，创建一个转换后的 mesh
        let mut converted_mesh = mesh.clone();

        // 转换顶点坐标
        for vertex in &mut converted_mesh.vertices {
            *vertex = unit_converter.convert_vec3(vertex);
        }

        ensure_normals_sane(&mut converted_mesh);

        // 导出转换后的 mesh
        converted_mesh
            .export_obj(false, output_path)
            .context("导出 OBJ 文件失败")?;
    } else {
        // 不需要转换，也先做法线校验，避免 OBJ 中出现 NaN 法线。
        let mut mesh = mesh.clone();
        ensure_normals_sane(&mut mesh);
        mesh.export_obj(false, output_path)
            .context("导出 OBJ 文件失败")?;
    }

    Ok(())
}

fn sanitize_obj_group_name(name: &str) -> String {
    // OBJ 的 group/object 名称以空格分隔；为保证各类查看器/工具链一致性，这里做最小清洗：
    // - 用 '_' 替换 '/'（refno 常见形式为 24381/129928）
    // - 将空白字符替换为 '_'（避免被拆成多个 token）
    name.replace('/', "_")
        .replace(['\t', '\r', '\n', ' '], "_")
}

fn export_export_data_to_obj_grouped(
    export_data: &ExportData,
    output_path: &str,
    unit_converter: &UnitConverter,
    mesh_dir: &Path,
    verbose: bool,
) -> Result<()> {
    use crate::fast_model::export_model::export_common::GltfMeshCache;

    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent).context("创建输出目录失败")?;
    }

    let mut out = BufWriter::new(File::create(output_path).context("创建 OBJ 文件失败")?);
    writeln!(out, "# OBJ file exported from AIOS (grouped by refno)")?;
    writeln!(out, "# Components: {}", export_data.components.len())?;
    writeln!(out, "# Tubings: {}", export_data.tubings.len())?;
    writeln!(out, "# Total instances: {}", export_data.total_instances)?;
    writeln!(out)?;

    let mesh_cache = GltfMeshCache::new();
    let mut vertex_base: usize = 1; // OBJ 使用 1-based index

    fn write_mesh_instance<W: Write>(
        out: &mut W,
        export_data: &ExportData,
        mesh_cache: &GltfMeshCache,
        geo_hash: &str,
        transform: &glam::DMat4,
        unit_converter: &UnitConverter,
        mesh_dir: &Path,
        vertex_base: &mut usize,
    ) -> Result<(usize, usize)> {
        if !export_data.valid_geo_hashes.contains(geo_hash) {
            return Ok((0, 0));
        }

        let arc_mesh = mesh_cache.load_or_get(geo_hash, mesh_dir)?;
        let mut mesh = arc_mesh.as_ref().transform_by(transform);

        if unit_converter.needs_conversion() {
            for v in &mut mesh.vertices {
                *v = unit_converter.convert_vec3(v);
            }
        }

        // 仅用于 debug/定位：目前 OBJ 只输出 v/f（不输出 vn），避免写出 NaN normals。
        for v in &mesh.vertices {
            writeln!(out, "v {:.6} {:.6} {:.6}", v.x, v.y, v.z)?;
        }

        let mut face_cnt = 0usize;
        for tri in mesh.indices.chunks(3) {
            if tri.len() < 3 {
                continue;
            }
            let a = *vertex_base + tri[0] as usize;
            let b = *vertex_base + tri[1] as usize;
            let c = *vertex_base + tri[2] as usize;
            writeln!(out, "f {} {} {}", a, b, c)?;
            face_cnt += 1;
        }

        let v_cnt = mesh.vertices.len();
        *vertex_base = vertex_base.saturating_add(v_cnt);
        Ok((v_cnt, face_cnt))
    }

    // components：按 refno 分组（便于在 DCC/查看器中按组定位）
    for comp in &export_data.components {
        let group = sanitize_obj_group_name(&comp.refno.to_string());
        writeln!(out, "\ng {}", group)?;

        let mut wrote_any = false;
        for inst in &comp.geometries {
            let combined_transform = if comp.has_neg {
                inst.geo_transform
            } else {
                comp.world_transform * inst.geo_transform
            };

            let (v_cnt, f_cnt) = write_mesh_instance(
                &mut out,
                export_data,
                &mesh_cache,
                &inst.geo_hash,
                &combined_transform,
                unit_converter,
                mesh_dir,
                &mut vertex_base,
            )?;
            wrote_any |= v_cnt > 0 || f_cnt > 0;
        }

        if verbose && !wrote_any {
            eprintln!(
                "[export_obj] ⚠️  组内无可导出几何体（可能 mesh 缺失/被过滤）：refno={}",
                comp.refno
            );
        }
    }

    // tubings：每段一个 group，名字自带 refno
    for tubi in &export_data.tubings {
        let group = sanitize_obj_group_name(&tubi.name);
        writeln!(out, "\ng {}", group)?;
        let _ = write_mesh_instance(
            &mut out,
            export_data,
            &mesh_cache,
            &tubi.geo_hash,
            &tubi.transform,
            unit_converter,
            mesh_dir,
            &mut vertex_base,
        )?;
    }

    let _ = out.flush();
    Ok(())
}

/// OBJ 导出前的准备结果：包含汇总后的 mesh 与统计信息
#[derive(Debug, Clone)]
pub struct PreparedObjExport {
    pub mesh: PlantMesh,
    pub stats: ExportStats,
}



fn merge_export_data_into_mesh(export_data: &ExportData, mesh_dir: &Path) -> PlantMesh {
    use crate::fast_model::export_model::export_common::GltfMeshCache;
    let mesh_cache = GltfMeshCache::new();
    let mut merged_mesh = PlantMesh::default();

    // 辅助函数：合并单个实例
    let mut merge_instance = |geo_hash: &str, transform: &glam::DMat4| {
        match mesh_cache.load_or_get(geo_hash, mesh_dir) {
            Ok(arc_mesh) => {
                let transformed = arc_mesh.as_ref().transform_by(transform);
                merged_mesh.merge(&transformed);
            }
            Err(e) => {
                eprintln!(
                    "[export_obj] ⚠️ 加载 mesh {} 失败，跳过实例: {}",
                    geo_hash, e
                );
            }
        }
    };

    for component in &export_data.components {
        for instance in &component.geometries {
            // 根据 has_neg 决定变换方式：
            // - has_neg=true（booled_id）: geo_transform 已经包含世界变换(world_trans.d)，直接使用
            // - has_neg=false（普通几何体）: 需要 world_transform × geo_transform
            let combined_transform = if component.has_neg {
                // booled_id: 查询时 transform 已经是 world_trans.d
                instance.geo_transform
            } else {
                // 普通几何体: inst_transform × geo_transform
                component.world_transform * instance.geo_transform
            };

            merge_instance(&instance.geo_hash, &combined_transform);
        }
    }

    for tubing in &export_data.tubings {
        merge_instance(&tubing.geo_hash, &tubing.transform);
    }

    merged_mesh
}

/// 准备 OBJ 导出所需的数据（汇总 mesh + 统计信息）
pub async fn prepare_obj_export(
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    config: &CommonExportConfig,
) -> Result<PreparedObjExport> {
    // 统一 mesh_dir：很多调用方传的是 `assets/meshes`，但 GLB 实际存放在 `assets/meshes/lod_L{N}`。
    // 这里必须跟随当前 active_precision.default_lod，否则会误读旧的 LOD（常见表现：截图/OBJ 与最新 mesh 不一致）。 
    let default_lod = aios_core::mesh_precision::active_precision().default_lod;
    let effective_mesh_dir = match mesh_dir.file_name().and_then(|n| n.to_str()) {
        Some(name) if name.starts_with("lod_") => mesh_dir.to_path_buf(),
        _ => mesh_dir.join(format!("lod_{default_lod:?}")),
    };

    let mut stats = ExportStats::new();
    stats.refno_count = refnos.len();

    if config.verbose {
        println!("🔄 开始准备 OBJ 导出数据...");
        println!("   - 参考号数量: {}", refnos.len());
        println!("   - Mesh 目录: {}", effective_mesh_dir.display());
        if let Some(ref nouns) = config.filter_nouns {
            println!("   - 类型过滤: {:?}", nouns);
        }
        println!("   - 包含子孙节点: {}", config.include_descendants);
    }

    if refnos.is_empty() {
        if config.verbose {
            println!("⚠️  输入参考号为空，跳过导出");
        }
        return Ok(PreparedObjExport {
            mesh: PlantMesh::default(),
            stats,
        });
    }

    let all_refnos = collect_export_refnos(
        refnos,
        config.include_descendants,
        config.filter_nouns.as_deref(),
        config.verbose,
    )
    .await?;

    stats.descendant_count = all_refnos.len().saturating_sub(refnos.len());

    let geom_insts = if config.allow_surrealdb {
        query_geometry_instances_ext(&all_refnos, true, config.include_negative, config.verbose)
            .await?
    } else {
        let cache_dir = config
            .cache_dir
            .as_ref()
            .context("allow_surrealdb=false 时必须提供 CommonExportConfig.cache_dir")?;
        query_geometry_instances_ext_from_cache(
            &all_refnos,
            cache_dir,
            true,
            config.include_negative,
            config.verbose,
        )
        .await?
    };

    let export_data = collect_export_data(
        geom_insts,
        &all_refnos,
        &effective_mesh_dir,
        config.verbose,
        None,
        config.allow_surrealdb,
    )
    .await?;

    if export_data.total_instances == 0 {
        if config.verbose {
            println!("⚠️  未找到任何几何体数据");
        }
        return Ok(PreparedObjExport {
            mesh: PlantMesh::default(),
            stats,
        });
    }

    stats.mesh_files_found = export_data.loaded_count;
    stats.mesh_files_missing = export_data.failed_count;
    stats.geometry_count = export_data.total_instances;

    let merged_mesh = merge_export_data_into_mesh(&export_data, &effective_mesh_dir);

    Ok(PreparedObjExport {
        mesh: merged_mesh,
        stats,
    })
}

struct PreparedObjExportData {
    export_data: ExportData,
    effective_mesh_dir: std::path::PathBuf,
    stats: ExportStats,
}

async fn prepare_obj_export_data(
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    config: &CommonExportConfig,
) -> Result<PreparedObjExportData> {
    // 统一 mesh_dir：很多调用方传的是 `assets/meshes`，但 GLB 实际存放在 `assets/meshes/lod_L{N}`。
    let default_lod = aios_core::mesh_precision::active_precision().default_lod;
    let effective_mesh_dir = match mesh_dir.file_name().and_then(|n| n.to_str()) {
        Some(name) if name.starts_with("lod_") => mesh_dir.to_path_buf(),
        _ => mesh_dir.join(format!("lod_{default_lod:?}")),
    };

    let mut stats = ExportStats::new();
    stats.refno_count = refnos.len();

    if config.verbose {
        println!("🔄 开始准备 OBJ 导出数据...");
        println!("   - 参考号数量: {}", refnos.len());
        println!("   - Mesh 目录: {}", effective_mesh_dir.display());
        if let Some(ref nouns) = config.filter_nouns {
            println!("   - 类型过滤: {:?}", nouns);
        }
        println!("   - 包含子孙节点: {}", config.include_descendants);
    }

    if refnos.is_empty() {
        if config.verbose {
            println!("⚠️  输入参考号为空，跳过导出");
        }
        return Ok(PreparedObjExportData {
            export_data: ExportData {
                valid_geo_hashes: Default::default(),
                components: Vec::new(),
                tubings: Vec::new(),
                loaded_count: 0,
                failed_count: 0,
                total_instances: 0,
                tubi_count: 0,
                cache_hits: 0,
                cache_misses: 0,
            },
            effective_mesh_dir,
            stats,
        });
    }

    let all_refnos = collect_export_refnos(
        refnos,
        config.include_descendants,
        config.filter_nouns.as_deref(),
        config.verbose,
    )
    .await?;

    stats.descendant_count = all_refnos.len().saturating_sub(refnos.len());

    let geom_insts = if config.allow_surrealdb {
        query_geometry_instances_ext(&all_refnos, true, config.include_negative, config.verbose)
            .await?
    } else {
        let cache_dir = config
            .cache_dir
            .as_ref()
            .context("allow_surrealdb=false 时必须提供 CommonExportConfig.cache_dir")?;
        query_geometry_instances_ext_from_cache(
            &all_refnos,
            cache_dir,
            true,
            config.include_negative,
            config.verbose,
        )
        .await?
    };

    let export_data = collect_export_data(
        geom_insts,
        &all_refnos,
        &effective_mesh_dir,
        config.verbose,
        None,
        config.allow_surrealdb,
    )
    .await?;

    stats.mesh_files_found = export_data.loaded_count;
    stats.mesh_files_missing = export_data.failed_count;
    stats.geometry_count = export_data.total_instances;

    Ok(PreparedObjExportData {
        export_data,
        effective_mesh_dir,
        stats,
    })
}

/// 导出指定 refno 的整体 OBJ 模型
///
/// # 参数
///
/// * `refnos` - 要导出的参考号列表
/// * `mesh_dir` - mesh 文件目录
/// * `output_path` - 输出的 OBJ 文件路径
/// * `filter_nouns` - 可选的类型过滤器（如 ["EQUI", "PIPE"]）
/// * `include_descendants` - 是否包含子孙节点
///
/// # 返回值
///
/// 返回 `Result<()>` 表示导出是否成功
pub async fn export_obj_for_refnos(
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    output_path: &str,
    filter_nouns: Option<&[String]>,
    include_descendants: bool,
) -> Result<()> {
    println!("🔄 开始导出 OBJ 模型...");
    println!("   - 参考号数量: {}", refnos.len());
    println!("   - Mesh 目录: {}", mesh_dir.display());
    println!("   - 输出文件: {}", output_path);
    if let Some(nouns) = filter_nouns {
        println!("   - 类型过滤: {:?}", nouns);
    }
    println!("   - 包含子孙节点: {}", include_descendants);

    let filter_vec = filter_nouns.map(|n| n.to_vec());
    let common_config = CommonExportConfig {
        include_descendants,
        filter_nouns: filter_vec,
        verbose: true,
        unit_converter: UnitConverter::default(),
        use_basic_materials: false,
        include_negative: false,
        allow_surrealdb: false,
        cache_dir: Some(PathBuf::from("output/foyer_cache")),
    };

    let PreparedObjExportData {
        export_data,
        effective_mesh_dir,
        mut stats,
    } = prepare_obj_export_data(refnos, mesh_dir, &common_config).await?;

    if export_data.total_instances == 0 {
        println!("⚠️  未找到任何几何体数据");
        return Ok(());
    }

    export_export_data_to_obj_grouped(
        &export_data,
        output_path,
        &common_config.unit_converter,
        &effective_mesh_dir,
        common_config.verbose,
    )?;

    println!("✅ 导出完成: {}", output_path);
    if let Ok(metadata) = std::fs::metadata(output_path) {
        stats.output_file_size = metadata.len();
    }
    if common_config.verbose {
        stats.print_summary("OBJ");
    }

    Ok(())
}

/// OBJ 导出器
pub struct ObjExporter;

impl ObjExporter {
    /// 创建新的 OBJ 导出器
    pub fn new() -> Self {
        Self
    }
}

impl Default for ObjExporter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ModelExporter for ObjExporter {
    type Config = ObjExportConfig;
    type Stats = ExportStats;

    async fn export(
        &self,
        refnos: &[RefnoEnum],
        mesh_dir: &Path,
        output_path: &str,
        config: Self::Config,
    ) -> Result<Self::Stats> {
        let start_time = Instant::now();
        let PreparedObjExportData {
            export_data,
            effective_mesh_dir,
            mut stats,
        } = prepare_obj_export_data(refnos, mesh_dir, &config.common).await?;

        if export_data.total_instances == 0 {
            if config.common.verbose {
                println!("⚠️  未找到任何几何体数据");
            }
            stats.elapsed_time = start_time.elapsed();
            return Ok(stats);
        }

        export_export_data_to_obj_grouped(
            &export_data,
            output_path,
            &config.common.unit_converter,
            &effective_mesh_dir,
            config.common.verbose,
        )?;

        if let Ok(metadata) = std::fs::metadata(output_path) {
            stats.output_file_size = metadata.len();
        }

        stats.elapsed_time = start_time.elapsed();

        if config.common.verbose {
            stats.print_summary("OBJ");
        }

        Ok(stats)
    }

    fn file_extension(&self) -> &str {
        "obj"
    }

    fn format_name(&self) -> &str {
        "OBJ"
    }
}
