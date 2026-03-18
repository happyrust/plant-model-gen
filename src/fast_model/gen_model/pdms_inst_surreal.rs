use aios_core::{RefnoEnum, model_primary_db};
/// SurrealDB 极简版：单表存储所有关联数据
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use std::collections::HashMap;
use surrealdb_types::SurrealValue;

/// refno 关联的所有数据（扁平化）
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Default, SurrealValue)]
pub struct RefnoRelations {
    pub refno: RefnoEnum,
    pub dbnum: u32,
    pub inst_keys: Vec<String>,
    #[serde_as(as = "Vec<DisplayFromStr>")]
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
    let all_refnos =
        aios_core::collect_descendant_filter_ids_with_self(seed_refnos, &[], None, true).await?;

    if all_refnos.is_empty() {
        return Ok(());
    }

    // 2. Surreal 对嵌套 record id 的 `WHERE id IN [...]` 解析不稳定，
    //    这里改成逐条点删，避免在 regen 前清理阶段直接报 SQL parse error。
    // 分批执行，避免单次发送十万条 DELETE 导致 SurrealDB 解析卡死。
    const CHUNK_SIZE: usize = 500;
    for chunk in all_refnos.chunks(CHUNK_SIZE) {
        let sql = chunk
            .iter()
            .map(|r| format!("DELETE refno_relations:⟨{}⟩;", r.to_pe_key()))
            .collect::<Vec<_>>()
            .join("\n");

        if let Err(e) = model_primary_db().query(&sql).await {
            eprintln!("[cleanup_surreal] chunk delete error: {}", e);
        }
    }

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

    // Surreal 对带嵌套 record id 的 INSERT tuple 解析同样不稳定，
    // 改为逐条 UPSERT record-id CONTENT。
    let mut sqls = Vec::with_capacity(relations.len());
    for rel in relations {
        let json = serde_json::to_string(rel)?;
        sqls.push(format!(
            "UPSERT refno_relations:⟨{}⟩ CONTENT {};",
            rel.refno.to_pe_key(),
            json
        ));
    }

    model_primary_db().query(&sqls.join("\n")).await?;
    Ok(())
}

/// 批量读取
pub async fn load_refno_relations_surreal(refnos: &[RefnoEnum]) -> Result<Vec<RefnoRelations>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let refno_ids = refnos
        .iter()
        .map(|r| format!("refno_relations:{}", r.to_pe_key()))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!("SELECT * FROM refno_relations WHERE id IN [{}];", refno_ids);
    let results: Vec<RefnoRelations> = model_primary_db().query(&sql).await?.take(0)?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::RefnoRelations;
    use aios_core::RefnoEnum;
    use std::str::FromStr;

    #[test]
    fn geo_hashes_are_serialized_as_strings_for_surreal_content() {
        let rel = RefnoRelations {
            refno: RefnoEnum::from_str("7997/1").unwrap(),
            dbnum: 7997,
            inst_keys: vec!["inst-a".to_string()],
            geo_hashes: vec![12_452_550_876_698_633_064],
            tubi_segments: vec![],
            bool_results: vec![],
            world_matrices: vec![],
        };

        let value = serde_json::to_value(&rel).unwrap();
        let geo_hashes = value
            .get("geo_hashes")
            .and_then(|v| v.as_array())
            .expect("geo_hashes should be an array");

        assert_eq!(geo_hashes.len(), 1);
        assert_eq!(
            geo_hashes[0].as_str(),
            Some("12452550876698633064"),
            "u64 geo_hash 应以字符串写入，避免 Surreal JSON number 解析成 i64 失败"
        );
    }
}
