use std::collections::VecDeque;

use aios_core::rs_surreal::pe_transform::{
    PeTransformEntry, ensure_pe_transform_schema, save_pe_transform_entries,
};
use aios_core::transform::get_local_mat4;
use aios_core::{
    RefnoEnum, SurrealQueryExt, Transform, get_children_refnos, get_named_attmap,
    project_primary_db,
};
use anyhow::{Context, Result};
use glam::DMat4;
use serde_json::Value;

const PE_TRANSFORM_BATCH_SIZE: usize = 500;

pub async fn refresh_pe_transform_for_dbnums_compat(dbnums: &[u32]) -> Result<usize> {
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

    for dbnum in dbnums {
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

            let local_mat = match get_local_mat4(root_refno).await {
                Ok(mat) => mat.filter(|m| !m.is_nan()),
                Err(err) => {
                    eprintln!("⚠️  刷新根节点本地变换失败: {} -> {}", root_refno, err);
                    None
                }
            };
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
                let children = match get_children_refnos(parent_refno).await {
                    Ok(children) => children,
                    Err(err) => {
                        eprintln!("⚠️  获取子节点失败: {} -> {}", parent_refno, err);
                        continue;
                    }
                };

                for child in children {
                    let local_mat = match get_local_mat4(child).await {
                        Ok(mat) => mat.filter(|m| !m.is_nan()),
                        Err(err) => {
                            eprintln!("⚠️  刷新本地变换失败: {} -> {}", child, err);
                            None
                        }
                    };
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

                    if entries.len() >= PE_TRANSFORM_BATCH_SIZE {
                        save_pe_transform_entries(&entries).await.with_context(|| {
                            format!("批量写入 pe_transform 失败: dbnum={}", dbnum)
                        })?;
                        entries.clear();
                        print_progress(dbnum_processed, total_nodes, true);
                        dbnum_last_print = dbnum_processed;
                    }
                }
            }
        }

        println!();
    }

    if !entries.is_empty() {
        save_pe_transform_entries(&entries)
            .await
            .context("写入最后一批 pe_transform 失败")?;
    }

    println!("\r✅ 完成！共处理 {} 个节点                    ", total);
    Ok(total)
}

pub async fn refresh_pe_transform_for_root_refnos_compat(
    root_refnos: &[RefnoEnum],
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

    for root_refno in roots {
        let root_local = match get_local_mat4(root_refno).await {
            Ok(mat) => mat.filter(|m| !m.is_nan()),
            Err(err) => {
                eprintln!("⚠️  刷新根节点本地变换失败: {} -> {}", root_refno, err);
                None
            }
        };
        let root_world = compute_world_mat_from_owner_chain(root_refno)
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
            let children = match get_children_refnos(parent_refno).await {
                Ok(children) => children,
                Err(err) => {
                    eprintln!("⚠️  获取子节点失败: {} -> {}", parent_refno, err);
                    continue;
                }
            };

            for child in children {
                let local_mat = match get_local_mat4(child).await {
                    Ok(mat) => mat.filter(|m| !m.is_nan()),
                    Err(err) => {
                        eprintln!("⚠️  刷新本地变换失败: {} -> {}", child, err);
                        None
                    }
                };
                let world_mat = match local_mat {
                    Some(local) => parent_world * local,
                    None => parent_world,
                };
                push_entry(&mut entries, &mut total, child, local_mat, Some(world_mat));
                queue.push_back((child, world_mat));

                if entries.len() >= PE_TRANSFORM_BATCH_SIZE {
                    save_pe_transform_entries(&entries).await.with_context(|| {
                        format!("批量写入 pe_transform 失败: root_refno={}", root_refno)
                    })?;
                    entries.clear();
                }
            }
        }
    }

    if !entries.is_empty() {
        save_pe_transform_entries(&entries)
            .await
            .context("写入最后一批 pe_transform 失败")?;
    }

    println!(
        "\r✅ 子树刷新完成！共处理 {} 个节点                    ",
        total
    );
    Ok(total)
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

fn dmat4_to_transform_option(matrix: Option<DMat4>) -> Option<Transform> {
    matrix
        .filter(|mat| !mat.is_nan())
        .map(|mat| Transform::from_matrix(mat.as_mat4()))
        .filter(|transform| transform.is_finite())
}

async fn compute_world_mat_from_owner_chain(refno: RefnoEnum) -> Result<DMat4> {
    let mut chain = vec![refno];
    let mut current = refno;

    loop {
        let att = get_named_attmap(current)
            .await
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
        let local = get_local_mat4(node)
            .await
            .with_context(|| format!("计算局部变换失败: {}", node))?
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
