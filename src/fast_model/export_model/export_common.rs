use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::{GeomInstQuery, RefnoEnum};
use anyhow::{Context, Result, anyhow};
use aios_core::Transform;
use chrono;
use dashmap::DashMap;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use glam::{DMat4, Vec3};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use serde_json::Value as JsonValue;
use std::io::Write;
use aios_core::geometry::csg::{unit_box_mesh, unit_cylinder_mesh, unit_sphere_mesh};
use aios_core::mesh_precision::LodMeshSettings;



/// 清洗节点名称，确保符合 glTF 规范
///
/// 规则：
/// - 移除或替换非法字符（控制字符、特殊符号等）
/// - 限制长度（最大 128 字符）
/// - 处理空字符串
pub fn sanitize_node_name(name: &str) -> String {
    // 移除前后空白
    let trimmed = name.trim();

    // 去掉开头的 /
    let trimmed = trimmed.trim_start_matches('/');

    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.into()

    // 替换或移除非法字符
    // let sanitized: String = trimmed
    //     .chars()
    //     .map(|c| {
    //         match c {
    //             // 保留字母、数字、常见符号
    //             'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '.' | ' ' => c,
    //             // 中文字符保留
    //             '\u{4e00}'..='\u{9fff}' => c,
    //             // 其他字符替换为下划线
    //             _ => '_',
    //         }
    //     })
    //     .collect();

    // // 限制长度
    // if sanitized.len() > 128 {
    //     sanitized.chars().take(128).collect()
    // } else {
    //     sanitized
    // }
}

/// 简单处理节点名称：只去掉开头的斜线，保持其他字符原样
///
/// 用于名称匹配场景，不需要严格的 glTF 字符清洗
pub fn trim_leading_slash(name: &str) -> String {
    let trimmed = name.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct PrimitiveSegment {
    pub noun: String,
    pub refno: RefnoEnum,
    pub geo_hash: String,
    pub index_start: usize,
    pub index_count: usize,
    pub vertex_start: usize,
    pub vertex_count: usize,
    /// 是否为 TUBI 管道
    pub is_tubi: bool,
    /// 段索引（用于 TUBI 命名和排序）
    pub segment_index: usize,
    /// 节点名称（已清洗）
    pub name: Option<String>,
}

/// 几何体实例（元件的一个几何体）
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    pub geo_hash: String,
    pub geo_transform: DMat4, // 几何体相对于 refno 的局部变换
    pub index: usize,           // 几何体索引
    pub unit_flag: bool,        // 是否为单位 mesh
}

/// 元件记录（包含多个几何体）
#[derive(Debug, Clone)]
pub struct ComponentRecord {
    pub refno: RefnoEnum,
    pub noun: String,
    pub name: Option<String>,
    pub world_transform: DMat4,  // refno 的世界变换
    pub geometries: Vec<GeometryInstance>,
    /// inst_relate 的 owner refno（例如设备 EQUI、结构 BRAN 等）
    pub owner_refno: Option<RefnoEnum>,
    /// owner 的 noun（EQUI/BRAN/HANG/...），大写
    pub owner_noun: Option<String>,
    /// 设备类型（仅当 owner_noun = Some("EQUI") 时有意义）
    pub owner_type: Option<String>,
    /// 规格值（来自 ZONE 的 owner.spec_value）
    pub spec_value: Option<i64>,
    /// 是否使用布尔结果 mesh（booled_id 存在时为 true）
    /// true: 几何体变换直接使用 world_transform（geo_transform 已包含世界变换）
    /// false: 使用 world_transform × geo_transform
    pub has_neg: bool,
    /// 世界坐标系下的包围盒（可能为空）
    pub aabb: Option<aios_core::types::PlantAabb>,
}

/// TUBI 记录
#[derive(Debug, Clone)]
pub struct TubiRecord {
    pub refno: RefnoEnum,
    /// TUBI 所属的 BRAN/HANG（owner_refno）
    pub owner_refno: RefnoEnum,
    pub geo_hash: String,
    pub transform: DMat4,
    pub index: usize,
    pub name: String,
    /// 规格值（来自 ZONE 的 owner.spec_value）
    pub spec_value: Option<i64>,
    /// 世界坐标系下的包围盒（可能为空）
    pub aabb: Option<aios_core::types::PlantAabb>,
    /// 世界包围盒 hash（数据库 record ID）
    pub world_aabb_hash: Option<String>,
    /// 世界变换 hash（数据库 record ID）
    pub world_trans_hash: Option<String>,
}

/// 线程安全的几何体缓存
pub struct GltfMeshCache {
    cache: DashMap<String, Arc<PlantMesh>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl GltfMeshCache {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    fn standard_unit_mesh(geo_hash: &str) -> Option<PlantMesh> {
        match geo_hash {
            "1" => Some(unit_box_mesh()),
            "2" => Some(unit_cylinder_mesh(&LodMeshSettings::default(), false)),
            "3" => Some(unit_sphere_mesh()),
            _ => None,
        }
    }

    /// 加载或获取缓存的 mesh（原始局部坐标系）
    pub fn load_or_get(&self, geo_hash: &str, mesh_dir: &Path) -> Result<Arc<PlantMesh>> {
        // 检查缓存
        if let Some(cached) = self.cache.get(geo_hash) {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(cached.clone());
        }

        // 处理 TUBI 的虚拟 geo_hash（t_2 -> 2.mesh）
        let actual_geo_hash = if geo_hash.starts_with("t_") {
            &geo_hash[2..] // 去掉 "t_" 前缀
        } else {
            geo_hash
        };

        // 标准单位几何体（1/2/3）不依赖磁盘文件：
        // - cache-only 路径下，mesh_worker_cache 可能会错误地产生 `1_L?.glb` 这类“按 geo_hash 复用但尺寸不一致”的文件；
        // - 为保证导出稳定性，这里对 1/2/3 一律使用内置 unit_*_mesh。
        if let Some(mesh) = Self::standard_unit_mesh(actual_geo_hash) {
            let arc_mesh = Arc::new(mesh);
            self.cache.insert(geo_hash.to_string(), arc_mesh.clone());
            self.misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(arc_mesh);
        }

        // 尝试从目录名推断 LOD 级别
        let lod_suffix = mesh_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|dir_name| {
                if dir_name.starts_with("lod_") {
                    Some(&dir_name[4..]) // 提取 "L1", "L2", "L3" 等
                } else {
                    None
                }
            });

        // 优先尝试带 LOD 后缀的 GLB（新格式）
        let mesh_path = if let Some(lod) = lod_suffix {
            let path_with_suffix = mesh_dir.join(format!("{}_{}.glb", actual_geo_hash, lod));
            println!(
                "[Mesh加载] 尝试加载 GLB: {} (exists={})",
                path_with_suffix.display(),
                path_with_suffix.exists()
            );
            if path_with_suffix.exists() {
                path_with_suffix
            } else {
                // 回退到不带后缀的文件名（兼容旧格式）
                let fallback = mesh_dir.join(format!("{}.glb", actual_geo_hash));
                println!(
                    "[Mesh加载] 回退到 GLB: {} (exists={})",
                    fallback.display(),
                    fallback.exists()
                );
                fallback
            }
        } else {
            mesh_dir.join(format!("{}.glb", actual_geo_hash))
        };

        if !mesh_path.exists() {
            return Err(anyhow!("Mesh 文件不存在: {}", mesh_path.display()));
        }

        let mesh = crate::fast_model::export_model::import_glb::import_glb_to_mesh(&mesh_path)
            .with_context(|| format!("加载 GLB 文件失败: {}", mesh_path.display()))?;

        let arc_mesh = Arc::new(mesh);

        // 缓存时使用原始的 geo_hash（包含 t_ 前缀）
        self.cache.insert(geo_hash.to_string(), arc_mesh.clone());
        self.misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(arc_mesh)
    }

    /// 获取缓存统计信息 (缓存大小, 命中次数, 未命中次数)
    pub fn cache_stats(&self) -> (usize, usize, usize) {
        let hits = self.hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = self.misses.load(std::sync::atomic::Ordering::Relaxed);
        (self.cache.len(), hits, misses)
    }
}

/// 导出数据结果（新版：分离元件和 TUBI）
pub struct ExportData {
    /// 有效的几何体集合 (存在的 geo_hash)
    pub valid_geo_hashes: std::collections::HashSet<String>,
    /// 元件记录（按 refno 分组）
    pub components: Vec<ComponentRecord>,
    /// TUBI 记录（扁平列表）
    pub tubings: Vec<TubiRecord>,
    /// 加载成功的几何体数量
    pub loaded_count: usize,
    /// 加载失败的几何体数量
    pub failed_count: usize,
    /// 总实例数量
    pub total_instances: usize,
    /// TUBI 管道数量
    pub tubi_count: usize,
    /// 缓存命中次数
    pub cache_hits: usize,
    /// 缓存未命中次数
    pub cache_misses: usize,
}

/// 收集导出数据（分离元件和 TUBI）
pub async fn collect_export_data(
    geom_insts: Vec<GeomInstQuery>,
    _refnos: &[RefnoEnum],
    mesh_dir: &Path,
    verbose: bool,
    _bran_roots: Option<&[RefnoEnum]>,
    tubi_use_inst_world_only: bool,
) -> Result<ExportData> {
    if verbose {
        println!("   - 找到 {} 个几何体组", geom_insts.len());
    }

    if verbose {
        println!("\n🔨 收集实例信息...");
    }

        // 导出阶段不访问 SurrealDB；
        // - tubing 从几何实例里的 is_tubi 标记拆分
        // - name 只用 refno（稳定可对齐）
    let mut components: Vec<ComponentRecord> = Vec::new();
    let mut tubings: Vec<TubiRecord> = Vec::new();
    let mut tubi_refno_counters: HashMap<RefnoEnum, usize> = HashMap::new();

    for geom_inst in &geom_insts {
        let noun = "UNKNOWN".to_string();
        let name = Some(geom_inst.refno.to_string());

        let mut geometries: Vec<GeometryInstance> = Vec::new();

        for (geo_index, inst) in geom_inst.insts.iter().enumerate() {
            if inst.is_tubi {
                let world_matrix = if tubi_use_inst_world_only {
                    // SurrealDB-only：inst.geo_transform 已是 tubi 的世界矩阵，禁止与 refno world_trans 混用。
                    inst.geo_transform.to_matrix().as_dmat4()
                } else {
                    geom_inst.world_trans.to_matrix().as_dmat4()
                        * inst.geo_transform.to_matrix().as_dmat4()
                };

                let idx = tubi_refno_counters.entry(geom_inst.refno).or_insert(0);
                let seg_index = *idx;
                *idx += 1;

                let tubi_name = if seg_index == 0 {
                    format!("TUBI_{}", geom_inst.refno)
                } else {
                    format!("TUBI_{}_{}", geom_inst.refno, seg_index + 1)
                };

                tubings.push(TubiRecord {
                    refno: geom_inst.refno,
                    owner_refno: geom_inst.owner,
                    geo_hash: inst.geo_hash.clone(),
                    transform: world_matrix,
                    index: seg_index,
                    name: tubi_name,
                    spec_value: None,
                    aabb: geom_inst.world_aabb,
                    world_aabb_hash: None,
                    world_trans_hash: None,
                });
                continue;
            }

            if verbose {
                let max_scale = inst
                    .geo_transform
                    .scale
                    .x
                    .max(inst.geo_transform.scale.y)
                    .max(inst.geo_transform.scale.z);
                if max_scale > 100000.0 {
                    println!("       ⚠️  警告:scale 异常大!");
                }
            }

            geometries.push(GeometryInstance {
                geo_hash: inst.geo_hash.clone(),
                geo_transform: inst.geo_transform.to_matrix().as_dmat4(),
                index: geo_index,
                unit_flag: inst.unit_flag,
            });
        }

        if !geometries.is_empty() {
            if verbose {
                println!("   - comp[{}] AABB: {:?}", components.len(), geom_inst.world_aabb);
            }
            components.push(ComponentRecord {
                refno: geom_inst.refno,
                noun,
                name,
                world_transform: geom_inst.world_trans.to_matrix().as_dmat4(),
                geometries,
                owner_refno: Some(geom_inst.owner),
                owner_noun: None,
                owner_type: None,
                spec_value: None,
                has_neg: geom_inst.has_neg,
                aabb: geom_inst.world_aabb,
            });
        }
    }

    // 统计每个 geo_hash 的使用次数
    let mut geo_hash_usage: HashMap<String, usize> = HashMap::new();

    // 统计元件的几何体
    for component in &components {
        for geometry in &component.geometries {
            *geo_hash_usage.entry(geometry.geo_hash.clone()).or_insert(0) += 1;
        }
    }

    // 统计 TUBI 的几何体（移除 t_ 前缀，与普通组件共享几何体）
    for tubing in &tubings {
        let clean_geo_hash = if tubing.geo_hash.starts_with("t_") {
            &tubing.geo_hash[2..] // 移除 "t_" 前缀
        } else {
            &tubing.geo_hash
        };
        *geo_hash_usage
            .entry(clean_geo_hash.to_string())
            .or_insert(0) += 1;
    }

    let total_component_instances: usize = components.iter().map(|c| c.geometries.len()).sum();
    let total_instances = total_component_instances + tubings.len();

    if verbose {
        println!("\n📦 加载唯一几何体...");
        println!("   - 唯一 geo_hash 数量: {}", geo_hash_usage.len());
        println!("   - 元件数量: {}", components.len());
        println!("   - 元件几何体实例数: {}", total_component_instances);
        println!("   - TUBI 数量: {}", tubings.len());
        println!("   - 总实例数量: {}", total_instances);
    }

    // 检查几何体文件存在性 (GLB)
    let mut valid_geo_hashes = std::collections::HashSet::new();
    let mut loaded_count: usize = 0;
    let mut failed_count: usize = 0;

    // 按使用次数排序，优先检查高频几何
    let mut sorted_geo_hashes: Vec<_> = geo_hash_usage.iter().collect();
    sorted_geo_hashes.sort_by(|a, b| b.1.cmp(a.1));

    let pb = ProgressBar::new(sorted_geo_hashes.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );
    if !verbose {
        pb.set_draw_target(ProgressDrawTarget::hidden());
    }

    // 默认检查 active_precision.default_lod 级别的 GLB（与本次 mesh 生成/布尔结果写盘保持一致）。
    // 若这里硬编码 L1，会导致“导出/截图读到旧 L1 文件”，从而看起来模型仍不对。
    let default_lod = aios_core::mesh_precision::active_precision().default_lod;
    
    // 确定搜索目录：mesh_dir/lod_L1/
    // 如果 mesh_dir 已经是 lod_XX，则直接使用，否则拼接
    let search_dir = if let Some(dir_name) = mesh_dir.file_name() {
        let dir_str = dir_name.to_string_lossy();
        if dir_str.starts_with("lod_") {
            mesh_dir.to_path_buf()
        } else {
            mesh_dir.join(format!("lod_{:?}", default_lod))
        }
    } else {
        mesh_dir.join(format!("lod_{:?}", default_lod))
    };

    if verbose {
        println!("   🔍 检查 GLB 文件 (目录: {})...", search_dir.display());
    }

    // 性能优化：预先一次性读取目录下的所有文件，避免在循环中进行密集的 exists() 调用
    let existing_files: std::collections::HashSet<std::ffi::OsString> = if search_dir.exists() {
        std::fs::read_dir(&search_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    for (geo_hash, usage_count) in &sorted_geo_hashes {
        // 构建文件名: {geo_hash}_{lod}.glb
        // 注意：gen_inst_meshes 生成的文件名格式为 {geo_hash}_{lod}.glb
        let filename = format!("{}_{:?}.glb", geo_hash, default_lod);
        
        // 同时也尝试不仅带后缀的文件名 (兼容旧数据)
        let fallback_filename = format!("{}.glb", geo_hash);

        let file_exists = existing_files.contains(std::ffi::OsStr::new(&filename));
        let fallback_exists = existing_files.contains(std::ffi::OsStr::new(&fallback_filename));
        let is_tubi_hash = geo_hash.as_str() == aios_core::prim_geo::basic::TUBI_GEO_HASH.to_string().as_str() || geo_hash.starts_with("t_");

        if file_exists || fallback_exists || is_tubi_hash {
            valid_geo_hashes.insert((*geo_hash).clone());
            loaded_count += 1;
            if verbose && **usage_count > 1 {
                pb.set_message(format!("geo_hash: {} (复用 {} 次)", geo_hash, usage_count));
            }
        } else {
            // 检查是否为标准单位几何体 (1, 2, 3)
            match geo_hash.as_str() {
                "1" | "2" | "3" => {
                    // 单位几何体总是视为有效（后续导出时动态生成或使用内置资源）
                    valid_geo_hashes.insert((*geo_hash).clone());
                    loaded_count += 1;
                }
                _ => {
                    failed_count += 1;
                }
            }
        }
        pb.inc(1);
    }

    if verbose {
        pb.finish_with_message("检查完成");
    } else {
        pb.finish_and_clear();
    }

    // cache-only：导出阶段不做“缺失 mesh 自动补生成”。缺失即跳过（避免在导出期触发 DB/解析侧依赖）。

    // 过滤掉缺失 mesh 的几何体，避免导出阶段出现缺失警告
    for component in &mut components {
        component
            .geometries
            .retain(|g| valid_geo_hashes.contains(&g.geo_hash));
    }
    components.retain(|c| !c.geometries.is_empty());
    tubings.retain(|t| valid_geo_hashes.contains(&t.geo_hash));

    let total_component_instances: usize = components.iter().map(|c| c.geometries.len()).sum();
    let total_instances = total_component_instances + tubings.len();

    // 获取缓存统计 (这里不再使用 GltfMeshCache，所以置 0)
    let cache_hits = 0;
    let cache_misses = 0;

    if verbose {
        println!("\n✅ 几何体检查完成:");
        println!("   - 有效几何体数量: {}", loaded_count);
        println!("   - 缺失: {}", failed_count);
        println!("   - 元件数量: {}", components.len());
        println!("   - 元件几何体实例数: {}", total_component_instances);
        println!("   - TUBI 数量: {}", tubings.len());
        println!("   - 总实例数量: {}", total_instances);
        if loaded_count > 0 {
            let reuse_rate = (total_instances as f32 / loaded_count as f32 - 1.0) * 100.0;
            println!("   - 几何复用率: {:.1}%", reuse_rate);
        }
    }

    let tubi_count = tubings.len();
    Ok(ExportData {
        valid_geo_hashes,
        components,
        tubings,
        loaded_count,
        failed_count,
        total_instances,
        tubi_count,
        cache_hits,
        cache_misses,
    })
}

/// 确保 mesh 有法线数据，如果缺失则计算
///
/// 修复法向量方向问题：对于封闭几何体（如box），确保法向量指向外部，
/// 避免某些面因为法向量指向内部而被背面剔除（back-face culling）导致看不见。
fn ensure_normals(mesh: &mut PlantMesh) {
    use glam::Vec3;

    let vertex_count = mesh.vertices.len();

    // 如果法线数量与顶点数量一致，则无需处理
    if mesh.normals.len() == vertex_count {
        return;
    }

    // 计算几何体的中心点（用于判断法向量方向）
    let center = if vertex_count > 0 {
        let mut sum = Vec3::ZERO;
        for &v in &mesh.vertices {
            sum += v;
        }
        sum / vertex_count as f32
    } else {
        Vec3::ZERO
    };

    // 重新计算法线
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

        // 计算三角形法向量
        let normal = (b - a).cross(c - a);
        if normal.length_squared() > f32::EPSILON {
            normals[a_idx] += normal;
            normals[b_idx] += normal;
            normals[c_idx] += normal;

            // 用整体趋势判断是否需要全局翻转法线方向
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

    // 归一化法线
    for normal in normals.iter_mut() {
        if normal.length_squared() > f32::EPSILON {
            *normal = normal.normalize();
        }
    }

    mesh.normals = normals;
}

// =============================================================================
// inst_relate 共享查询
// =============================================================================

use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

/// inst_relate 查询结果（统一结构体）
///
/// 两种导出路径（Parquet / JSON）共用。
/// `name` 和 `aabb_hash` 为可选字段，仅在 JSON 导出路径中填充。
#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
pub struct InstRelateRow {
    pub owner_refno: Option<RefnoEnum>,
    pub owner_type: Option<String>,
    pub refno: RefnoEnum,
    pub noun: Option<String>,
    pub name: Option<String>,
    pub spec_value: Option<i64>,
}

/// inst_relate_aabb 查询结果
#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
pub struct InstRelateAabbRow {
    pub refno: RefnoEnum,
    pub aabb_hash: Option<String>,
}

/// 批量查询 inst_relate（基础版，不含 aabb_hash）
///
/// 使用图遍历语法 `[{pe_list}]->inst_relate` 替代 `FROM inst_relate WHERE in IN [...]`。
/// `include_name` 为 true 时额外查询 `fn::default_full_name(in) as name`。
pub async fn query_inst_relate_batch(
    refnos: &[RefnoEnum],
    include_name: bool,
    verbose: bool,
) -> Result<Vec<InstRelateRow>> {
    use aios_core::{SurrealQueryExt, model_primary_db};

    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    const BATCH_SIZE: usize = 500;
    let mut rows = Vec::new();

    for (idx, chunk) in refnos.chunks(BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 inst_relate 分批 {}/{} (批大小 {})",
                idx + 1,
                (refnos.len() + BATCH_SIZE - 1) / BATCH_SIZE,
                chunk.len()
            );
        }

        let pe_list = chunk
            .iter()
            .map(|r| format!("pe:⟨{}⟩", r.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        let name_field = if include_name {
            "fn::default_full_name(in) as name,"
        } else {
            "NONE as name,"
        };

        let sql = format!(
            r#"
            SELECT
                owner_refno,
                owner_type,
                in as refno,
                in.noun as noun,
                {name_field}
                spec_value
            FROM [{pe_list}]->inst_relate
            "#
        );

        let mut chunk_rows: Vec<InstRelateRow> =
            model_primary_db().query_take(&sql, 0).await?;
        rows.append(&mut chunk_rows);
    }

    Ok(rows)
}

/// 批量查询 inst_relate_aabb（独立步骤）
///
/// 从 PE 节点出发图遍历 `inst_relate_aabb`，返回 refno → aabb_hash 映射。
/// 仅返回有有效 AABB 数据的记录（`out != NONE AND out.d != NONE`）。
pub async fn query_inst_relate_aabb_batch(
    refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<std::collections::HashMap<RefnoEnum, String>> {
    use aios_core::{SurrealQueryExt, model_primary_db};

    let mut map = std::collections::HashMap::new();
    if refnos.is_empty() {
        return Ok(map);
    }

    const BATCH_SIZE: usize = 500;

    for (idx, chunk) in refnos.chunks(BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 inst_relate_aabb 分批 {}/{} (批大小 {})",
                idx + 1,
                (refnos.len() + BATCH_SIZE - 1) / BATCH_SIZE,
                chunk.len()
            );
        }

        let pe_list = chunk
            .iter()
            .map(|r| format!("pe:⟨{}⟩", r.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            r#"
            SELECT
                record::id(refno) as refno,
                record::id(aabb_id) as aabb_hash
            FROM inst_relate_aabb
            WHERE refno IN [{pe_list}] AND aabb_id != NONE AND aabb_id.d != NONE
            "#
        );

        let chunk_rows: Vec<InstRelateAabbRow> =
            model_primary_db().query_take(&sql, 0).await?;

        for row in chunk_rows {
            if let Some(hash) = row.aabb_hash {
                map.insert(row.refno, hash);
            }
        }
    }

    if verbose {
        println!("   ✅ inst_relate_aabb 命中: {} 条", map.len());
    }

    Ok(map)
}
