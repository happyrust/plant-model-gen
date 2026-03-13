//! DB 批量补齐模块：从 SurrealDB 补齐内存中缺失的 CataNeg 布尔任务。
//!
//! 当 `enable_db_backfill=true` 且 `use_surrealdb=true` 时启用。
//! 查询 DB 中 `has_cata_neg=true` 但不在现有内存任务中的 refno，
//! 转换为 `BooleanTask` 后合并到内存任务列表。

use std::collections::HashSet;

use aios_core::{RefnoEnum, SurrealQueryExt, model_primary_db};

use super::boolean_task::{BooleanTask, BooleanTaskType, CataGeoData, CataNegBoolTask};

/// 查询 DB 中需要 cata_neg 布尔但不在现有任务中的 refno 候选集。
///
/// 逻辑：查询 `inst_relate` 中 `has_cata_neg=true` 的记录，
/// 排除已在 `existing_task_refnos` 中的 refno。
pub async fn query_cata_backfill_candidates(
    existing_task_refnos: &HashSet<RefnoEnum>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    // 查询 DB 中所有标记了 has_cata_neg 的 refno
    let sql = "SELECT VALUE in.id FROM inst_relate WHERE has_cata_neg = true";
    let all_cata_refnos: Vec<RefnoEnum> = model_primary_db()
        .query_take(sql, 0)
        .await
        .unwrap_or_default();

    let candidates: Vec<RefnoEnum> = all_cata_refnos
        .into_iter()
        .filter(|r| !existing_task_refnos.contains(r))
        .collect();

    Ok(candidates)
}

/// 从 DB 加载指定 refno 的 cata_neg 数据，转换为 BooleanTask。
///
/// 查询 inst_relate -> geo_relate 链，构建 boolean_groups 和 geo_data_map。
pub async fn fetch_cata_bool_tasks_from_db(
    refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<BooleanTask>> {
    use aios_core::Transform;
    use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
    use std::collections::HashMap;

    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let mut tasks = Vec::new();

    // 分块查询，每次 50 个 refno
    for chunk in refnos.chunks(50) {
        let pe_keys: Vec<String> = chunk.iter().map(|r| r.to_pe_key()).collect();
        let pe_keys_str = pe_keys.join(",");

        // 查询 inst_relate 获取 inst_info_id 和 noun
        let inst_sql = format!(
            "SELECT in.id AS refno_id, id AS inst_relate_id, out AS inst_info_id, in.noun AS noun \
             FROM [{}]->inst_relate WHERE has_cata_neg = true",
            pe_keys_str
        );

        let inst_rows: Vec<serde_json::Value> = model_primary_db()
            .query_take(&inst_sql, 0)
            .await
            .unwrap_or_default();

        for row in &inst_rows {
            let refno_str = row
                .get("refno_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if refno_str.is_empty() {
                continue;
            }
            let refno = RefnoEnum::from(refno_str);

            let noun = row
                .get("noun")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if noun.as_deref() == Some("BRAN") {
                continue;
            }

            let inst_info_id = row
                .get("inst_info_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            if inst_info_id.is_empty() {
                continue;
            }

            // 查询该 inst_info 下的 geo_relate，获取正/负几何体关系
            let geo_sql = format!(
                "SELECT out AS geo_id, geom_refno, geo_type, trans \
                 FROM {}-\u{2192}geo_relate",
                inst_info_id
            );

            let geo_rows: Vec<serde_json::Value> = model_primary_db()
                .query_take(&geo_sql, 0)
                .await
                .unwrap_or_default();

            let mut boolean_groups: Vec<Vec<RefnoEnum>> = Vec::new();
            let mut geo_data_map: HashMap<RefnoEnum, CataGeoData> = HashMap::new();

            // 简化处理：收集所有 Pos 和 Neg/CataCrossNeg 几何体
            let mut pos_geos: Vec<RefnoEnum> = Vec::new();
            let mut neg_geos: Vec<RefnoEnum> = Vec::new();

            for geo_row in &geo_rows {
                let geo_type = geo_row
                    .get("geo_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let geom_refno_str = geo_row
                    .get("geom_refno")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if geom_refno_str.is_empty() {
                    continue;
                }
                let geom_refno = RefnoEnum::from(geom_refno_str);

                // 为 geo_data_map 插入条目（使用默认参数，实际参数后续按需补齐）
                geo_data_map
                    .entry(geom_refno)
                    .or_insert_with(|| CataGeoData {
                        geo_hash: 0, // 将从 inst_geo 表补齐
                        param: PdmsGeoParam::default(),
                        transform: Transform::default(),
                    });

                match geo_type {
                    "Pos" | "CatePos" | "Compound" => pos_geos.push(geom_refno),
                    "Neg" | "CataCrossNeg" => neg_geos.push(geom_refno),
                    _ => {}
                }
            }

            // 构建 boolean_groups：每个 Pos 几何体作为一个分组的首元素，后接所有 Neg
            if !pos_geos.is_empty() && !neg_geos.is_empty() {
                for pos in &pos_geos {
                    let mut group = vec![*pos];
                    group.extend_from_slice(&neg_geos);
                    boolean_groups.push(group);
                }
            }

            if boolean_groups.is_empty() {
                continue;
            }

            tasks.push(BooleanTask {
                refno,
                noun,
                task_type: BooleanTaskType::CataNeg(CataNegBoolTask {
                    inst_info_id,
                    boolean_groups,
                    geo_data_map,
                }),
            });
        }
    }

    Ok(tasks)
}

/// 执行 DB backfill：查询候选 refno，转换为 BooleanTask，合并到内存任务列表。
///
/// 内存任务优先：已存在于 `existing_tasks` 中的 refno 不会被 DB 数据覆盖。
pub async fn backfill_cata_tasks_from_db(
    existing_tasks: &mut Vec<BooleanTask>,
    use_surrealdb: bool,
) -> anyhow::Result<usize> {
    if !use_surrealdb {
        println!("[db_backfill] use_surrealdb=false，跳过 DB 补齐（无 DB 可查）");
        return Ok(0);
    }

    let existing_refnos: HashSet<RefnoEnum> = existing_tasks.iter().map(|t| t.refno).collect();

    let candidates = query_cata_backfill_candidates(&existing_refnos).await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    println!(
        "[db_backfill] 发现 {} 个 DB 中有 cata_neg 但内存中缺失的 refno，开始补齐",
        candidates.len()
    );

    let backfill_tasks = fetch_cata_bool_tasks_from_db(&candidates).await?;
    let count = backfill_tasks.len();

    existing_tasks.extend(backfill_tasks);

    println!("[db_backfill] 成功补齐 {} 个 cata 布尔任务", count);
    Ok(count)
}
