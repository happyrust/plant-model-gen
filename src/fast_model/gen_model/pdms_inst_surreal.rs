/// SurrealDB 极简版：单表存储所有关联数据
use anyhow::Result;
use aios_core::{RefnoEnum, model_primary_db};
use serde::{Deserialize, Serialize};
use surrealdb_types::SurrealValue;
use std::collections::HashMap;

/// refno 关联的所有数据（扁平化）
#[derive(Debug, Clone, Serialize, Deserialize, Default, SurrealValue)]
pub struct RefnoRelations {
    pub refno: RefnoEnum,
    pub dbnum: u32,
    pub inst_keys: Vec<String>,
    pub geo_hashes: Vec<u64>,
    pub tubi_segments: Vec<Vec<u8>>,
    pub bool_results: Vec<Vec<u8>>,
    pub world_matrices: Vec<Vec<u8>>,
}

/// 极简版清理：单条 DELETE 完成
pub async fn pre_cleanup_for_regen_surreal(seed_refnos: &[RefnoEnum]) -> Result<()> {
    if seed_refnos.is_empty() {
        return Ok(());
    }

    let t = std::time::Instant::now();

    // 1. 展开后代
    let all_refnos = aios_core::collect_descendant_filter_ids_with_self(
        seed_refnos, &[], None, true
    ).await?;

    if all_refnos.is_empty() {
        return Ok(());
    }

    // 2. 构建 refno ID 列表
    let refno_ids = all_refnos.iter()
        .map(|r| format!("refno_relations:{}", r.to_pe_key()))
        .collect::<Vec<_>>()
        .join(",");

    // 3. 单条 DELETE 清理所有关联数据
    let sql = format!("DELETE FROM refno_relations WHERE id IN [{}];", refno_ids);
    model_primary_db().query(&sql).await?;

    println!(
        "[cleanup_surreal] 删除 {} 个 refno，耗时 {} ms",
        all_refnos.len(),
        t.elapsed().as_millis()
    );

    Ok(())
}

/// 批量保存关联数据
pub async fn save_refno_relations_surreal(relations: &[RefnoRelations]) -> Result<()> {
    if relations.is_empty() {
        return Ok(());
    }

    // 构建批量 INSERT 语句
    let mut values = Vec::new();
    for rel in relations {
        let json = serde_json::to_string(rel)?;
        values.push(format!("(refno_relations:{}, {})", rel.refno.to_pe_key(), json));
    }

    let sql = format!(
        "INSERT INTO refno_relations {} ON DUPLICATE KEY UPDATE;",
        values.join(",")
    );

    model_primary_db().query(&sql).await?;
    Ok(())
}

/// 批量读取
pub async fn load_refno_relations_surreal(refnos: &[RefnoEnum]) -> Result<Vec<RefnoRelations>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let refno_ids = refnos.iter()
        .map(|r| format!("refno_relations:{}", r.to_pe_key()))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!("SELECT * FROM refno_relations WHERE id IN [{}];", refno_ids);
    let results: Vec<RefnoRelations> = model_primary_db()
        .query(&sql)
        .await?
        .take(0)?;

    Ok(results)
}
