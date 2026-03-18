//! Instanced Bundle 导出器
//!
//! 为每个 geo_hash 生成多级 LOD (L1/L2/L3) 的 GLB 文件，
//! 并输出 JSON 清单用于 instanced-mesh 加载

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aios_core::RefnoEnum;
use aios_core::SurrealQueryExt;
use aios_core::geometry::ShapeInstancesData;
use aios_core::mesh_precision::{LodLevel, MeshPrecisionSettings, set_active_precision};
use aios_core::options::DbOption;
use aios_core::shape::pdms_shape::PlantMesh;
use anyhow::{Context, Result};
use glam::{DMat4, Vec3};
use serde::{Deserialize, Serialize, Serializer};

use super::export_common::{ComponentRecord, ExportData, TubiRecord};
use super::export_glb::export_single_mesh_to_glb;
use crate::fast_model::mesh_generate::gen_inst_meshes;

/// LOD 配置
const LOD_LEVELS: &[LodLevel] = &[LodLevel::L1, LodLevel::L2, LodLevel::L3];

/// LOD 距离配置 (单位：米)
const LOD_DISTANCES: &[f32] = &[0.0, 50.0, 200.0];

/// Manifest 清单文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstancedManifest {
    pub version: String,
    pub export_time: String,
    pub total_archetypes: usize,
    pub total_instances: usize,
    pub archetypes: Vec<ArchetypeInfo>,
}

/// Archetype 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeInfo {
    pub id: String,
    pub noun: String,
    pub material: String,
    pub lod_levels: Vec<LodLevelInfo>,
    pub instances_url: String,
    pub instance_count: usize,
}

/// LOD 级别信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LodLevelInfo {
    pub level: String,
    pub geometry_url: String,
    pub distance: f32,
}

/// 实例数据文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstancesData {
    pub geo_hash: String,
    pub instances: Vec<InstanceInfo>,
}

fn serialize_matrix_limit_precision<S>(matrix: &[f64; 16], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(16))?;
    for &num in matrix {
        let rounded = (num * 1000.0).round() / 1000.0;
        seq.serialize_element(&rounded)?;
    }
    seq.end()
}

/// 单个实例信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub refno: String,
    #[serde(serialize_with = "serialize_matrix_limit_precision")]
    pub matrix: [f64; 16],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Instanced Bundle 导出器
pub struct InstancedBundleExporter {
    db_option: Arc<DbOption>,
    verbose: bool,
}

impl InstancedBundleExporter {
    pub fn new(db_option: Arc<DbOption>, verbose: bool) -> Self {
        Self { db_option, verbose }
    }

    /// 导出 instanced bundle
    pub async fn export(
        &self,
        export_data: &ExportData,
        output_dir: &Path,
        mesh_dir: &Path,
    ) -> Result<()> {
        if self.verbose {
            println!("\n🚀 开始导出 Instanced Bundle...");
        }

        // 仅当启用 JSON 导出时创建 instances 目录
        let instances_dir = output_dir.join("instances");
        if self.db_option.export_json {
            fs::create_dir_all(&instances_dir).context("创建 instances 目录失败")?;
        }

        if self.verbose {
            println!("   ✅ 创建目录结构完成");
            if self.db_option.export_json {
                println!("   - instances: {}", instances_dir.display());
            }
        }

        // 收集所有唯一的 geo_hash
        let mut geo_hash_usage: HashMap<String, Vec<InstanceInfo>> = HashMap::new();
        let mut geo_hash_noun_map: HashMap<String, String> = HashMap::new();

        // 收集 components 的实例
        if self.verbose {
            println!("\n📦 收集元件实例数据...");
        }
        for component in &export_data.components {
            for geom_inst in &component.geometries {
                let instance = InstanceInfo {
                    refno: component.refno.to_string(),
                    matrix: geom_inst.geo_transform.to_cols_array(),
                    color: None, // 可以后续添加颜色映射
                    name: component.name.clone(),
                };

                // 检查 matrix 有效性
                if instance.matrix.iter().any(|v| !v.is_finite()) {
                    eprintln!(
                        "   ⚠️  警告: Component 实例 matrix 包含 NaN/Inf! refno={}, matrix={:?}",
                        instance.refno, instance.matrix
                    );
                }

                geo_hash_usage
                    .entry(geom_inst.geo_hash.clone())
                    .or_insert_with(Vec::new)
                    .push(instance);

                // 记录 noun 类型
                geo_hash_noun_map
                    .entry(geom_inst.geo_hash.clone())
                    .or_insert_with(|| component.noun.clone());
            }
        }

        // 收集 TUBI 的实例
        if self.verbose {
            println!("   - 收集 TUBI 管道实例数据...");
        }
        for tubi in &export_data.tubings {
            let instance = InstanceInfo {
                refno: tubi.refno.to_string(),
                matrix: tubi.transform.to_cols_array(),
                color: None,
                name: Some(tubi.name.clone()),
            };

            // 检查 matrix 有效性
            if instance.matrix.iter().any(|v| !v.is_finite()) {
                eprintln!(
                    "   ⚠️  警告: TUBI 实例 matrix 包含 NaN/Inf! refno={}, matrix={:?}",
                    instance.refno, instance.matrix
                );
            }

            geo_hash_usage
                .entry(tubi.geo_hash.clone())
                .or_insert_with(Vec::new)
                .push(instance);

            geo_hash_noun_map
                .entry(tubi.geo_hash.clone())
                .or_insert_with(|| "TUBI".to_string());
        }

        if self.verbose {
            println!("   ✅ 收集到 {} 个唯一几何体", geo_hash_usage.len());
            println!("   ✅ 总实例数: {}", export_data.total_instances);
        }

        // 为每个 geo_hash 生成 LOD 几何体并写入实例数据
        let mut archetypes = Vec::new();

        if self.verbose {
            println!("\n🔨 为每个 geo_hash 生成 LOD 几何体...");
        }

        for (geo_hash, instances) in &geo_hash_usage {
            let noun = geo_hash_noun_map
                .get(geo_hash)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            if self.verbose {
                println!(
                    "\n   处理 geo_hash: {} (noun: {}, {} 个实例)",
                    geo_hash,
                    noun,
                    instances.len()
                );
            }

            // 检查几何体是否有效
            if !export_data.valid_geo_hashes.contains(geo_hash) {
                if self.verbose {
                    eprintln!("   ⚠️  geo_hash {} 的 GLB 文件不存在，跳过", geo_hash);
                }
                continue;
            }

            // 生成 LOD 信息（不再复制文件）
            let lod_levels = self.generate_lod_geometries(geo_hash, mesh_dir).await?;

            // 写入实例数据
            if self.db_option.export_json {
                let instances_data = InstancesData {
                    geo_hash: geo_hash.clone(),
                    instances: instances.clone(),
                };

                let instances_file = instances_dir.join(format!("{}.json", geo_hash));
                let instances_json =
                    serde_json::to_string_pretty(&instances_data).context("序列化实例数据失败")?;
                fs::write(&instances_file, instances_json)
                    .with_context(|| format!("写入实例文件失败: {}", instances_file.display()))?;

                if self.verbose {
                    println!("   ✅ 写入实例文件: {}", instances_file.display());
                }
            }

            // 添加到 archetypes 列表
            archetypes.push(ArchetypeInfo {
                id: geo_hash.clone(),
                noun: noun.clone(),
                material: "default".to_string(), // 可以后续添加材质映射
                lod_levels,
                instances_url: if self.db_option.export_json {
                    format!("instances/{}.json", geo_hash)
                } else {
                    "".to_string()
                },
                instance_count: instances.len(),
            });
        }

        // 写入 manifest.json
        let manifest = InstancedManifest {
            version: "1.0".to_string(),
            export_time: chrono::Utc::now().to_rfc3339(),
            total_archetypes: archetypes.len(),
            total_instances: export_data.total_instances,
            archetypes,
        };

        let manifest_file = output_dir.join("manifest.json");
        let manifest_json =
            serde_json::to_string_pretty(&manifest).context("序列化 manifest 失败")?;
        fs::write(&manifest_file, manifest_json)
            .with_context(|| format!("写入 manifest 文件失败: {}", manifest_file.display()))?;

        if self.verbose {
            println!("\n✅ Manifest 文件写入完成: {}", manifest_file.display());
            println!("   - 总 archetype 数: {}", manifest.total_archetypes);
            println!("   - 总实例数: {}", manifest.total_instances);
        }

        println!("\n🎉 Instanced Bundle 导出完成！");
        println!("   输出目录: {}", output_dir.display());

        Ok(())
    }

    /// 获取单个 geo_hash 的 LOD 几何体信息 (不再复制)
    async fn generate_lod_geometries(
        &self,
        geo_hash: &str,
        mesh_dir: &Path,
    ) -> Result<Vec<LodLevelInfo>> {
        let mut lod_levels = Vec::new();

        // 使用传入的 mesh_dir 作为基础目录
        let base_mesh_dir = mesh_dir;

        for (lod_index, &lod_level) in LOD_LEVELS.iter().enumerate() {
            if self.verbose {
                println!("      处理 LOD {:?}...", lod_level);
            }

            // 确定源文件位置
            // 策略：优先找 lod_XX/geo_hash_lod.glb，然后找 lod_XX/geo_hash.glb
            let lod_dir = base_mesh_dir.join(format!("lod_{:?}", lod_level));

            // 1. 标准生成名: {geo_hash}_{lod}.glb
            let src_path_1 = lod_dir.join(format!("{}_{:?}.glb", geo_hash, lod_level));
            // 2. 兼容名（可能没带后缀）: {geo_hash}.glb
            let src_path_2 = lod_dir.join(format!("{}.glb", geo_hash));

            let src_path = if src_path_1.exists() {
                Some(src_path_1)
            } else if src_path_2.exists() {
                Some(src_path_2)
            } else {
                // 如果是 L1 (默认)，尝试去更上层找，或者如果其他 LOD 不存在则跳过
                if lod_level == aios_core::mesh_precision::LodLevel::L1 {
                    // 尝试在 base_mesh_dir 直接找
                    let fallback = base_mesh_dir.join(format!("{}.glb", geo_hash));
                    if fallback.exists() {
                        Some(fallback)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            // 特殊处理单位几何体，如果文件不存在，可能需要动态生成或者报错
            // 这里为了简化，如果找不到文件，且是通用ID (1,2,3)，假设前端有内置或者后续补充
            // 实际上 gen_inst_meshes 应该已经生成了它们。

            if let Some(src) = src_path {
                // 不再复制文件，直接返回相对于 meshes 目录的 URL
                // 假设前端知道如何映射这些资源（通常基础路径是 meshes_path）

                lod_levels.push(LodLevelInfo {
                    level: format!("{:?}", lod_level),
                    geometry_url: if lod_index == 0 {
                        format!("{}.glb", geo_hash)
                    } else {
                        format!("lod_{:?}/{}.glb", lod_level, geo_hash)
                    },
                    distance: LOD_DISTANCES[lod_index],
                });
            } else {
                if self.verbose {
                    // 仅对 L1 报错，其他 LOD 可能是可选的
                    if lod_level == aios_core::mesh_precision::LodLevel::L1 {
                        eprintln!(
                            "         ⚠️  未找到 LOD {:?} GLB 源文件: {}",
                            lod_level, geo_hash
                        );
                    }
                }

                // 如果是 L1 缺失，看看是否是标准几何体 (1, 2, 3) 用到的
                // 这里如果 L1 都没有，则这个 geo_hash 实际上是无法显示的。
                // 但为了不中断流程，我们跳过它。
            }
        }

        Ok(lod_levels)
    }
}

/// 为指定 refnos 导出 instanced bundle（入口函数）
/// 返回 ExportData 以便调用方进行进一步处理（如写入 Parquet）
pub async fn export_instanced_bundle_for_refnos(
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
) -> Result<super::export_common::ExportData> {
    use super::export_common::collect_export_data;
    use aios_core::query_insts;

    if verbose {
        println!("🚀 开始导出 Instanced Bundle...");
        println!("   - 参考号数量: {}", refnos.len());
        println!("   - Mesh 目录: {}", mesh_dir.display());
        println!("   - 输出目录: {}", output_dir.display());
    }

    // 自动扩展层级：获取所有子孙节点
    // 这确保了像 BRAN/ZONE 这样的容器节点能导出其下的所有组件
    let mut all_refnos = refnos.to_vec();
    if !refnos.is_empty() {
        if verbose {
            println!("   - 正在展开通过层级查询子孙节点...");
        }
        // 查询所有类型的子孙节点（层级查询统一走 indextree/TreeIndex）
        let descendants = crate::fast_model::query_provider::query_multi_descendants_with_self(
            refnos,
            &[],
            false,
        )
        .await
        .unwrap_or_default();

        if !descendants.is_empty() {
            if verbose {
                println!("   - 找到 {} 个子孙节点，合并导出列表", descendants.len());
            }
            all_refnos.extend(descendants);
            // 去重
            all_refnos.sort();
            all_refnos.dedup();
        }
    }

    // 查询几何体数据
    if verbose {
        println!("\n📊 查询几何体数据 (共 {} 个节点)...", all_refnos.len());
    }
    let geom_insts = query_insts(&all_refnos, true)
        .await
        .context("查询 inst_relate 数据失败")?;

    // --- 诊断代码：query_insts 返回后立即检查 ---
    if verbose {
        println!("   - query_insts 返回 {} 条记录", geom_insts.len());
        if geom_insts.is_empty() && !refnos.is_empty() {
            println!("🔍 [DEBUG] query_insts 返回空，直接查询数据库诊断...");
            let pe_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
            let pe_list = pe_keys.join(",");

            // 1. 检查记录是否存在（不带过滤条件）
            let count_sql = format!(
                "SELECT count() AS cnt FROM [{}]->inst_relate GROUP ALL;",
                pe_list
            );
            match aios_core::project_primary_db()
                .query_response(&count_sql)
                .await
            {
                Ok(mut resp) => {
                    if let Ok(counts) = resp.take::<Vec<serde_json::Value>>(0) {
                        println!("   - inst_relate 总记录数（无过滤）: {:?}", counts);
                    }
                }
                Err(e) => println!("   - 查询失败: {}", e),
            }

            // 2. 检查带过滤条件的记录数
            let filter_sql = format!(
                r#"SELECT count() AS cnt FROM [{}]->inst_relate
                   WHERE (SELECT VALUE world_trans
                          FROM pe_transform 
                          WHERE id = type::record('pe_transform', record::id(in)) 
                          LIMIT 1
                         )[0] != NONE 
                     AND record::exists(type::record('inst_relate_aabb', record::id(in)))
                   GROUP ALL;"#,
                pe_list
            );
            match aios_core::project_primary_db()
                .query_response(&filter_sql)
                .await
            {
                Ok(mut resp) => {
                    if let Ok(counts) = resp.take::<Vec<serde_json::Value>>(0) {
                        println!(
                            "   - inst_relate 记录数（带 aabb/trans 过滤）: {:?}",
                            counts
                        );
                    }
                }
                Err(e) => println!("   - 过滤查询失败: {}", e),
            }

            // 3. 检查记录的详细状态
            let detail_sql = format!(
                r#"SELECT 
                       in,
                       (SELECT VALUE world_trans 
                        FROM pe_transform 
                        WHERE id = type::record('pe_transform', record::id(in)) 
                        LIMIT 1
                       )[0] as pe_transform_world_trans_id,
                       (SELECT VALUE world_trans.d 
                        FROM pe_transform 
                        WHERE id = type::record('pe_transform', record::id(in)) 
                        LIMIT 1
                       )[0] as world_trans,
                       record::exists(type::record('inst_relate_aabb', record::id(in))) as has_aabb
                   FROM [{}]->inst_relate
                   LIMIT 5;"#,
                pe_list
            );
            match aios_core::project_primary_db()
                .query_response(&detail_sql)
                .await
            {
                Ok(mut resp) => {
                    if let Ok(records) = resp.take::<Vec<serde_json::Value>>(0) {
                        println!("   - 详细记录（前5条）:");
                        for (i, rec) in records.iter().enumerate() {
                            let has_aabb = rec
                                .get("has_aabb")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let has_trans = rec.get("world_trans").is_some()
                                && rec.get("world_trans") != Some(&serde_json::json!(null));
                            println!(
                                "     [{}] in={}, aabb={}, trans={}",
                                i,
                                rec.get("in").unwrap_or(&serde_json::json!("?")),
                                if has_aabb {
                                    "有值".to_string()
                                } else {
                                    "NONE".to_string()
                                },
                                if has_trans { "有值" } else { "NONE" }
                            );
                        }
                        if records.is_empty() {
                            println!(
                                "     ⚠️ 没有找到任何 inst_relate 记录！数据可能未生成或未保存到数据库。"
                            );
                        }
                    }
                }
                Err(e) => println!("   - 详细查询失败: {}", e),
            }
        }
    }
    // --- 诊断代码结束 ---

    // 筛选 BRAN/HANG 类型的 refnos 作为 bran_roots（TUBI 管道数据查询依赖）
    //
    // 约定：过滤逻辑统一走 indextree（TreeIndex），避免在 cache-only 导出路径里隐式依赖 SurrealDB。
    let mut bran_roots: Vec<RefnoEnum> = Vec::new();
    if !all_refnos.is_empty() {
        use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
        use std::collections::HashMap;

        let mut managers: HashMap<u32, TreeIndexManager> = HashMap::new();
        for &refno in &all_refnos {
            let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(refno) else {
                continue;
            };
            let manager = managers
                .entry(dbnum)
                .or_insert_with(|| TreeIndexManager::with_default_dir(vec![dbnum]));
            let Some(noun) = manager.get_noun(refno) else {
                continue;
            };
            let noun = noun.to_uppercase();
            if noun == "BRAN" || noun == "HANG" {
                bran_roots.push(refno);
            }
        }

        if verbose && !bran_roots.is_empty() {
            println!(
                "   - 找到 {} 个 BRAN/HANG 节点用于 TUBI 查询",
                bran_roots.len()
            );
        }
    }

    // 收集导出数据
    let bran_roots_ref: Option<&[RefnoEnum]> = if bran_roots.is_empty() {
        None
    } else {
        Some(&bran_roots)
    };
    let export_data = collect_export_data(
        geom_insts,
        &all_refnos,
        mesh_dir,
        verbose,
        bran_roots_ref,
        true,
    )
    .await?;

    println!(
        "🔍 [DEBUG] collect_export_data 完成: total_instances={}, components={}, tubings={}",
        export_data.total_instances,
        export_data.components.len(),
        export_data.tubings.len()
    );

    if export_data.total_instances == 0 {
        println!("⚠️  未找到任何几何体数据");

        // --- 诊断代码开始 ---
        if verbose && !refnos.is_empty() {
            println!("🔍 [DEBUG] 诊断：直接查询 inst_relate 状态...");
            let pe_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
            let pe_list = pe_keys.join(",");

            // 1. 检查记录是否存在
            let count_sql = format!(
                "SELECT count() AS cnt FROM [{}]->inst_relate GROUP ALL;",
                pe_list
            );
            match aios_core::project_primary_db()
                .query_response(&count_sql)
                .await
            {
                Ok(mut resp) => {
                    if let Ok(counts) = resp.take::<Vec<serde_json::Value>>(0) {
                        println!("   - inst_relate 记录数: {:?}", counts);
                    }
                }
                Err(e) => println!("   - 查询失败: {}", e),
            }

            // 2. 检查关键字段 (aabb, world_trans)
            let check_sql = format!(
                "SELECT in, aabb, world_trans, geo_type, visible FROM [{}]->inst_relate;",
                pe_list
            );
            match aios_core::project_primary_db()
                .query_response(&check_sql)
                .await
            {
                Ok(mut resp) => {
                    if let Ok(records) = resp.take::<Vec<serde_json::Value>>(0) {
                        println!("   - 详细记录检查 (前10条):");
                        for (i, rec) in records.iter().enumerate().take(10) {
                            let has_aabb = rec.get("aabb").and_then(|v| v.get("d")).is_some();
                            let has_trans =
                                rec.get("world_trans").and_then(|v| v.get("d")).is_some();
                            println!(
                                "     [{}] in={}, aabb={}, trans={}, type={}, vis={}",
                                i,
                                rec.get("in").unwrap_or(&serde_json::json!("?")),
                                if has_aabb { "OK" } else { "MISSING" },
                                if has_trans { "OK" } else { "MISSING" },
                                rec.get("geo_type").unwrap_or(&serde_json::json!("?")),
                                rec.get("visible").unwrap_or(&serde_json::json!("?"))
                            );
                        }
                    }
                }
                Err(e) => println!("   - 详细查询失败: {}", e),
            }
        }
        // --- 诊断代码结束 ---

        return Ok(export_data);
    }

    if export_data.valid_geo_hashes.is_empty() {
        println!("⚠️  没有可导出的几何体");
        return Ok(export_data);
    }

    // 创建导出器并导出
    let exporter = InstancedBundleExporter::new(db_option, verbose);
    exporter.export(&export_data, output_dir, mesh_dir).await?;

    Ok(export_data)
}
