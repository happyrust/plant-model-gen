//! inst_relate/geo_relate 查询（v2）
//!
//! 迁移目标：
//! - inst_relate 不再保存 world_trans
//! - 世界变换统一从 pe_transform(world_trans) 获取
//! - 实例级 AABB 统一从 inst_relate_aabb(out) 获取
//!
//! 该模块提供与 aios_core::query_insts(_with_batch) 等价的查询能力，
//! 但不依赖 inst_relate.world_trans / inst_relate.aabb 字段，避免旧 schema 绑定。
//!
//! ## v2 简化逻辑
//!
//! 分两路并行查询，然后合并：
//! 1. **布尔结果路径**：直接从 `inst_relate_bool` 获取 status='Success' 的记录
//! 2. **原始几何路径**：从 `inst_relate` 查询，跳过已有布尔结果的 refnos
//!
//! 字段来源（利用 pe 表计算字段）：
//! - `owner` → `pe.owner`
//! - `world_trans` → `pe.world_trans`（计算字段，自动从 pe_transform 获取）
//! - `world_aabb` → `pe.world_aabb`（计算字段，自动从 inst_relate_aabb 获取）
//!
//! ## geo_type 语义约定
//!
//! | geo_type | 含义 | 是否导出 |
//! |----------|------|----------|
//! | Pos | 原始几何（未布尔运算） | ✅ 导出 |
//! | DesiPos | 设计位置 | ✅ 导出 |
//! | CatePos | 布尔运算后的结果 | ✅ 导出 |
//! | Compound | 组合几何体（包含负实体引用） | ❌ 不导出 |
//! | CateNeg | 负实体 | ❌ 不导出 |
//! | CataCrossNeg | 交叉负实体 | ❌ 不导出 |
//!
//! 查询条件：`geo_type IN ['Pos', 'DesiPos', 'CatePos']`

use anyhow::Context;
use aios_core::{GeomInstQuery, RefnoEnum, SUL_DB, SurrealQueryExt};
use surrealdb::types::SurrealValue;
use serde::{Deserialize, Serialize};
use aios_core::prim_geo::basic::TUBI_GEO_HASH;
use aios_core::shape::pdms_shape::RsVec3;
use aios_core::rs_surreal::geometry_query::PlantTransform;
use aios_core::types::PlantAabb;
use aios_core::ModelHashInst;

#[derive(Serialize, Deserialize, Debug, SurrealValue)]
struct TubiQueryResult {
    pub refno: RefnoEnum,
    pub owner: RefnoEnum,
    #[serde(default)]
    pub world_trans: Option<PlantTransform>,
    #[serde(default)]
    pub world_aabb: Option<PlantAabb>,
    #[serde(default)]
    pub start_pt: Option<RsVec3>,
    #[serde(default)]
    pub end_pt: Option<RsVec3>,
    #[serde(default)]
    pub geo_hash: Option<String>,
    #[serde(default)]
    pub index: Option<i64>,
}

fn normalize_inst_geo_hash(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("inst_geo:`") {
        return rest.trim_end_matches('`').to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("inst_geo:") {
        return rest.trim_matches('`').to_string();
    }
    trimmed.to_string()
}

/// 查询几何实例信息（带 batch）
///
/// 分两路并行查询：
/// - enable_holes=true: 布尔结果从 inst_relate_bool 获取，原始几何从 inst_relate 获取
/// - enable_holes=false: 始终从 inst_relate 获取原始几何
pub async fn query_insts_with_batch(
    refnos: &[RefnoEnum],
    enable_holes: bool,
    batch_size: Option<usize>,
) -> anyhow::Result<Vec<GeomInstQuery>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let batch = batch_size.unwrap_or(50).max(1);
    let mut results: Vec<GeomInstQuery> = Vec::new();

    for chunk in refnos.chunks(batch) {
        if enable_holes {
            // ========== 路径 A：布尔结果查询 ==========
            // 直接从 inst_relate_bool 获取有成功布尔结果的记录
            // 使用 FROM [ids] 语法直接指定要查询的记录
            let bool_keys: Vec<String> = chunk
                .iter()
                .map(|r| r.to_table_key("inst_relate_bool"))
                .collect();
            let bool_keys_str = bool_keys.join(",");

            let bool_sql = format!(
                r#"
                SELECT
                    refno,
                    refno.owner ?? refno as owner,
                    refno.world_trans as world_trans,
                    refno.world_aabb as world_aabb,
                    [{{ "geo_transform": refno.world_trans, "geo_hash": mesh_id, "is_tubi": false, "unit_flag": false }}] as insts,
                    true as has_neg
                FROM [{bool_keys}]
                WHERE status = 'Success'
                  AND refno.world_trans != NONE
                "#,
                bool_keys = bool_keys_str
            );

            let mut bool_results: Vec<GeomInstQuery> = SUL_DB
                .query_take(&bool_sql, 0)
                .await
                .with_context(|| format!("query_insts_with_batch bool SQL: {}", bool_sql))?;

            // 收集已有布尔结果的 refnos
            let bool_refnos: std::collections::HashSet<_> =
                bool_results.iter().map(|r| r.refno.clone()).collect();

            results.append(&mut bool_results);

            // ========== 路径 B：原始几何查询（排除已有布尔结果的） ==========
            let non_bool_keys: Vec<String> = chunk
                .iter()
                .filter(|r| !bool_refnos.contains(*r))
                .map(|r| r.to_inst_relate_key())
                .collect();

            if !non_bool_keys.is_empty() {
                let non_bool_keys_str = non_bool_keys.join(",");
                // 利用 pe 表的计算字段：in.world_trans / in.world_aabb
                let geo_sql = format!(
                    r#"
                    SELECT
                        in.id as refno,
                        in.owner ?? in as owner,
                        in.world_trans as world_trans,
                        in.world_aabb as world_aabb,
                        (SELECT trans.d as geo_transform, record::id(out) as geo_hash, false as is_tubi, out.unit_flag ?? false as unit_flag
                         FROM $parent.out->geo_relate
                         WHERE visible && out.meshed
                           && (trans.d ?? NONE) != NONE
                           && geo_type IN ['Pos', 'DesiPos', 'CatePos', 'Compound']) as insts,
                        false as has_neg
                    FROM [{non_bool_keys}]
                    WHERE in.world_trans != NONE
                    "#,
                    non_bool_keys = non_bool_keys_str
                );

                let mut geo_results: Vec<GeomInstQuery> = SUL_DB
                    .query_take(&geo_sql, 0)
                    .await
                    .with_context(|| format!("query_insts_with_batch geo SQL: {}", geo_sql))?;
                results.append(&mut geo_results);
            }
        } else {
            // ========== enable_holes=false：始终返回原始几何 ==========
            let inst_relate_keys: Vec<String> =
                chunk.iter().map(|r| r.to_inst_relate_key()).collect();
            let inst_relate_keys_str = inst_relate_keys.join(",");

            // 利用 pe 表的计算字段简化查询
            // 仍然需要检查 inst_relate_bool 来设置 has_neg 标志
            let sql = format!(
                r#"
                SELECT
                    in.id as refno,
                    in.owner ?? in as owner,
                    in.world_trans as world_trans,
                    in.world_aabb as world_aabb,
                    (SELECT trans.d as geo_transform, record::id(out) as geo_hash, false as is_tubi, out.unit_flag ?? false as unit_flag
                     FROM $parent.out->geo_relate
                     WHERE visible && out.meshed
                       && (trans.d ?? NONE) != NONE
                       && geo_type IN ['Pos', 'Compound']) as insts,
                    false as has_neg
                FROM [{inst_relate_keys}]
                WHERE in.world_trans != NONE
                "#,
                inst_relate_keys = inst_relate_keys_str
            );

            let mut chunk_result: Vec<GeomInstQuery> = SUL_DB
                .query_take(&sql, 0)
                .await
                .with_context(|| format!("query_insts_with_batch SQL: {}", sql))?;
            results.append(&mut chunk_result);
        }

        // ========== TUBI 查询 ==========
        // tubi_relate 使用复合 ID（pe, index）；这里为每个 refno 发起 range 查询并合并为 is_tubi 实例。
        let mut tubi_sql_batch = String::new();
        for r in chunk {
            let pe_key = r.to_pe_key();
            tubi_sql_batch.push_str(&format!(
                r#"
                SELECT
                    id[0] as refno,
                    in as owner,
                    world_trans.d as world_trans,
                    aabb.d as world_aabb,
                    start_pt.d as start_pt,
                    end_pt.d as end_pt,
                    record::id(geo) as geo_hash,
                    id[1] as index
                FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                "#
            ));
        }

        if !tubi_sql_batch.is_empty() {
            let mut resp = SUL_DB
                .query_response(&tubi_sql_batch)
                .await
                .with_context(|| format!("query_insts_with_batch tubi SQL: {}", tubi_sql_batch))?;

            for (stmt_idx, _) in chunk.iter().enumerate() {
                let raw_tubis: Vec<TubiQueryResult> = resp.take(stmt_idx)?;
                for raw in raw_tubis {
                    let geo_hash = raw
                        .geo_hash
                        .as_deref()
                        .map(normalize_inst_geo_hash)
                        .unwrap_or_else(|| TUBI_GEO_HASH.to_string());
                    let wt = raw.world_trans.unwrap_or_default();
                    results.push(GeomInstQuery {
                        // 约定：TUBI 的 refno 使用 leave（tubi_relate 的 in），owner 使用 BRAN/HANG（tubi_relate 的 id[0]）。
                        // 这样与 model cache（insert_tubi(leave_refno, EleGeosInfo{ owner_refno=bran })）保持一致，
                        // 也能让导出侧（collect_export_data）生成稳定、可追溯的 tubi 名称/分组。
                        refno: raw.owner,
                        owner: raw.refno,
                        world_aabb: raw.world_aabb,
                        // TUBI world_trans 直接放到 inst.geo_transform（视为世界矩阵），避免与 refno 的 world_trans 混乘。
                        world_trans: PlantTransform::default(),
                        insts: vec![ModelHashInst {
                            geo_hash,
                            geo_transform: wt,
                            is_tubi: true,
                            unit_flag: false,
                        }],
                        has_neg: false,
                    });
                }
            }
        }
    }

    Ok(results)
}

/// 查询几何实例信息（默认 batch）
pub async fn query_insts(refnos: &[RefnoEnum], enable_holes: bool) -> anyhow::Result<Vec<GeomInstQuery>> {
    query_insts_with_batch(refnos, enable_holes, None).await
}

