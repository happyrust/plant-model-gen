use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use aios_core::rs_surreal::pe_transform::{PeTransformEntry, ensure_pe_transform_schema};
use aios_core::transform::get_local_mat4;
use aios_core::{
    RefnoEnum, SurrealQueryExt, Transform, get_children_refnos, get_named_attmap, get_type_name,
    project_primary_db,
};
use anyhow::{Context, Result};
use glam::DMat4;
use serde_json::Value;

use crate::options::{DbOptionExt, get_db_option_ext};

const PE_TRANSFORM_BATCH_SIZE: usize = 100;
const PE_TRANSFORM_PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
const PE_TRANSFORM_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn refresh_pe_transform_for_dbnums_compat(dbnums: &[u32]) -> Result<usize> {
    let db_option = get_db_option_ext();
    refresh_pe_transform_for_dbnums(dbnums, &db_option).await
}

/// 探测 pe_transform 是否已覆盖指定 dbnum（整库 count 比较）。
///
/// **仅供 L2 全库刷新路径**（`PeTransformPrecheckMode::FullDbnum` / 运维 CLI）。
/// GenPipeline 默认走 L0（session transforms）或 L1（`refresh_pe_transform_for_root_refnos`），
/// 不应再以本函数未覆盖为由触发 10 万+ 节点全库刷新。
///
/// 生产路径不能只检查根节点：如果上一次刷新在中间批次被取消，根节点
/// 可能已经存在，但后续 Parquet/模型查询仍会缺少大量 world transform。
/// 因此这里用 dbnum 下 pe 总数与 pe_transform 命中数做完整覆盖判断。
/// 表缺失 / 查询失败时返回 Err，由调用方按未覆盖处理。
pub async fn pe_transform_covers_dbnum(dbnum: u32) -> Result<bool> {
    let expected = query_total_nodes_for_dbnum(dbnum)
        .await
        .with_context(|| format!("探测 pe_transform 覆盖时统计 dbnum {} 节点失败", dbnum))?;
    if expected == 0 {
        return Ok(false);
    }

    let covered = query_pe_transform_count_for_dbnum(dbnum)
        .await
        .with_context(|| format!("探测 pe_transform 覆盖失败: dbnum={dbnum}"))?;
    Ok(covered >= expected)
}

/// 探测 pe_transform 是否已覆盖指定 dbnum 的**实例**（inst_relate 行）。
///
/// 导出只需要实例级 world_trans（`pe_transform[inst_refno].world_trans`），而模型
/// 生成阶段的专门 `persist_pe_transform` 已按实例落库。因此导出前用
/// 「pe_transform 命中数 ≥ inst_relate 实例数」判断覆盖：命中即可跳过整库 BFS 刷新；
/// 仅当未覆盖（旧库/未按新阶段生成）时才回退整库刷新。无实例时视为已覆盖
/// （导出无内容可导，无需刷新）。
///
/// 与 `pe_transform_covers_dbnum`（按全部 pe 节点比对）区别：本函数只关心导出真正
/// 需要变换的实例集，避免生成阶段已写实例后仍被整库覆盖探测判为「未覆盖」。
pub async fn pe_transform_covers_instances_for_dbnum(dbnum: u32) -> Result<bool> {
    let expected = query_inst_relate_count_for_dbnum(dbnum)
        .await
        .with_context(|| {
            format!(
                "探测 pe_transform 实例覆盖时统计 dbnum {} inst_relate 失败",
                dbnum
            )
        })?;
    if expected == 0 {
        return Ok(true);
    }

    let covered = query_pe_transform_count_for_dbnum(dbnum)
        .await
        .with_context(|| format!("探测 pe_transform 实例覆盖失败: dbnum={dbnum}"))?;
    Ok(covered >= expected)
}

async fn query_inst_relate_count_for_dbnum(dbnum: u32) -> Result<usize> {
    let sql = format!(
        "SELECT count() AS count FROM inst_relate WHERE dbnum = {} GROUP ALL",
        dbnum
    );
    let rows: Vec<Value> = project_primary_db()
        .query_take(&sql, 0)
        .await
        .with_context(|| format!("执行 inst_relate 统计 SQL 失败: {}", sql))?;

    Ok(rows
        .iter()
        .find_map(extract_count_from_json_value)
        .unwrap_or(0) as usize)
}

pub async fn refresh_pe_transform_for_dbnums(
    dbnums: &[u32],
    db_option: &DbOptionExt,
) -> Result<usize> {
    ensure_pe_transform_schema()
        .await
        .context("初始化 pe_transform schema 失败")?;

    if dbnums.is_empty() {
        println!("⚠️  未提供 dbnum 列表");
        return Ok(0);
    }

    println!("📋 刷新 dbnums: {:?}", dbnums);

    let mut entries: Vec<PeTransformEntry> = Vec::with_capacity(PE_TRANSFORM_BATCH_SIZE);
    let mut total = 0usize;
    let mut total_primed = 0usize;
    let refresh_started = Instant::now();
    let mut last_progress = Instant::now();
    let dbnums_total = dbnums.len();

    record_transform_refresh_progress(
        "refresh_pe_transform_dbnums_started",
        format!("dbnums_total={dbnums_total} dbnums={dbnums:?}"),
        refresh_started,
    );

    for (dbnum_idx, dbnum) in dbnums.iter().enumerate() {
        let total_nodes = match query_total_nodes_for_dbnum(*dbnum).await {
            Ok(count) => count,
            Err(err) => {
                eprintln!(
                    "⚠️  统计 dbnum {} 总节点数失败，将继续刷新但不显示百分比: {}",
                    dbnum, err
                );
                0
            }
        };

        println!("📊 dbnum {} 总节点数: {}", dbnum, total_nodes);
        record_transform_refresh_progress(
            "refresh_pe_transform_dbnum_started",
            format!(
                "dbnum_index={}/{} dbnum={} total_nodes={}",
                dbnum_idx + 1,
                dbnums_total,
                dbnum,
                total_nodes
            ),
            refresh_started,
        );

        let roots = query_root_refnos(*dbnum)
            .await
            .with_context(|| format!("查询 dbnum {} 根节点失败", dbnum))?;

        if roots.is_empty() {
            println!("⚠️  dbnum {} 没有找到根节点", dbnum);
            continue;
        }

        println!("🔍 处理 dbnum {}, 找到 {} 个根节点", dbnum, roots.len());

        let mut dbnum_processed = 0usize;
        let mut dbnum_last_print = 0usize;

        for root_refno in roots {
            let mut queue: VecDeque<(RefnoEnum, DMat4)> = VecDeque::new();

            let local_mat = get_sanitized_local_mat4(root_refno, refresh_started).await;
            let world_mat = local_mat.unwrap_or(DMat4::IDENTITY);
            push_entry(
                &mut entries,
                &mut total,
                root_refno,
                local_mat,
                Some(world_mat),
            );
            dbnum_processed += 1;
            queue.push_back((root_refno, world_mat));

            while let Some((parent_refno, parent_world)) = queue.pop_front() {
                let children =
                    match get_children_refnos_with_timeout(parent_refno, refresh_started).await {
                        Ok(children) => children,
                        Err(err) => {
                            eprintln!("⚠️  获取子节点失败: {} -> {}", parent_refno, err);
                            continue;
                        }
                    };

                for child in children {
                    let local_mat = get_sanitized_local_mat4(child, refresh_started).await;
                    let world_mat = match local_mat {
                        Some(local) => parent_world * local,
                        None => parent_world,
                    };
                    push_entry(&mut entries, &mut total, child, local_mat, Some(world_mat));
                    dbnum_processed += 1;
                    queue.push_back((child, world_mat));

                    if dbnum_processed - dbnum_last_print >= 10 {
                        print_progress(dbnum_processed, total_nodes, false);
                        dbnum_last_print = dbnum_processed;
                    }

                    if last_progress.elapsed() >= PE_TRANSFORM_PROGRESS_INTERVAL {
                        record_transform_refresh_progress(
                            "refresh_pe_transform_dbnum_progress",
                            format!(
                                "dbnum_index={}/{} dbnum={} processed={} dbnum_processed={} total_nodes={} primed={} pending_batch={}",
                                dbnum_idx + 1,
                                dbnums_total,
                                dbnum,
                                total,
                                dbnum_processed,
                                total_nodes,
                                total_primed,
                                entries.len()
                            ),
                            refresh_started,
                        );
                        last_progress = Instant::now();
                    }

                    if entries.len() >= PE_TRANSFORM_BATCH_SIZE {
                        flush_entries(db_option, &mut entries, &mut total_primed, refresh_started)
                            .await
                            .with_context(|| {
                                format!("批量写入 pe_transform 失败: dbnum={}", dbnum)
                            })?;
                        entries.clear();
                        print_progress(dbnum_processed, total_nodes, true);
                        dbnum_last_print = dbnum_processed;
                        record_transform_refresh_progress(
                            "refresh_pe_transform_dbnum_batch_saved",
                            format!(
                                "dbnum_index={}/{} dbnum={} processed={} dbnum_processed={} primed={}",
                                dbnum_idx + 1,
                                dbnums_total,
                                dbnum,
                                total,
                                dbnum_processed,
                                total_primed
                            ),
                            refresh_started,
                        );
                        last_progress = Instant::now();
                    }
                }
            }
        }

        println!();
    }

    if !entries.is_empty() {
        flush_entries(db_option, &mut entries, &mut total_primed, refresh_started)
            .await
            .context("写入最后一批 pe_transform 失败")?;
    }

    println!(
        "\r✅ 完成！共处理 {} 个节点，预热 transform_cache {} 个节点                    ",
        total, total_primed
    );
    record_transform_refresh_progress(
        "refresh_pe_transform_dbnums_done",
        format!("processed={total} primed={total_primed}"),
        refresh_started,
    );
    Ok(total)
}

pub async fn refresh_pe_transform_for_root_refnos_compat(
    root_refnos: &[RefnoEnum],
) -> Result<usize> {
    let db_option = get_db_option_ext();
    refresh_pe_transform_for_root_refnos(root_refnos, &db_option).await
}

/// 使 roots 及其子孙的 pe_transform 与内存 transform_cache 失效。
///
/// 增量 owner / POS / ORI 变更后调用：只清受影响子树，禁止「任一缺口 → 整库」。
/// 后续读路径靠 session transforms（L0）或 lazy miss 回写补齐。
pub async fn invalidate_pe_transform_for_root_refnos(root_refnos: &[RefnoEnum]) -> Result<usize> {
    let started = Instant::now();
    let affected = collect_subtree_refnos(root_refnos).await?;
    if affected.is_empty() {
        return Ok(0);
    }

    let cleared = crate::pe_transform_store::clear_pe_transform_for_refnos(&affected)
        .await
        .context("失效 pe_transform 子树失败")?;

    #[cfg(feature = "gen_model")]
    {
        let _ =
            crate::fast_model::gen_model::transform_cache::clear_global_transform_cache_for_refnos(
                &affected,
            );
    }

    record_transform_refresh_progress(
        "invalidate_pe_transform_subtree_done",
        format!(
            "roots={} affected={} cleared_keys={} elapsed_ms={}",
            root_refnos.len(),
            affected.len(),
            cleared,
            started.elapsed().as_millis()
        ),
        started,
    );
    println!(
        "[pe_transform] invalidate subtree: roots={} affected={} cleared_keys={}",
        root_refnos.len(),
        affected.len(),
        cleared
    );
    Ok(affected.len())
}

/// BFS 收集 roots ∪ 子孙（children 查询失败时仍保留已收集节点）。
async fn collect_subtree_refnos(root_refnos: &[RefnoEnum]) -> Result<Vec<RefnoEnum>> {
    let mut roots = root_refnos.to_vec();
    roots.sort_unstable_by_key(|r| r.to_string());
    roots.dedup();
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    for root in roots {
        if seen.insert(root) {
            queue.push_back(root);
        }
    }

    let started = Instant::now();
    while let Some(parent) = queue.pop_front() {
        match get_children_refnos_with_timeout(parent, started).await {
            Ok(children) => {
                for child in children {
                    if seen.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "⚠️  invalidate 收集子节点失败（保留已收集）: {} -> {}",
                    parent, err
                );
            }
        }
    }

    Ok(seen.into_iter().collect())
}

pub async fn refresh_pe_transform_for_root_refnos(
    root_refnos: &[RefnoEnum],
    db_option: &DbOptionExt,
) -> Result<usize> {
    ensure_pe_transform_schema()
        .await
        .context("初始化 pe_transform schema 失败")?;

    let mut roots = root_refnos.to_vec();
    roots.sort_unstable_by_key(|refno| refno.to_string());
    roots.dedup();

    if roots.is_empty() {
        println!("⚠️  未提供 root refno 列表");
        return Ok(0);
    }

    println!("📋 刷新 root_refnos: {:?}", roots);

    let mut entries: Vec<PeTransformEntry> = Vec::with_capacity(PE_TRANSFORM_BATCH_SIZE);
    let mut total = 0usize;
    let mut total_primed = 0usize;
    let refresh_started = Instant::now();
    let mut last_progress = Instant::now();
    let roots_total = roots.len();

    record_transform_refresh_progress(
        "refresh_pe_transform_started",
        format!("roots_total={roots_total}"),
        refresh_started,
    );

    for (root_idx, root_refno) in roots.into_iter().enumerate() {
        record_transform_refresh_progress(
            "refresh_pe_transform_root_started",
            format!(
                "root_index={}/{} root_refno={}",
                root_idx + 1,
                roots_total,
                root_refno
            ),
            refresh_started,
        );
        let root_local = get_sanitized_local_mat4(root_refno, refresh_started).await;
        let root_world = compute_world_mat_from_owner_chain(root_refno, refresh_started)
            .await
            .with_context(|| format!("计算 root 世界变换失败: {}", root_refno))?;
        push_entry(
            &mut entries,
            &mut total,
            root_refno,
            root_local,
            Some(root_world),
        );

        let mut queue: VecDeque<(RefnoEnum, DMat4)> = VecDeque::new();
        queue.push_back((root_refno, root_world));

        while let Some((parent_refno, parent_world)) = queue.pop_front() {
            let children =
                match get_children_refnos_with_timeout(parent_refno, refresh_started).await {
                    Ok(children) => children,
                    Err(err) => {
                        eprintln!("⚠️  获取子节点失败: {} -> {}", parent_refno, err);
                        continue;
                    }
                };

            for child in children {
                let local_mat = get_sanitized_local_mat4(child, refresh_started).await;
                let world_mat = match local_mat {
                    Some(local) => parent_world * local,
                    None => parent_world,
                };
                push_entry(&mut entries, &mut total, child, local_mat, Some(world_mat));
                queue.push_back((child, world_mat));

                if last_progress.elapsed() >= PE_TRANSFORM_PROGRESS_INTERVAL {
                    record_transform_refresh_progress(
                        "refresh_pe_transform_progress",
                        format!(
                            "root_index={}/{} root_refno={} processed={} primed={} pending_batch={}",
                            root_idx + 1,
                            roots_total,
                            root_refno,
                            total,
                            total_primed,
                            entries.len()
                        ),
                        refresh_started,
                    );
                    last_progress = Instant::now();
                }

                if entries.len() >= PE_TRANSFORM_BATCH_SIZE {
                    flush_entries(db_option, &mut entries, &mut total_primed, refresh_started)
                        .await
                        .with_context(|| {
                            format!("批量写入 pe_transform 失败: root_refno={}", root_refno)
                        })?;
                    entries.clear();
                    record_transform_refresh_progress(
                        "refresh_pe_transform_batch_saved",
                        format!(
                            "root_index={}/{} root_refno={} processed={} primed={}",
                            root_idx + 1,
                            roots_total,
                            root_refno,
                            total,
                            total_primed
                        ),
                        refresh_started,
                    );
                    last_progress = Instant::now();
                }
            }
        }
    }

    if !entries.is_empty() {
        flush_entries(db_option, &mut entries, &mut total_primed, refresh_started)
            .await
            .context("写入最后一批 pe_transform 失败")?;
    }

    record_transform_refresh_progress(
        "refresh_pe_transform_done",
        format!("processed={total} primed={total_primed}"),
        refresh_started,
    );
    println!(
        "\r✅ 子树刷新完成！共处理 {} 个节点，预热 transform_cache {} 个节点                    ",
        total, total_primed
    );
    Ok(total)
}

async fn flush_entries(
    db_option: &DbOptionExt,
    entries: &mut Vec<PeTransformEntry>,
    total_primed: &mut usize,
    refresh_started: Instant,
) -> Result<()> {
    let entry_count = entries.len();
    record_transform_refresh_progress(
        "refresh_pe_transform_flush_started",
        format!("entries={} primed_before={}", entry_count, *total_primed),
        refresh_started,
    );
    crate::pe_transform_store::save_entries_with_backend(db_option, entries).await?;
    record_transform_refresh_progress(
        "refresh_pe_transform_backend_saved",
        format!("entries={entry_count}"),
        refresh_started,
    );
    let before_prime = *total_primed;
    // 全局 transform 缓存预热属于 gen_model 网格管线；瘦构建（无 gen_model）跳过。
    #[cfg(feature = "gen_model")]
    {
        *total_primed +=
            crate::fast_model::gen_model::transform_cache::prime_global_transform_cache_from_pe_entries(
                entries,
            );
    }
    record_transform_refresh_progress(
        "refresh_pe_transform_flush_done",
        format!(
            "entries={} primed_delta={} primed_total={}",
            entry_count,
            total_primed.saturating_sub(before_prime),
            *total_primed
        ),
        refresh_started,
    );
    Ok(())
}

async fn query_total_nodes_for_dbnum(dbnum: u32) -> Result<usize> {
    let sql = format!(
        "SELECT count() AS count FROM pe WHERE dbnum = {} GROUP ALL",
        dbnum
    );
    let rows: Vec<Value> = project_primary_db()
        .query_take(&sql, 0)
        .await
        .with_context(|| format!("执行节点统计 SQL 失败: {}", sql))?;

    Ok(rows
        .iter()
        .find_map(extract_count_from_json_value)
        .unwrap_or(0) as usize)
}

async fn query_root_refnos(dbnum: u32) -> Result<Vec<RefnoEnum>> {
    let sql = format!(
        "SELECT VALUE refno FROM pe WHERE dbnum = {} AND (noun = 'SITE' OR noun = 'WORL') AND owner.refno = NONE",
        dbnum
    );
    project_primary_db()
        .query_take(&sql, 0)
        .await
        .with_context(|| format!("执行根节点查询失败: {}", sql))
}

fn push_entry(
    entries: &mut Vec<PeTransformEntry>,
    total: &mut usize,
    refno: RefnoEnum,
    local_mat: Option<DMat4>,
    world_mat: Option<DMat4>,
) {
    let local = dmat4_to_transform_option(local_mat);
    let world = dmat4_to_transform_option(world_mat);
    if local.is_none() && world.is_none() {
        return;
    }
    entries.push(PeTransformEntry {
        refno,
        local,
        world,
    });
    *total += 1;
}

fn sanitize_dmat4(matrix: Option<DMat4>) -> Option<DMat4> {
    matrix.filter(|mat| !mat.is_nan())
}

fn dmat4_to_transform_option(matrix: Option<DMat4>) -> Option<Transform> {
    sanitize_dmat4(matrix)
        .map(|mat| Transform::from_matrix(mat.as_mat4()))
        .filter(|transform| transform.is_finite())
}

async fn get_children_refnos_with_timeout(
    refno: RefnoEnum,
    refresh_started: Instant,
) -> Result<Vec<RefnoEnum>> {
    record_transform_refresh_progress(
        "refresh_pe_transform_children_query_started",
        format!(
            "refno={refno} timeout_secs={}",
            PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
        ),
        refresh_started,
    );
    match tokio::time::timeout(PE_TRANSFORM_QUERY_TIMEOUT, get_children_refnos(refno)).await {
        Ok(result) => result,
        Err(_) => {
            record_transform_refresh_progress(
                "refresh_pe_transform_children_query_timeout",
                format!(
                    "refno={refno} timeout_secs={}",
                    PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
                ),
                refresh_started,
            );
            Err(anyhow::anyhow!(
                "get_children_refnos timed out after {:?}",
                PE_TRANSFORM_QUERY_TIMEOUT
            ))
        }
    }
}

async fn get_sanitized_local_mat4(refno: RefnoEnum, refresh_started: Instant) -> Option<DMat4> {
    record_transform_refresh_progress(
        "refresh_pe_transform_local_query_started",
        format!(
            "refno={refno} timeout_secs={}",
            PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
        ),
        refresh_started,
    );
    if is_transform_refresh_passthrough_marker(refno, refresh_started).await {
        record_transform_refresh_progress(
            "refresh_pe_transform_local_query_skipped_marker",
            format!("refno={refno}"),
            refresh_started,
        );
        return None;
    }

    match tokio::time::timeout(PE_TRANSFORM_QUERY_TIMEOUT, get_local_mat4(refno)).await {
        Ok(Ok(mat)) => sanitize_dmat4(mat),
        Ok(Err(err)) => {
            eprintln!("⚠️  获取本地变换失败: {} -> {}", refno, err);
            None
        }
        Err(_) => {
            eprintln!(
                "⚠️  获取本地变换超时: {} after {:?}",
                refno, PE_TRANSFORM_QUERY_TIMEOUT
            );
            record_transform_refresh_progress(
                "refresh_pe_transform_local_query_timeout",
                format!(
                    "refno={refno} timeout_secs={}",
                    PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
                ),
                refresh_started,
            );
            None
        }
    }
}

async fn is_transform_refresh_passthrough_marker(
    refno: RefnoEnum,
    refresh_started: Instant,
) -> bool {
    match tokio::time::timeout(PE_TRANSFORM_QUERY_TIMEOUT, get_type_name(refno)).await {
        Ok(Ok(noun)) => matches!(noun.as_str(), "JLDATU" | "PLDATU"),
        Ok(Err(err)) => {
            eprintln!("⚠️  获取节点类型失败: {} -> {}", refno, err);
            false
        }
        Err(_) => {
            eprintln!(
                "⚠️  获取节点类型超时: {} after {:?}",
                refno, PE_TRANSFORM_QUERY_TIMEOUT
            );
            record_transform_refresh_progress(
                "refresh_pe_transform_type_query_timeout",
                format!(
                    "refno={refno} timeout_secs={}",
                    PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
                ),
                refresh_started,
            );
            false
        }
    }
}

async fn compute_world_mat_from_owner_chain(
    refno: RefnoEnum,
    refresh_started: Instant,
) -> Result<DMat4> {
    let mut chain = vec![refno];
    let mut current = refno;

    loop {
        record_transform_refresh_progress(
            "refresh_pe_transform_owner_query_started",
            format!(
                "refno={current} timeout_secs={}",
                PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
            ),
            refresh_started,
        );
        let att = tokio::time::timeout(PE_TRANSFORM_QUERY_TIMEOUT, get_named_attmap(current))
            .await
            .map_err(|_| {
                record_transform_refresh_progress(
                    "refresh_pe_transform_owner_query_timeout",
                    format!(
                        "refno={current} timeout_secs={}",
                        PE_TRANSFORM_QUERY_TIMEOUT.as_secs()
                    ),
                    refresh_started,
                );
                anyhow::anyhow!(
                    "读取属性超时: {} after {:?}",
                    current,
                    PE_TRANSFORM_QUERY_TIMEOUT
                )
            })?
            .with_context(|| format!("读取属性失败: {}", current))?;
        let owner = att.get_owner();
        if owner.is_unset() {
            break;
        }
        chain.push(owner);
        current = owner;
    }

    chain.reverse();

    let mut world = DMat4::IDENTITY;
    for node in chain {
        let local = get_sanitized_local_mat4(node, refresh_started)
            .await
            .unwrap_or(DMat4::IDENTITY);
        world *= local;
    }

    Ok(world)
}

fn print_progress(processed: usize, total_nodes: usize, saved_batch: bool) {
    let percentage = if total_nodes > 0 {
        (processed as f64 / total_nodes as f64 * 100.0) as usize
    } else {
        0
    };
    let suffix = if saved_batch {
        " [已保存批次]"
    } else {
        ""
    };
    print!(
        "\r📊 进度: {}/{} ({:3}%){}...",
        processed, total_nodes, percentage, suffix
    );
    use std::io::Write;
    std::io::stdout().flush().ok();
}

async fn query_pe_transform_count_for_dbnum(dbnum: u32) -> Result<usize> {
    let sql = format!(
        "SELECT count() AS count FROM pe_transform WHERE record::id(id) INSIDE \
         (SELECT VALUE record::id(id) FROM pe WHERE dbnum = {}) GROUP ALL",
        dbnum
    );
    let rows: Vec<Value> = project_primary_db()
        .query_take(&sql, 0)
        .await
        .with_context(|| format!("执行 pe_transform 覆盖统计 SQL 失败: {}", sql))?;

    Ok(rows
        .iter()
        .find_map(extract_count_from_json_value)
        .unwrap_or(0) as usize)
}

fn record_transform_refresh_progress(stage: &str, detail: String, started: Instant) {
    crate::perf_metrics::record_generate_progress(
        stage,
        Some(&detail),
        started.elapsed().as_millis() as u64,
    );
}

pub(crate) fn extract_count_from_json_value(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| (n >= 0).then_some(n as u64))),
        Value::Object(map) => map
            .get("count")
            .or_else(|| map.get("cnt"))
            .and_then(extract_count_from_json_value),
        Value::Array(items) => items.iter().find_map(extract_count_from_json_value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::extract_count_from_json_value;
    use serde_json::json;

    #[test]
    fn extract_count_accepts_object_shape() {
        assert_eq!(
            extract_count_from_json_value(&json!({"count": 18649})),
            Some(18649)
        );
    }

    #[test]
    fn extract_count_accepts_scalar_shape() {
        assert_eq!(extract_count_from_json_value(&json!(18649)), Some(18649));
    }

    #[test]
    fn extract_count_accepts_nested_array_shape() {
        assert_eq!(
            extract_count_from_json_value(&json!([[{"count": 18649}]])),
            Some(18649)
        );
    }
}
