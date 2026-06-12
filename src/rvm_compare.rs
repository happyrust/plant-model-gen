//! spec 009 T004:RVM 基准 vs gen_model Parquet 导出的三层对比引擎。
//!
//! 基准侧:`--import-rvm` 落入 ModelRelationStore 的 RVM 数据(身份已解析);
//! 生成侧:站点 Parquet 导出包(instances/tubings/transforms/aabb.parquet)。
//!
//! L1 成员清单:RVM root 子树 vs instances 子树(owner 链)∪ tubings(owner=BRAN);
//! L2 类型级:RVM 推导 noun vs instances.noun(Parquet 基准下参数级降级为类型+AABB,
//!           见 spec 009 Q3 修正);
//! L3 空间级:组级合并 AABB(RVM bbox_world ∪)vs gen 实例 AABB(aabb_hash 关联),
//!           平移取 transforms 列主序矩阵第 4 列。

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RvmCompareOptions {
    pub dbnum: u32,
    pub root_refno: u64,
    pub relation_store_root: PathBuf,
    pub parquet_dir: PathBuf,
    pub report_dir: PathBuf,
    /// AABB 各分量允许偏差(mm)。
    pub tol_aabb_mm: f64,
}

#[derive(Debug, Default)]
pub struct RvmCompareSummary {
    pub rvm_members: usize,
    pub gen_members: usize,
    pub matched: usize,
    pub missing_in_gen: usize,
    pub extra_in_gen: usize,
    pub noun_mismatch: usize,
    pub aabb_mismatch: usize,
    pub aabb_compared: usize,
    pub rvm_tube_geos: usize,
    pub gen_tubi_segments: usize,
}

// ───────────────────────── RVM 侧(relation store) ─────────────────────────

#[derive(Debug)]
struct RvmMember {
    refno: u64,
    name: Option<String>,
    noun: Option<String>,
    resolved: bool,
    parent: Option<u64>,
    /// 组内全部 geometry 的 bbox_world 合并结果。
    aabb: Option<[f64; 6]>,
    geo_count: usize,
}

fn load_rvm_side(
    store_root: &Path,
    dbnum: u32,
) -> Result<HashMap<u64, RvmMember>> {
    let db_path = store_root.join(format!("{dbnum}")).join("relations.db");
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .with_context(|| format!("打开 relation store 失败: {}", db_path.display()))?;

    let mut members: HashMap<u64, RvmMember> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT refno, inst_id, parent_refno, name, noun, resolved FROM inst_relate",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
            ))
        })?;
        for row in rows {
            let (refno, inst_id, parent, name, noun, resolved) = row?;
            members.insert(
                inst_id,
                RvmMember {
                    refno,
                    name,
                    noun,
                    resolved,
                    parent,
                    aabb: None,
                    geo_count: 0,
                },
            );
        }
    }

    // 组级 AABB:geo_relate join inst_geo,按 inst_id 合并。
    {
        let mut stmt = conn.prepare(
            "SELECT g.inst_id, i.aabb_min_x, i.aabb_min_y, i.aabb_min_z,
                    i.aabb_max_x, i.aabb_max_y, i.aabb_max_z
             FROM geo_relate g JOIN inst_geo i ON i.hash = g.geo_hash",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, Option<f64>>(6)?,
            ))
        })?;
        for row in rows {
            let (inst_id, min_x, min_y, min_z, max_x, max_y, max_z) = row?;
            let Some(member) = members.get_mut(&inst_id) else {
                continue;
            };
            member.geo_count += 1;
            let (Some(ax), Some(ay), Some(az), Some(bx), Some(by), Some(bz)) =
                (min_x, min_y, min_z, max_x, max_y, max_z)
            else {
                continue;
            };
            member.aabb = Some(match member.aabb {
                None => [ax, ay, az, bx, by, bz],
                Some(prev) => [
                    prev[0].min(ax),
                    prev[1].min(ay),
                    prev[2].min(az),
                    prev[3].max(bx),
                    prev[4].max(by),
                    prev[5].max(bz),
                ],
            });
        }
    }

    Ok(members)
}

/// 从 root 出发按 parent 链取 RVM 子树(含 root 自身)。key=inst_id。
fn rvm_subtree(members: &HashMap<u64, RvmMember>, root_refno: u64) -> BTreeSet<u64> {
    // refno(解析后) == inst_id 约定;子树判断走 parent 链向上找 root。
    let mut result = BTreeSet::new();
    for (inst_id, m) in members {
        let mut cur = Some(*inst_id);
        let mut guard = 0;
        while let Some(c) = cur {
            if guard > 64 {
                break;
            }
            guard += 1;
            if members.get(&c).map(|x| x.refno) == Some(root_refno) || c == root_refno {
                result.insert(*inst_id);
                break;
            }
            cur = members.get(&c).and_then(|x| x.parent);
        }
        let _ = m;
    }
    result
}

// ───────────────────────── gen 侧(Parquet 包) ─────────────────────────

#[derive(Debug, Default, Clone)]
struct GenInstance {
    noun: String,
    owner: Option<u64>,
    aabb_hash: Option<String>,
    aabb: Option<[f64; 6]>,
}

fn read_parquet_batches(path: &Path) -> Result<Vec<arrow_array::RecordBatch>> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let file = std::fs::File::open(path)
        .with_context(|| format!("打开 Parquet 失败: {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    Ok(reader.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn col_u64(batch: &arrow_array::RecordBatch, name: &str) -> Result<arrow_array::UInt64Array> {
    use arrow_array::Array;
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("Parquet 缺少列: {name}"))?;
    Ok(arrow_array::UInt64Array::from(col.to_data()))
}

fn col_str(batch: &arrow_array::RecordBatch, name: &str) -> Result<arrow_array::StringArray> {
    use arrow_array::Array;
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("Parquet 缺少列: {name}"))?;
    Ok(arrow_array::StringArray::from(col.to_data()))
}

fn col_f64(batch: &arrow_array::RecordBatch, name: &str) -> Result<arrow_array::Float64Array> {
    use arrow_array::Array;
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("Parquet 缺少列: {name}"))?;
    Ok(arrow_array::Float64Array::from(col.to_data()))
}

fn load_gen_side(
    parquet_dir: &Path,
) -> Result<(HashMap<u64, GenInstance>, HashMap<u64, usize>)> {
    use arrow_array::Array;

    // aabb.parquet: hash -> box
    let mut aabb_by_hash: HashMap<String, [f64; 6]> = HashMap::new();
    for batch in read_parquet_batches(&parquet_dir.join("aabb.parquet"))? {
        let hash = col_str(&batch, "aabb_hash")?;
        let (min_x, min_y, min_z) = (
            col_f64(&batch, "min_x")?,
            col_f64(&batch, "min_y")?,
            col_f64(&batch, "min_z")?,
        );
        let (max_x, max_y, max_z) = (
            col_f64(&batch, "max_x")?,
            col_f64(&batch, "max_y")?,
            col_f64(&batch, "max_z")?,
        );
        for i in 0..batch.num_rows() {
            aabb_by_hash.insert(
                hash.value(i).to_string(),
                [
                    min_x.value(i),
                    min_y.value(i),
                    min_z.value(i),
                    max_x.value(i),
                    max_y.value(i),
                    max_z.value(i),
                ],
            );
        }
    }

    // instances.parquet
    let mut instances: HashMap<u64, GenInstance> = HashMap::new();
    for batch in read_parquet_batches(&parquet_dir.join("instances.parquet"))? {
        let refno = col_u64(&batch, "refno_u64")?;
        let noun = col_str(&batch, "noun")?;
        let owner = col_u64(&batch, "owner_refno_u64")?;
        let aabb_hash = col_str(&batch, "aabb_hash")?;
        for i in 0..batch.num_rows() {
            let hash = if aabb_hash.is_null(i) {
                None
            } else {
                let v = aabb_hash.value(i);
                (!v.is_empty()).then(|| v.to_string())
            };
            let aabb = hash.as_deref().and_then(|h| aabb_by_hash.get(h)).copied();
            instances.insert(
                refno.value(i),
                GenInstance {
                    noun: noun.value(i).to_string(),
                    owner: (!owner.is_null(i)).then(|| owner.value(i)),
                    aabb_hash: hash,
                    aabb,
                },
            );
        }
    }

    // tubings.parquet:owner(BRAN) -> 段数
    let mut tubi_by_owner: HashMap<u64, usize> = HashMap::new();
    let tubings_path = parquet_dir.join("tubings.parquet");
    if tubings_path.exists() {
        for batch in read_parquet_batches(&tubings_path)? {
            let owner = col_u64(&batch, "owner_refno_u64")?;
            for i in 0..batch.num_rows() {
                *tubi_by_owner.entry(owner.value(i)).or_default() += 1;
            }
        }
    }

    Ok((instances, tubi_by_owner))
}

/// gen 侧 root 子树(owner 链向上能到 root 的实例集合,含 root)。
fn gen_subtree(instances: &HashMap<u64, GenInstance>, root: u64) -> BTreeSet<u64> {
    let mut result = BTreeSet::new();
    for (refno, _) in instances {
        let mut cur = Some(*refno);
        let mut guard = 0;
        while let Some(c) = cur {
            if guard > 64 {
                break;
            }
            guard += 1;
            if c == root {
                result.insert(*refno);
                break;
            }
            cur = instances.get(&c).and_then(|x| x.owner);
        }
    }
    result
}

// ───────────────────────── 对比与报告 ─────────────────────────

pub fn compare_rvm_mode(options: &RvmCompareOptions) -> Result<()> {
    println!("\n🔍 RVM 基准对拍 (spec 009)");
    println!("==========================================");
    println!("   - dbnum: {}", options.dbnum);
    println!("   - root refno(u64): {}", options.root_refno);
    println!("   - relation store: {}", options.relation_store_root.display());
    println!("   - parquet dir: {}", options.parquet_dir.display());
    println!("   - AABB 容差: {} mm", options.tol_aabb_mm);

    let members = load_rvm_side(&options.relation_store_root, options.dbnum)?;
    let (instances, tubi_by_owner) = load_gen_side(&options.parquet_dir)?;

    let rvm_tree = rvm_subtree(&members, options.root_refno);
    let gen_tree = gen_subtree(&instances, options.root_refno);

    let mut summary = RvmCompareSummary::default();
    let mut items: Vec<serde_json::Value> = Vec::new();

    // L1+L2+L3:逐 RVM 成员对比。
    for inst_id in &rvm_tree {
        let m = &members[inst_id];
        // 未解析成员无法按 refno join,单独记账。
        if !m.resolved {
            items.push(json!({
                "refno": m.refno,
                "name": m.name,
                "status": "unresolved_identity",
            }));
            continue;
        }
        summary.rvm_members += 1;

        // RVM 侧零几何成员(如 GASKET)豁免 missing 判定,但保留记录。
        let gen_inst = instances.get(&m.refno);
        match gen_inst {
            None => {
                if m.geo_count == 0 {
                    items.push(json!({
                        "refno": m.refno,
                        "name": m.name,
                        "status": "rvm_zero_geo_not_in_gen",
                        "exempted": true,
                    }));
                } else {
                    summary.missing_in_gen += 1;
                    items.push(json!({
                        "refno": m.refno,
                        "name": m.name,
                        "rvm_geos": m.geo_count,
                        "status": "missing_in_gen",
                    }));
                }
            }
            Some(inst) => {
                summary.matched += 1;
                let mut item = json!({
                    "refno": m.refno,
                    "name": m.name,
                    "status": "matched",
                });
                // L2:noun
                if let Some(rvm_noun) = &m.noun {
                    if rvm_noun != &inst.noun {
                        summary.noun_mismatch += 1;
                        item["noun_mismatch"] = json!({
                            "rvm": rvm_noun,
                            "gen": inst.noun,
                        });
                    }
                }
                // L3:AABB
                if let (Some(ra), Some(ga)) = (m.aabb, inst.aabb) {
                    summary.aabb_compared += 1;
                    let max_delta = ra
                        .iter()
                        .zip(ga.iter())
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0_f64, f64::max);
                    if max_delta > options.tol_aabb_mm {
                        summary.aabb_mismatch += 1;
                        item["aabb_mismatch"] = json!({
                            "rvm": ra,
                            "gen": ga,
                            "max_delta_mm": max_delta,
                        });
                    } else {
                        item["aabb_max_delta_mm"] = json!(max_delta);
                    }
                }
                items.push(item);
            }
        }
    }

    summary.gen_members = gen_tree.len();

    // L1 反向:gen 子树中 RVM 没有的实例(extra)。
    let rvm_refnos: BTreeSet<u64> = rvm_tree
        .iter()
        .filter_map(|id| {
            let m = &members[id];
            m.resolved.then_some(m.refno)
        })
        .collect();
    for refno in &gen_tree {
        if !rvm_refnos.contains(refno) {
            summary.extra_in_gen += 1;
            let noun = instances
                .get(refno)
                .map(|i| i.noun.clone())
                .unwrap_or_default();
            items.push(json!({
                "refno": refno,
                "noun": noun,
                "status": "extra_in_gen",
            }));
        }
    }

    // TUBE 段对齐:root(BRAN)的 RVM 自身几何数 vs tubings 段数。
    if let Some(root_member) = members
        .values()
        .find(|m| m.resolved && m.refno == options.root_refno)
    {
        summary.rvm_tube_geos = root_member.geo_count;
    }
    summary.gen_tubi_segments = tubi_by_owner
        .get(&options.root_refno)
        .copied()
        .unwrap_or(0);

    // 报告输出。
    let report = json!({
        "version": 1,
        "dbnum": options.dbnum,
        "root_refno": options.root_refno,
        "tolerances": { "aabb_mm": options.tol_aabb_mm },
        "summary": {
            "rvm_members": summary.rvm_members,
            "gen_members": summary.gen_members,
            "matched": summary.matched,
            "missing_in_gen": summary.missing_in_gen,
            "extra_in_gen": summary.extra_in_gen,
            "noun_mismatch": summary.noun_mismatch,
            "aabb_compared": summary.aabb_compared,
            "aabb_mismatch": summary.aabb_mismatch,
            "rvm_tube_geos_on_root": summary.rvm_tube_geos,
            "gen_tubi_segments_on_root": summary.gen_tubi_segments,
        },
        "items": items,
    });

    std::fs::create_dir_all(&options.report_dir)?;
    let report_path = options.report_dir.join(format!(
        "rvm-compare-{}-{}.json",
        options.root_refno,
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;

    println!("\n📊 对拍摘要:");
    println!("   - RVM 已解析成员: {}", summary.rvm_members);
    println!("   - gen 子树成员: {}", summary.gen_members);
    println!("   - matched: {}", summary.matched);
    println!("   - missing_in_gen: {}", summary.missing_in_gen);
    println!("   - extra_in_gen: {}", summary.extra_in_gen);
    println!("   - noun_mismatch: {}", summary.noun_mismatch);
    println!(
        "   - aabb: compared={} mismatch={} (tol={}mm)",
        summary.aabb_compared, summary.aabb_mismatch, options.tol_aabb_mm
    );
    println!(
        "   - TUBE: rvm_root_geos={} gen_tubi_segments={}",
        summary.rvm_tube_geos, summary.gen_tubi_segments
    );
    println!("📄 报告: {}", report_path.display());

    let has_diff = summary.missing_in_gen > 0
        || summary.extra_in_gen > 0
        || summary.noun_mismatch > 0
        || summary.aabb_mismatch > 0;
    if has_diff {
        // CI 语义:有差异以错误码退出(退出码 1 由 main 的 Err 路径承担)。
        return Err(anyhow!(
            "对拍存在差异: missing={} extra={} noun={} aabb={}(详见报告 {})",
            summary.missing_in_gen,
            summary.extra_in_gen,
            summary.noun_mismatch,
            summary.aabb_mismatch,
            report_path.display()
        ));
    }
    println!("✅ 对拍通过(容差内)");
    Ok(())
}

// 保持 BTreeMap 引用以避免未使用告警(报告 items 已排序输出足够)。
#[allow(dead_code)]
type _Unused = BTreeMap<u64, ()>;
