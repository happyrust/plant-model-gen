//! PDMS Tree Parquet 导出
//!
//! 目的：
//! - 为前端提供”完全离线”的模型树查询数据源
//! - 覆盖 e3d tree API 的核心字段：refno / owner / noun / name / children_count / dbnum
//!
//! 输出：
//! - `pdms_tree_{dbnum}.parquet`：指定 dbnum 的树节点表（来源：TreeIndex + SurrealDB pe.name）
//! - `world_sites.parquet`：WORL -> SITE 节点列表（来源：MDB 元信息）

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use aios_core::pdms_types::{RefU64, RefnoEnum};
use aios_core::tool::db_tool::db1_dehash;
use aios_core::{DBType, SurrealQueryExt, project_primary_db};
use anyhow::{Context, Result};
use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use chrono::{SecondsFormat, Utc};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::fast_model::gen_model::tree_index_manager::{
    TreeIndexManager, load_index_with_large_stack,
};

#[derive(Debug, Clone, Serialize)]
pub struct PdmsTreeParquetStats {
    pub dbnum: u32,
    pub node_count: usize,
    pub total_bytes: u64,
    pub generated_at: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldSitesParquetStats {
    pub world_refno: RefnoEnum,
    pub site_count: usize,
    pub total_bytes: u64,
    pub generated_at: String,
    pub file_name: String,
}

#[derive(Debug, Clone)]
struct SiteLite {
    refno: RefnoEnum,
    noun: String,
    name: String,
    children_count: u32,
    dbnum: u32,
}

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).unwrap(),
        ))
        .build()
}

fn write_parquet(path: &Path, batch: &RecordBatch) -> Result<u64> {
    let file = fs::File::create(path)
        .with_context(|| format!("创建 Parquet 文件失败: {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(writer_props()))?;
    writer.write(batch)?;
    writer.close()?;
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(size)
}

fn pdms_tree_schema() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("parent", DataType::Int64, true),
        Field::new("refno_str", DataType::Utf8, false),
        Field::new("owner_refno_str", DataType::Utf8, true),
        Field::new("noun", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("children_count", DataType::UInt32, false),
        Field::new("dbnum", DataType::UInt32, false),
    ])
}

fn world_sites_schema() -> Schema {
    Schema::new(vec![
        Field::new("world_id", DataType::Int64, false),
        Field::new("world_refno_str", DataType::Utf8, false),
        Field::new("site_id", DataType::Int64, false),
        Field::new("site_refno_str", DataType::Utf8, false),
        Field::new("site_noun", DataType::Utf8, false),
        Field::new("site_name", DataType::Utf8, false),
        Field::new("children_count", DataType::UInt32, false),
        Field::new("dbnum", DataType::UInt32, false),
    ])
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeNameRow {
    pub refno: Option<RefnoEnum>,
    pub name: Option<String>,
}

async fn query_pe_names(refnos: &[RefnoEnum], verbose: bool) -> Result<HashMap<RefnoEnum, String>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    const BATCH_SIZE: usize = 800;
    let mut out: HashMap<RefnoEnum, String> = HashMap::with_capacity(refnos.len());

    for (idx, chunk) in refnos.chunks(BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 pe.name 分批 {}/{} (批大小 {})",
                idx + 1,
                (refnos.len() + BATCH_SIZE - 1) / BATCH_SIZE,
                chunk.len()
            );
        }

        // SurrealDB record id 形式：pe:⟨24381/104070⟩
        let pe_list = chunk
            .iter()
            .map(|r| format!("pe:⟨{}⟩", r.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        // NOTE: 这里 **不要** 用 fn::default_name(id)
        // - default_name 会调用 fn::order / pe_owner 图遍历，批量导出时开销非常大（会导致导出耗时数十分钟甚至更久）
        // - 直接取 pe.name：对“有真实命名”的节点（如 BRAN/PIPE/EQUI/SITE 等）足够用于按名称搜索
        // - name 为空时，前端仍可按 refno/noun 搜索
        // 性能：用 `FROM [pe:..., pe:...]` 直接批量取记录，比 `FROM pe WHERE id IN [...]` 更快、更稳定。
        let sql = format!(
            r#"
            SELECT
              refno,
              name
            FROM [{pe_list}];
            "#
        );

        let rows: Vec<PeNameRow> = match project_primary_db().query_take(&sql, 0).await {
            Ok(v) => v,
            Err(e) => {
                if verbose {
                    println!(
                        "⚠️  查询 pe.name 失败，将使用 refno 作为 name 兜底（可能是数据库未启动）：{e}"
                    );
                }
                break;
            }
        };
        for row in rows {
            let Some(r) = row.refno else { continue };
            let name = row
                .name
                .unwrap_or_default()
                .trim()
                .trim_start_matches('/')
                .to_string();
            out.insert(r, name);
        }
    }

    Ok(out)
}

/// 导出指定 dbnum 的 PDMS 树为 Parquet。
///
/// 数据来源：
/// - 层级/owner/noun：TreeIndex (.tree)
/// - name：SurrealDB pe + fn::default_name
pub async fn export_pdms_tree_parquet(
    dbnum: u32,
    output_dir: &Path,
    verbose: bool,
) -> Result<PdmsTreeParquetStats> {
    let start = std::time::Instant::now();
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    // 1) 加载 TreeIndex
    let tree_manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let tree_dir = tree_manager.tree_dir().to_path_buf();
    let tree_path = tree_dir.join(format!("{}.tree", dbnum));
    let tree_index = load_index_with_large_stack(&tree_dir, dbnum)
        .with_context(|| format!("加载 TreeIndex 失败: {}", tree_path.display()))?;

    // 2) all refnos + children_count
    let mut all_u64s: Vec<RefU64> = tree_index.all_refnos();
    all_u64s.sort_by_key(|r| r.0);

    let mut children_count: HashMap<RefU64, u32> = HashMap::new();
    for r in &all_u64s {
        if let Some(meta) = tree_index.node_meta(*r) {
            if meta.owner.0 != 0 {
                *children_count.entry(meta.owner).or_insert(0) += 1;
            }
        }
    }

    // 3) 查询 names
    if verbose {
        println!("🔍 查询 pe.name (用于树节点显示/搜索) ...");
    }
    let all_refnos: Vec<RefnoEnum> = all_u64s.iter().copied().map(RefnoEnum::from).collect();
    let name_map = query_pe_names(&all_refnos, verbose).await?;

    // 3.5) 计算每个节点在兄弟中的顺序（用于 default_name 兜底）
    // 与 SurrealDB fn::default_name / fn::order 逻辑一致：
    // - 按 owner 分组，同一 owner 下的子节点按 id 排序
    // - order = 在兄弟中的 0-based 索引
    // - default_name = "{noun} {order + 1}"
    let mut children_by_owner: HashMap<RefU64, Vec<RefU64>> = HashMap::new();
    for r in &all_u64s {
        if let Some(meta) = tree_index.node_meta(*r) {
            if meta.owner.0 != 0 {
                children_by_owner.entry(meta.owner).or_default().push(*r);
            }
        }
    }
    // 按 id 排序以保持与 SurrealDB fn::order 一致的顺序
    for children in children_by_owner.values_mut() {
        children.sort_by_key(|r| r.0);
    }
    // 构建 refno -> 0-based order 映射
    let mut order_map: HashMap<RefU64, usize> = HashMap::new();
    for children in children_by_owner.values() {
        for (idx, r) in children.iter().enumerate() {
            order_map.insert(*r, idx);
        }
    }

    // 4) 组装列
    let mut ids: Vec<i64> = Vec::with_capacity(all_u64s.len());
    let mut parents: Vec<Option<i64>> = Vec::with_capacity(all_u64s.len());
    let mut refno_strs: Vec<String> = Vec::with_capacity(all_u64s.len());
    let mut owner_refno_strs: Vec<Option<String>> = Vec::with_capacity(all_u64s.len());
    let mut nouns: Vec<String> = Vec::with_capacity(all_u64s.len());
    let mut names: Vec<Option<String>> = Vec::with_capacity(all_u64s.len());
    let mut children_counts: Vec<u32> = Vec::with_capacity(all_u64s.len());
    let mut dbnums: Vec<u32> = Vec::with_capacity(all_u64s.len());

    for ref_u64 in &all_u64s {
        let refno = RefnoEnum::from(*ref_u64);
        let meta = tree_index.node_meta(*ref_u64);
        let (owner_u64, noun_hash) = meta
            .as_ref()
            .map(|m| (m.owner, m.noun))
            .unwrap_or((RefU64(0), 0));

        ids.push(ref_u64.0 as i64);
        parents.push(if owner_u64.0 == 0 {
            None
        } else {
            Some(owner_u64.0 as i64)
        });
        refno_strs.push(refno.to_string());
        owner_refno_strs.push(if owner_u64.0 == 0 {
            None
        } else {
            Some(RefnoEnum::from(owner_u64).to_string())
        });
        let noun = db1_dehash(noun_hash);
        let mut name = name_map.get(&refno).cloned().unwrap_or_default();
        if name.trim().is_empty() {
            // 与 fn::default_name 一致：name 为空时生成 "{noun} {order+1}"
            let order = order_map.get(ref_u64).copied().unwrap_or(0);
            name = format!("{} {}", noun, order + 1);
        }
        nouns.push(noun);
        names.push(Some(name));
        children_counts.push(*children_count.get(ref_u64).unwrap_or(&0));
        dbnums.push(dbnum);
    }

    let schema = Arc::new(pdms_tree_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)) as ArrayRef,
            Arc::new(Int64Array::from(parents)) as ArrayRef,
            Arc::new(StringArray::from(refno_strs)) as ArrayRef,
            Arc::new(StringArray::from(owner_refno_strs)) as ArrayRef,
            Arc::new(StringArray::from(nouns)) as ArrayRef,
            Arc::new(StringArray::from(names)) as ArrayRef,
            Arc::new(UInt32Array::from(children_counts)) as ArrayRef,
            Arc::new(UInt32Array::from(dbnums)) as ArrayRef,
        ],
    )?;

    // 5) 写文件
    let file_name = format!("pdms_tree_{dbnum}.parquet");
    let file_path = output_dir.join(&file_name);
    let total_bytes = write_parquet(&file_path, &batch)?;

    // 写 manifest（便于前端快速探测/调试）
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let manifest = serde_json::json!({
        "version": 1,
        "format": "parquet",
        "generated_at": generated_at,
        "dbnum": dbnum,
        "tables": {
            "pdms_tree": { "file": file_name, "rows": all_u64s.len() }
        },
        "total_bytes": total_bytes,
    });
    let manifest_name = format!("manifest_pdms_tree_{dbnum}.json");
    let manifest_path = output_dir.join(&manifest_name);
    let _ = fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?);

    if verbose {
        println!(
            "✅ pdms_tree 导出完成: {} (rows={}, bytes={}, elapsed={:?})",
            file_path.display(),
            all_u64s.len(),
            total_bytes,
            start.elapsed()
        );
    }

    Ok(PdmsTreeParquetStats {
        dbnum,
        node_count: all_u64s.len(),
        total_bytes,
        generated_at,
        file_name,
    })
}

/// 导出 WORL -> SITE 节点列表为 Parquet。
///
/// 说明：后端 e3d children 对 WORL 有特判（直接返回 sites 列表），这里把同样的数据落盘，
/// 让前端在 Full Parquet Mode 下无需依赖 /api/e3d/*。
pub async fn export_world_sites_parquet(
    output_dir: &Path,
    verbose: bool,
) -> Result<WorldSitesParquetStats> {
    let start = std::time::Instant::now();
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    // 优先使用 MDB 查询（需要 SurrealDB）；若数据库不可用则回退到“离线 tree 扫描”。
    let (world, sites, dbnums_for_sites): (
        RefnoEnum,
        Vec<aios_core::pdms_types::EleTreeNode>,
        Vec<u32>,
    ) = match async {
        let db_option = aios_core::get_db_option();
        let mdb_name = db_option.mdb_name.clone();
        let world_u64 = aios_core::mdb::get_world_refno(mdb_name.clone())
            .await?
            .refno();
        let world = RefnoEnum::from(world_u64);

        // MDB 的 WORL -> SITE 列表（DESI）
        let sites = aios_core::get_mdb_world_site_ele_nodes(mdb_name, DBType::DESI).await?;

        // 计算每个 SITE 的 dbnum（用 ref0->dbnum 映射；不要求数据库在线）
        let mut dbnums_for_sites: Vec<u32> = Vec::with_capacity(sites.len());
        for ele in &sites {
            let site_refno = ele.refno;
            let dbnum = TreeIndexManager::resolve_dbnum_for_refno(site_refno).unwrap_or(0);
            dbnums_for_sites.push(dbnum);
        }

        Ok::<_, anyhow::Error>((world, sites, dbnums_for_sites))
    }
    .await
    {
        Ok(v) => v,
        Err(e) => {
            if verbose {
                println!("⚠️  WORL->SITE 数据库查询失败，回退到离线 tree 扫描模式：{e}");
            }

            // 离线模式：
            // - world：使用 0/0 作为合成根节点
            // - sites：扫描所有 *.tree，找 noun=SITE 且 owner_noun=WORL 的节点
            let manager = TreeIndexManager::with_default_dir(vec![]);
            let tree_dir = manager.tree_dir().to_path_buf();

            let mut dbnums: Vec<u32> = Vec::new();
            if tree_dir.is_dir() {
                for entry in fs::read_dir(&tree_dir)
                    .with_context(|| format!("读取 tree_dir 失败: {}", tree_dir.display()))?
                {
                    let entry = entry?;
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if let Some(stem) = name.strip_suffix(".tree") {
                        if let Ok(n) = stem.parse::<u32>() {
                            dbnums.push(n);
                        }
                    }
                }
            }
            dbnums.sort_unstable();
            dbnums.dedup();

            let mut sites_lite: Vec<SiteLite> = Vec::new();
            for dbnum in dbnums.iter().copied() {
                let tree_path = tree_dir.join(format!("{dbnum}.tree"));
                if !tree_path.is_file() {
                    continue;
                }
                let idx = load_index_with_large_stack(&tree_dir, dbnum)
                    .with_context(|| format!("加载 TreeIndex 失败: {}", tree_path.display()))?;

                let mut all_u64s = idx.all_refnos();
                all_u64s.sort_by_key(|r| r.0);

                let mut children_count: HashMap<RefU64, u32> = HashMap::new();
                for r in &all_u64s {
                    if let Some(meta) = idx.node_meta(*r) {
                        if meta.owner.0 != 0 {
                            *children_count.entry(meta.owner).or_insert(0) += 1;
                        }
                    }
                }

                for r in all_u64s {
                    let Some(meta) = idx.node_meta(r) else {
                        continue;
                    };
                    let noun = db1_dehash(meta.noun);
                    if noun != "SITE" {
                        continue;
                    }
                    // 尝试用 owner noun 过滤顶层 SITE（owner=WORL）
                    let owner_noun = idx
                        .node_meta(meta.owner)
                        .map(|m| db1_dehash(m.noun))
                        .unwrap_or_default();
                    if owner_noun != "WORL" {
                        continue;
                    }

                    let refno = RefnoEnum::from(meta.refno);
                    let cnt = *children_count.get(&meta.refno).unwrap_or(&0);
                    sites_lite.push(SiteLite {
                        refno,
                        noun: noun.to_string(),
                        name: refno.to_string(),
                        children_count: cnt,
                        dbnum,
                    });
                }
            }

            // 构造与 SPdmsElement 形状一致的输出（最小字段集合）
            // 注意：这里为了复用后续写 parquet 的逻辑，使用 SPdmsElement 的真实类型较麻烦；
            // 直接在下方写 parquet 时会改用 sites_lite。

            // 使用合成 world
            let world = RefnoEnum::from(RefU64(0));

            // 将 sites_lite 转成 SPdmsElement 需要 aios_core 结构体的构造器，这里避免依赖，
            // 改为用 sites_lite 分支写 parquet（见下方）。

            // 通过返回 Err 标记走离线分支
            return write_world_sites_offline(output_dir, verbose, start, world, sites_lite);
        }
    };

    let world_id = world.refno().0 as i64;

    // 计算每个 SITE 的 dbnum
    let mut world_ids: Vec<i64> = Vec::with_capacity(sites.len());
    let mut world_refno_strs: Vec<String> = Vec::with_capacity(sites.len());
    let mut site_ids: Vec<i64> = Vec::with_capacity(sites.len());
    let mut site_refno_strs: Vec<String> = Vec::with_capacity(sites.len());
    let mut site_nouns: Vec<String> = Vec::with_capacity(sites.len());
    let mut site_names: Vec<String> = Vec::with_capacity(sites.len());
    let mut children_counts: Vec<u32> = Vec::with_capacity(sites.len());
    let mut dbnums: Vec<u32> = Vec::with_capacity(sites.len());

    for (idx, ele) in sites.into_iter().enumerate() {
        let site_refno = ele.refno;
        let dbnum = dbnums_for_sites.get(idx).copied().unwrap_or(0);

        world_ids.push(world_id);
        world_refno_strs.push(world.to_string());
        site_ids.push(site_refno.refno().0 as i64);
        site_refno_strs.push(site_refno.to_string());
        site_nouns.push(ele.noun);
        site_names.push(ele.name);
        children_counts.push(u32::from(ele.children_count));
        dbnums.push(dbnum);
    }

    let schema = Arc::new(world_sites_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(world_ids)) as ArrayRef,
            Arc::new(StringArray::from(world_refno_strs)) as ArrayRef,
            Arc::new(Int64Array::from(site_ids)) as ArrayRef,
            Arc::new(StringArray::from(site_refno_strs)) as ArrayRef,
            Arc::new(StringArray::from(site_nouns)) as ArrayRef,
            Arc::new(StringArray::from(site_names)) as ArrayRef,
            Arc::new(UInt32Array::from(children_counts)) as ArrayRef,
            Arc::new(UInt32Array::from(dbnums)) as ArrayRef,
        ],
    )?;

    let file_name = "world_sites.parquet".to_string();
    let file_path = output_dir.join(&file_name);
    let total_bytes = write_parquet(&file_path, &batch)?;

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let manifest = serde_json::json!({
        "version": 1,
        "format": "parquet",
        "generated_at": generated_at,
        "world_refno": world.to_string(),
        "tables": {
            "world_sites": { "file": file_name, "rows": batch.num_rows() }
        },
        "total_bytes": total_bytes,
    });
    let manifest_path = output_dir.join("manifest_world_sites.json");
    let _ = fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?);

    if verbose {
        println!(
            "✅ world_sites 导出完成: {} (rows={}, bytes={}, elapsed={:?})",
            file_path.display(),
            batch.num_rows(),
            total_bytes,
            start.elapsed()
        );
    }

    Ok(WorldSitesParquetStats {
        world_refno: world,
        site_count: batch.num_rows(),
        total_bytes,
        generated_at,
        file_name,
    })
}

fn write_world_sites_offline(
    output_dir: &Path,
    verbose: bool,
    start: std::time::Instant,
    world: RefnoEnum,
    sites_lite: Vec<SiteLite>,
) -> Result<WorldSitesParquetStats> {
    let world_id = world.refno().0 as i64;

    let mut world_ids: Vec<i64> = Vec::with_capacity(sites_lite.len());
    let mut world_refno_strs: Vec<String> = Vec::with_capacity(sites_lite.len());
    let mut site_ids: Vec<i64> = Vec::with_capacity(sites_lite.len());
    let mut site_refno_strs: Vec<String> = Vec::with_capacity(sites_lite.len());
    let mut site_nouns: Vec<String> = Vec::with_capacity(sites_lite.len());
    let mut site_names: Vec<String> = Vec::with_capacity(sites_lite.len());
    let mut children_counts: Vec<u32> = Vec::with_capacity(sites_lite.len());
    let mut dbnums: Vec<u32> = Vec::with_capacity(sites_lite.len());

    for s in sites_lite {
        world_ids.push(world_id);
        world_refno_strs.push(world.to_string());
        site_ids.push(s.refno.refno().0 as i64);
        site_refno_strs.push(s.refno.to_string());
        site_nouns.push(s.noun);
        site_names.push(s.name);
        children_counts.push(s.children_count);
        dbnums.push(s.dbnum);
    }

    let schema = Arc::new(world_sites_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(world_ids)) as ArrayRef,
            Arc::new(StringArray::from(world_refno_strs)) as ArrayRef,
            Arc::new(Int64Array::from(site_ids)) as ArrayRef,
            Arc::new(StringArray::from(site_refno_strs)) as ArrayRef,
            Arc::new(StringArray::from(site_nouns)) as ArrayRef,
            Arc::new(StringArray::from(site_names)) as ArrayRef,
            Arc::new(UInt32Array::from(children_counts)) as ArrayRef,
            Arc::new(UInt32Array::from(dbnums)) as ArrayRef,
        ],
    )?;

    let file_name = "world_sites.parquet".to_string();
    let file_path = output_dir.join(&file_name);
    let total_bytes = write_parquet(&file_path, &batch)?;

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let manifest = serde_json::json!({
        "version": 1,
        "format": "parquet",
        "generated_at": generated_at,
        "world_refno": world.to_string(),
        "source": "offline_tree_scan",
        "tables": {
            "world_sites": { "file": file_name, "rows": batch.num_rows() }
        },
        "total_bytes": total_bytes,
    });
    let manifest_path = output_dir.join("manifest_world_sites.json");
    let _ = fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?);

    if verbose {
        println!(
            "✅ world_sites(offline) 导出完成: {} (rows={}, bytes={}, elapsed={:?})",
            file_path.display(),
            batch.num_rows(),
            total_bytes,
            start.elapsed()
        );
    }

    Ok(WorldSitesParquetStats {
        world_refno: world,
        site_count: batch.num_rows(),
        total_bytes,
        generated_at,
        file_name,
    })
}
