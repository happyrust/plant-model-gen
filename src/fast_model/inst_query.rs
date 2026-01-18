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

use anyhow::Context;
use aios_core::{GeomInstQuery, RefnoEnum, SUL_DB, SurrealQueryExt};

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
                    "" as generic,
                    refno.world_trans as world_trans,
                    refno.world_aabb as world_aabb,
                    NONE as pts,
                    [{{ "transform": refno.world_trans, "geo_hash": mesh_id, "is_tubi": false, "unit_flag": false }}] as insts,
                    true as has_neg,
                    NONE as date,
                    refno.spec_value as spec_value
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
                        "" as generic,
                        in.world_trans as world_trans,
                        in.world_aabb as world_aabb,
                        (SELECT value out.pts.*.d FROM out->geo_relate WHERE visible && out.meshed && (out.pts ?? NONE) != NONE LIMIT 1)[0] as pts,
                        (SELECT trans.d as transform, record::id(out) as geo_hash, false as is_tubi, out.unit_flag ?? false as unit_flag
                         FROM out->geo_relate
                         WHERE visible && (out.meshed || out.unit_flag || record::id(out) IN ['1','2','3'])
                           && (trans.d ?? NONE) != NONE
                           && geo_type IN ['Pos', 'Compound', 'DesiPos', 'CatePos']) as insts,
                        false as has_neg,
                        NONE as date,
                        spec_value
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
            let bool_check_expr = "type::record(\"inst_relate_bool\", record::id(in)).status = 'Success'";

            let sql = format!(
                r#"
                SELECT
                    in.id as refno,
                    in.owner ?? in as owner,
                    "" as generic,
                    in.world_trans as world_trans,
                    in.world_aabb as world_aabb,
                    (SELECT value out.pts.*.d FROM out->geo_relate WHERE visible && out.meshed && (out.pts ?? NONE) != NONE LIMIT 1)[0] as pts,
                    (SELECT trans.d as transform, record::id(out) as geo_hash, false as is_tubi, out.unit_flag ?? false as unit_flag
                     FROM out->geo_relate
                     WHERE visible && (out.meshed || out.unit_flag || record::id(out) IN ['1','2','3'])
                       && (trans.d ?? NONE) != NONE
                       && geo_type IN ['Pos', 'DesiPos', 'CatePos']) as insts,
                    ({bool_check} ?? false) as has_neg,
                    NONE as date,
                    spec_value
                FROM [{inst_relate_keys}]
                WHERE in.world_trans != NONE
                "#,
                inst_relate_keys = inst_relate_keys_str,
                bool_check = bool_check_expr
            );

            let mut chunk_result: Vec<GeomInstQuery> = SUL_DB
                .query_take(&sql, 0)
                .await
                .with_context(|| format!("query_insts_with_batch SQL: {}", sql))?;
            results.append(&mut chunk_result);
        }
    }

    Ok(results)
}

/// 查询几何实例信息（默认 batch）
pub async fn query_insts(refnos: &[RefnoEnum], enable_holes: bool) -> anyhow::Result<Vec<GeomInstQuery>> {
    query_insts_with_batch(refnos, enable_holes, None).await
}
