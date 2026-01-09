use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use aios_core::SurrealQueryExt;
use aios_core::rs_surreal::query_tubi_insts_by_brans;
use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::{GeomInstQuery, RefnoEnum, SUL_DB, TubiInstQuery, get_named_attmap};
use anyhow::{Context, Result, anyhow};
use bevy_transform::components::Transform;
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

use crate::fast_model::query_provider;

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
    pub local_transform: DMat4, // 几何体相对于 refno 的局部变换
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
    /// true: 几何体变换直接使用 world_transform（local_transform 已包含世界变换）
    /// false: 使用 world_transform × local_transform
    pub has_neg: bool,
    /// 世界坐标系下的包围盒（可能为空）
    pub aabb: Option<aios_core::types::PlantAabb>,
}

/// TUBI 记录
#[derive(Debug, Clone)]
pub struct TubiRecord {
    pub refno: RefnoEnum,
    /// BRAN/HANG 所在的 owner（tubi_relate 的 leave）
    pub owner_refno: RefnoEnum,
    pub geo_hash: String,
    pub transform: DMat4,
    pub index: usize,
    pub name: String,
    /// 规格值（来自 ZONE 的 owner.spec_value）
    pub spec_value: Option<i64>,
    /// 世界坐标系下的包围盒（可能为空）
    pub aabb: Option<aios_core::types::PlantAabb>,
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
            if let Some(mesh) = Self::standard_unit_mesh(actual_geo_hash) {
                let arc_mesh = Arc::new(mesh);
                self.cache.insert(geo_hash.to_string(), arc_mesh.clone());
                self.misses
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(arc_mesh);
            }
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
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    verbose: bool,
    bran_roots: Option<&[RefnoEnum]>,
) -> Result<ExportData> {
    if verbose {
        println!("   - 找到 {} 个几何体组", geom_insts.len());
    }

    let mut total_instances: usize = geom_insts.iter().map(|g| g.insts.len()).sum();
    if verbose {
        println!("   - 总几何体实例数: {}", total_instances);
    }

    if verbose {
        println!("\n🔨 收集实例信息...");
    }

    // 记录 owner 的 noun / 设备类型（目前仅关心 EQUI）
    let mut owner_noun_map: HashMap<RefnoEnum, String> = HashMap::new();
    let mut owner_type_map: HashMap<RefnoEnum, String> = HashMap::new();

    // 先准备 owner 集合，后续用来过滤 BRAN/HANG 查询 tubi
    let mut owner_refnos: Vec<RefnoEnum> = geom_insts.iter().map(|g| g.owner).collect();
    owner_refnos.sort();
    owner_refnos.dedup();

    if !owner_refnos.is_empty() {
        let mut owner_tasks = FuturesUnordered::new();
        for owner in &owner_refnos {
            let owner_ref = *owner;
            owner_tasks.push(async move {
                let mut noun: Option<String> = None;
                let mut owner_type: Option<String> = None;

                if let Ok(Some(pe)) = query_provider::get_pe(owner_ref).await {
                    if !pe.noun.is_empty() {
                        noun = Some(pe.noun.to_uppercase());
                    }
                }

                // 对于设备 EQUI，尝试从命名属性中获取类型信息
                if matches!(noun.as_deref(), Some("EQUI")) {
                    if let Ok(attmap) = get_named_attmap(owner_ref).await {
                        // 根据现有命名习惯，尝试几个常见字段；若需要可以再细化
                        let keys = ["EQUI_TYPE", "EQUIP_TYPE", "TYPE"];
                        for key in keys {
                            if let Some(t) = attmap.get_as_string(key) {
                                if !t.is_empty() {
                                    owner_type = Some(t);
                                    break;
                                }
                            }
                        }
                    }
                }

                (owner_ref, noun, owner_type)
            });
        }

        while let Some((owner_ref, noun, owner_type)) = owner_tasks.next().await {
            if let Some(noun) = noun {
                owner_noun_map.insert(owner_ref, noun);
            }
            if let Some(owner_type) = owner_type {
                owner_type_map.insert(owner_ref, owner_type);
            }
        }
    }

    // 先为输入 refnos 预取名称/类型，便于判定 BRAN/HANG
    let mut refno_name_map: HashMap<RefnoEnum, String> = HashMap::new();
    let mut refno_noun_map: HashMap<RefnoEnum, String> = HashMap::new();

    if !refnos.is_empty() {
        let mut name_tasks = FuturesUnordered::new();
        for refno in refnos {
            let refno = *refno;
            name_tasks.push(async move {
                let mut name = None;
                let mut noun = None;

                if let Ok(Some(pe)) = query_provider::get_pe(refno).await {
                    if !pe.name.is_empty() {
                        name = Some(pe.name);
                    }
                    noun = Some(pe.noun);
                }

                if name.is_none() {
                    if let Ok(attmap) = get_named_attmap(refno).await {
                        if let Some(attr_name) = attmap.get_as_string("NAME") {
                            if !attr_name.is_empty() {
                                name = Some(attr_name);
                            }
                        }
                        if noun.is_none() {
                            noun = Some(attmap.get_type_str().to_string());
                        }
                    }
                }

                (refno, name, noun)
            });
        }

        while let Some((refno, name, noun)) = name_tasks.next().await {
            if let Some(name) = name {
                let trimmed = trim_leading_slash(&name);
                if !trimmed.is_empty() {
                    refno_name_map.insert(refno, trimmed);
                }
            }
            if let Some(noun) = noun {
                refno_noun_map.insert(refno, noun.to_uppercase());
            }
        }
    }

    // 🏗️ 分层导出架构：使用从 inst_relate 查询的 BRAN/HANG owner
    // bran_roots 已经包含真正有子节点的 BRAN/HANG，无需额外过滤
    let bran_owners: Vec<RefnoEnum> = if let Some(roots) = bran_roots {
        if verbose {
            println!(
                "   ✅ 使用 inst_relate 查询的 BRAN/HANG owner: {} 个",
                roots.len()
            );
        }
        roots.to_vec()
    } else {
        // 如果外部未提供 bran_roots，尝试从输入的 refnos 中筛选 BRAN/HANG
        let derived_roots: Vec<RefnoEnum> = refnos.iter()
            .filter(|r| {
                if let Some(noun) = refno_noun_map.get(r) {
                    noun == "BRAN" || noun == "HANG"
                } else {
                    false
                }
            })
            .cloned()
            .collect();
            
        if verbose {
            println!("   ⚠️  bran_roots 参数未提供，从 refnos 推导 BRAN/HANG: {} 个", derived_roots.len());
        }
        derived_roots
    };

    if verbose {
        println!("\n📊 查询 tubi 管道数据...");
        println!("   - 查询的 refno 数量: {}", refnos.len());
        for (i, refno) in refnos.iter().take(5).enumerate() {
            println!("   - refno[{}]: {}", i, refno);
        }
        if refnos.len() > 5 {
            println!("   - ... 还有 {} 个 refno", refnos.len() - 5);
        }
        println!("   🔍 BRAN/HANG owner 数量: {}", bran_owners.len());
    }

    // 🏗️ 分层导出架构：TUBI 查询 - 跟随 BRAN/HANG 有序生成
    // 使用从 inst_relate 查询的真正有子节点的 BRAN/HANG，提高查询效率
    let mut tubi_insts: Vec<TubiInstQuery> = Vec::new();
    if !bran_owners.is_empty() {
        const TUBI_QUERY_CHUNK: usize = 256;
        for (idx, chunk) in bran_owners.chunks(TUBI_QUERY_CHUNK.max(1)).enumerate() {
            if verbose {
                println!(
                    "   - 查询 tubi 分批 {}/{} (批大小 {})",
                    idx + 1,
                    (bran_owners.len() + TUBI_QUERY_CHUNK - 1) / TUBI_QUERY_CHUNK,
                    chunk.len()
                );
            }

            // 使用 SurrealDB ID ranges 查询 tubi_relate 表
            let mut chunk_result = Vec::new();
            for bran_refno in chunk {
                let pe_key = bran_refno.to_pe_key();
                let sql = format!(
                    r#"
                    SELECT
                        id[0] as refno,
                        in as leave,
                        id[0].owner.noun as generic,
                        aabb.d as world_aabb,
                        world_trans.d as world_trans,
                        record::id(geo) as geo_hash,
                        id[0].dt as date,
                        spec_value
                    FROM tubi_relate:[{}, 0]..[{}, ..]
                    "#,
                    pe_key, pe_key
                );

                let mut result: Vec<TubiInstQuery> =
                    SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
                chunk_result.append(&mut result);
            }

            tubi_insts.extend(chunk_result);
        }
    }

    let tubi_count = tubi_insts.len();
    if verbose {
        println!("   - 找到 {} 个 tubi 管道", tubi_count);
        if tubi_count > 0 {
            for (i, tubi) in tubi_insts.iter().take(3).enumerate() {
                println!(
                    "   - tubi[{}]: refno={}, geo_hash={}",
                    i + 1,
                    tubi.refno,
                    tubi.geo_hash
                );
            }
            if tubi_count > 3 {
                println!("   - ... 还有 {} 个 tubi", tubi_count - 3);
            }
        } else {
            println!("   ⚠️  未找到 tubi 管道数据");
        }
    }

    total_instances += tubi_count;

    // 查询所有构件的名称和 noun（包括普通构件和 TUBI）
    // 收集所有需要查询的 refno（包含 TUBI 及其 owner）
    let mut all_query_refnos: Vec<RefnoEnum> = geom_insts.iter().map(|g| g.refno).collect();
    all_query_refnos.extend(tubi_insts.iter().map(|t| t.refno));
    all_query_refnos.extend(tubi_insts.iter().map(|t| t.leave));
    if let Some(roots) = bran_roots {
        all_query_refnos.extend(roots.iter().copied());
    }
    all_query_refnos.sort();
    all_query_refnos.dedup();

    if !all_query_refnos.is_empty() {
        let mut name_tasks = FuturesUnordered::new();
        for refno in &all_query_refnos {
            let refno = *refno;
            // 已有的跳过
            if refno_name_map.contains_key(&refno) && refno_noun_map.contains_key(&refno) {
                continue;
            }
            name_tasks.push(async move {
                // 优先从 PE 获取 name
                let mut name = None;
                let mut noun = None;

                if let Ok(Some(pe)) = query_provider::get_pe(refno).await {
                    if !pe.name.is_empty() {
                        name = Some(pe.name);
                    }
                    noun = Some(pe.noun);
                }

                // 如果 PE.name 为空，尝试从 NamedAttrMap 获取 NAME 属性
                if name.is_none() {
                    if let Ok(attmap) = get_named_attmap(refno).await {
                        if let Some(attr_name) = attmap.get_as_string("NAME") {
                            if !attr_name.is_empty() {
                                name = Some(attr_name);
                            }
                        }
                        if noun.is_none() {
                            noun = Some(attmap.get_type_str().to_string());
                        }
                    }
                }

                (refno, name, noun)
            });
        }

        while let Some((refno, name, noun)) = name_tasks.next().await {
            if let Some(name) = name {
                let trimmed = trim_leading_slash(&name);
                if !trimmed.is_empty() {
                    refno_name_map.insert(refno, trimmed);
                }
            }
            if let Some(noun) = noun {
                refno_noun_map.insert(refno, noun.to_uppercase());
            }
        }
    }

    // 收集元件记录（按 refno 分组）
    let mut components: Vec<ComponentRecord> = Vec::new();

    for geom_inst in &geom_insts {
        let noun = refno_noun_map
            .get(&geom_inst.refno)
            .cloned()
            .unwrap_or_else(|| geom_inst.generic.clone().to_uppercase());

        let name = refno_name_map.get(&geom_inst.refno).cloned();

        let mut geometries = Vec::new();

        for (geo_index, inst) in geom_inst.insts.iter().enumerate() {
            if verbose {
                let max_scale = inst
                    .transform
                    .scale
                    .x
                    .max(inst.transform.scale.y)
                    .max(inst.transform.scale.z);
                if max_scale > 100000.0 {
                    println!("       ⚠️  警告:scale 异常大!");
                }
            }


            // 计算世界变换矩阵: world_trans * geo_trans
            // - Compound 类型（布尔后）: geo_trans = 单位变换（trans:⟨0⟩）
            // - Pos 类型（原始）: geo_trans = 几何体局部变换
            let world_matrix = geom_inst.world_trans.to_matrix().as_dmat4()
                * inst.transform.to_matrix().as_dmat4();

            geometries.push(GeometryInstance {
                geo_hash: inst.geo_hash.clone(),
                local_transform: inst.transform.to_matrix().as_dmat4(),  // 几何体局部变换
                index: geo_index,
                unit_flag: inst.unit_flag,
            });
        }

        if !geometries.is_empty() {
            let owner_refno = Some(geom_inst.owner);
            let owner_noun = owner_noun_map.get(&geom_inst.owner).cloned();
            let owner_type = owner_type_map.get(&geom_inst.owner).cloned();

            if verbose {
                println!("   - comp[{}] AABB: {:?}", components.len(), geom_inst.world_aabb);
            }
            components.push(ComponentRecord {
                refno: geom_inst.refno,
                noun,
                name,
                world_transform: geom_inst.world_trans.to_matrix().as_dmat4(),  // refno 世界变换
                geometries,
                owner_refno,
                owner_noun,
                owner_type,
                spec_value: geom_inst.spec_value,
                has_neg: geom_inst.has_neg,  // 是否使用布尔结果 mesh
                aabb: geom_inst.world_aabb,
            });
        }
    }

    // 收集 TUBI 记录（扁平列表）
    let mut tubings: Vec<TubiRecord> = Vec::new();
    let mut tubi_refno_counters: HashMap<RefnoEnum, usize> = HashMap::new();

    for tubi in &tubi_insts {
        if verbose {
            let max_scale = tubi
                .world_trans
                .scale
                .x
                .max(tubi.world_trans.scale.y)
                .max(tubi.world_trans.scale.z);
            if max_scale > 100000.0 {
                println!("       ⚠️  警告:scale 异常大!");
            }
        }

        let world_matrix = tubi.world_trans.to_matrix().as_dmat4();

        let tubi_index = tubi_refno_counters.entry(tubi.refno).or_insert(0);
        *tubi_index += 1;

        // TUBI 命名格式: TUBI_refno_序号
        let tubi_name = format!("TUBI_{}_{}", tubi.refno, tubi_index);

        // 移除 t_ 前缀，与普通组件共享几何体索引
        let tubi_geo_hash = if tubi.geo_hash.starts_with("t_") {
            tubi.geo_hash[2..].to_string() // 移除 "t_" 前缀
        } else {
            tubi.geo_hash.clone()
        };

        // 使用 tubi.leave 作为 owner_refno，但如果是 TUBI 自身，则使用 BRAN/HANG owner
        let owner_refno = if tubi.leave == tubi.refno {
            // 如果 leave 指向自身，说明这是一个 TUBI 节点，需要查找真正的 BRAN/HANG owner
            // 由于我们使用 SurrealDB ID ranges 查询，tubi.leave 应该指向正确的 BRAN/HANG owner
            // 但如果仍然指向自身，则使用当前 BRAN/HANG 列表中的第一个作为 owner
            bran_owners.first().copied().unwrap_or(tubi.refno)
        } else {
            tubi.leave
        };

        if verbose {
            println!("   - tubi[{}] AABB: {:?}", tubings.len(), tubi.world_aabb);
        }
        tubings.push(TubiRecord {
            refno: tubi.refno,
            owner_refno,
            geo_hash: tubi_geo_hash,
            transform: world_matrix,
            index: *tubi_index - 1,
            name: tubi_name,
            spec_value: tubi.spec_value,
            aabb: tubi.world_aabb,
        });
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
    let mut loaded_count = 0;
    let mut failed_count = 0;

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

    // 默认检查 L1 级别的 GLB
    use aios_core::mesh_precision::set_active_precision;
    use aios_core::mesh_precision::MeshPrecisionSettings;
    // 获取默认精度设置 (通常是 L1)
    let default_lod = aios_core::mesh_precision::LodLevel::L1;
    
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

        if file_exists || fallback_exists {
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
            let mut n = normal.normalize();

            // 检查法向量方向：确保指向外部
            // 计算从中心点到三角形中心的向量
            let triangle_center = (a + b + c) / 3.0;
            let to_center = triangle_center - center;

            // 如果法向量与到中心的向量方向相同（点积>0），说明法向量指向内部，需要反转
            if n.dot(to_center) > 0.0 {
                n = -n;
            }

            normals[a_idx] += n;
            normals[b_idx] += n;
            normals[c_idx] += n;
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
